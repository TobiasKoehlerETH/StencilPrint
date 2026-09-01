use crate::gerber::{Aperture, MacroShape, ParsedLayer, Point, Primitive};
use geo::{
    unary_union, Area, BooleanOps, Buffer, Contains, Coord, LineString, MultiPolygon,
    Point as GeoPoint, Polygon,
};
use std::f64::consts::TAU;

const CURVE_SEGMENTS: usize = 32;
const OUTLINE_TOLERANCE: f64 = 0.01;
const GEOMETRY_EPSILON: f64 = 1e-7;
const RING_SIMPLIFY_TOLERANCE: f64 = 0.002;

#[derive(Clone)]
pub(crate) struct StencilGeometry {
    pub plate: Vec<Point>,
    pub openings: Vec<Vec<Point>>,
    pub selection_openings: Vec<Vec<Point>>,
    pub opening_sources: Vec<Vec<usize>>,
    pub inner_wall: Vec<Point>,
    pub outer_wall: Vec<Point>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy)]
pub(crate) struct GeometryOptions {
    pub clearance: f64,
    pub wall_thickness: f64,
    pub shrink: f64,
    pub nozzle_diameter: f64,
    pub enable_slotify: bool,
    pub drop_unprintable_grids: bool,
    pub mirror_back: bool,
}

impl StencilGeometry {
    pub fn from_layers(
        paste: &ParsedLayer,
        edge: &ParsedLayer,
        options: GeometryOptions,
    ) -> Result<Self, String> {
        Self::from_layers_excluding(paste, edge, options, &[])
    }

    pub fn from_layers_excluding(
        paste: &ParsedLayer,
        edge: &ParsedLayer,
        options: GeometryOptions,
        excluded_openings: &[usize],
    ) -> Result<Self, String> {
        let GeometryOptions {
            clearance,
            wall_thickness,
            shrink,
            nozzle_diameter,
            enable_slotify,
            drop_unprintable_grids,
            mirror_back,
        } = options;
        let board = outline_from_edge(edge, &paste.points())?;
        let inner_wall = offset_polygon(&board, clearance)?;
        let outer_wall = offset_polygon(&board, clearance + wall_thickness)?;
        let mut warnings = Vec::new();
        let mut paste_polygons = primitive_polygons(paste);
        if mirror_back {
            mirror_polygons(&mut paste_polygons, &board);
            warnings.push("Back paste mirrored around the board centre for registration.".into());
        }
        let selection_openings = paste_polygons.clone();
        paste_polygons = paste_polygons
            .into_iter()
            .enumerate()
            .filter_map(|(index, polygon)| (!excluded_openings.contains(&index)).then_some(polygon))
            .collect();
        let raw_opening_count = paste_polygons.len();
        let mut opening_polygons = union_polygons(&paste_polygons);
        if opening_polygons.is_empty() && excluded_openings.is_empty() {
            return Err("The paste layer contains no printable pad openings.".into());
        }

        if shrink.abs() > GEOMETRY_EPSILON {
            opening_polygons = offset_polygons(&opening_polygons, -shrink);
            if opening_polygons.is_empty() {
                return Err(
                    "Shrink removed every paste opening. Use a smaller shrink value.".into(),
                );
            }
            warnings.push(format!("Paste openings adjusted by {shrink:.2} mm shrink."));
        }

        let (compensated, compensation_applied) = compensate_for_nozzle(
            &opening_polygons,
            nozzle_diameter,
            enable_slotify,
            drop_unprintable_grids,
        );
        opening_polygons = compensated;
        if compensation_applied {
            warnings.push(format!(
                "Nozzle compensation applied for a {nozzle_diameter:.2} mm nozzle."
            ));
        }

        let (opening_polygons, clipped) = clip_openings(&opening_polygons, &inner_wall)?;
        if clipped {
            warnings.push("Paste geometry outside the board clearance was clipped.".into());
        }
        let openings = opening_polygons;
        if openings.is_empty() && excluded_openings.is_empty() {
            return Err("The paste layer contains no printable pad openings.".into());
        }
        validate_openings(&inner_wall, &openings)?;
        if raw_opening_count > openings.len() {
            warnings.push(format!(
                "{} overlapping or duplicate paste shapes were fused into {} openings.",
                raw_opening_count,
                openings.len()
            ));
        }
        let opening_sources = map_opening_sources(&selection_openings, &openings);
        Ok(Self {
            plate: outer_wall.clone(),
            openings,
            selection_openings,
            opening_sources,
            outer_wall,
            inner_wall,
            warnings,
        })
    }
}

fn outline_from_edge(edge: &ParsedLayer, _fallback: &[Point]) -> Result<Vec<Point>, String> {
    let segments = edge
        .primitives
        .iter()
        .filter_map(|primitive| match primitive {
            Primitive::Stroke(start, end, _) => Some((*start, *end)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut candidates = edge
        .primitives
        .iter()
        .filter_map(|primitive| match primitive {
            Primitive::Region(points) if points.len() >= 3 => Some(points.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    candidates.extend(closed_outlines(&segments));

    candidates
        .into_iter()
        .filter(|points| signed_area(points).abs() > 1e-9)
        .max_by(|left, right| {
            signed_area(left)
                .abs()
                .partial_cmp(&signed_area(right).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(ensure_ccw)
        .ok_or_else(|| {
            format!(
                "Could not trace a closed board outline in {}. Load an Edge.Cuts/Profile Gerber with a closed contour.",
                edge.filename
            )
        })
}

fn closed_outlines(segments: &[(Point, Point)]) -> Vec<Vec<Point>> {
    let mut used = vec![false; segments.len()];
    let mut outlines = Vec::new();

    for start_index in 0..segments.len() {
        if used[start_index] {
            continue;
        }
        let (first_start, first_end) = segments[start_index];
        used[start_index] = true;
        let mut outline = vec![first_start, first_end];
        let mut current = first_end;

        loop {
            if points_are_close(current, first_start) {
                outline.pop();
                if outline.len() >= 3 {
                    outlines.push(outline);
                }
                break;
            }

            let Some(index) = segments
                .iter()
                .enumerate()
                .find_map(|(index, (start, end))| {
                    (!used[index]
                        && (points_are_close(*start, current) || points_are_close(*end, current)))
                    .then_some(index)
                })
            else {
                break;
            };
            used[index] = true;
            let (start, end) = segments[index];
            current = if points_are_close(start, current) {
                end
            } else {
                start
            };
            outline.push(current);
        }
    }

    outlines
}

fn ensure_ccw(mut polygon: Vec<Point>) -> Vec<Point> {
    if signed_area(&polygon) < 0.0 {
        polygon.reverse();
    }
    polygon
}

pub(crate) fn signed_area(points: &[Point]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let next = points[(index + 1) % points.len()];
            point.x * next.y - next.x * point.y
        })
        .sum::<f64>()
        / 2.0
}

pub(crate) fn distance_squared(a: Point, b: Point) -> f64 {
    (a.x - b.x).powi(2) + (a.y - b.y).powi(2)
}

fn points_are_close(a: Point, b: Point) -> bool {
    (a.x - b.x).hypot(a.y - b.y) < OUTLINE_TOLERANCE
}

fn offset_polygon(polygon: &[Point], distance: f64) -> Result<Vec<Point>, String> {
    if polygon.len() < 3 {
        return Err("The board outline must contain at least three corners.".into());
    }
    if distance.abs() < f64::EPSILON {
        return Ok(ensure_ccw(polygon.to_vec()));
    }

    let shape = polygon_from_points(polygon)
        .ok_or_else(|| "The board outline is degenerate and cannot be offset.".to_string())?;
    let buffered = shape.buffer(distance);
    let result = buffered
        .0
        .into_iter()
        .max_by(|left, right| {
            left.unsigned_area()
                .partial_cmp(&right.unsigned_area())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|polygon| points_from_linestring(polygon.exterior()))
        .filter(|points| signed_area(points).abs() > 1e-9)
        .ok_or_else(|| {
            "The board outline collapsed while applying clearance or wall thickness.".to_string()
        })?;
    Ok(ensure_ccw(result))
}

fn polygon_from_points(points: &[Point]) -> Option<Polygon<f64>> {
    if points.len() < 3 {
        return None;
    }
    let mut coordinates = points
        .iter()
        .map(|point| Coord {
            x: point.x,
            y: point.y,
        })
        .collect::<Vec<_>>();
    if coordinates.first() != coordinates.last() {
        coordinates.push(*coordinates.first()?);
    }
    Some(Polygon::new(LineString::from(coordinates), Vec::new()))
}

fn points_from_linestring(linestring: &LineString<f64>) -> Vec<Point> {
    let mut points = linestring
        .0
        .iter()
        .map(|coordinate| Point {
            x: coordinate.x,
            y: coordinate.y,
        })
        .collect::<Vec<_>>();
    if points.len() > 1 && points_are_close(points[0], *points.last().unwrap()) {
        points.pop();
    }
    simplify_ring(points)
}

fn simplify_ring(mut points: Vec<Point>) -> Vec<Point> {
    if points.len() < 4 {
        return points;
    }
    loop {
        let mut removed = false;
        let count = points.len();
        for index in 0..count {
            let previous = points[(index + count - 1) % count];
            let current = points[index];
            let next = points[(index + 1) % count];
            if point_segment_distance(current, previous, next) <= RING_SIMPLIFY_TOLERANCE {
                points.remove(index);
                removed = true;
                break;
            }
        }
        if !removed || points.len() < 4 {
            return points;
        }
    }
}

fn point_segment_distance(point: Point, start: Point, end: Point) -> f64 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= GEOMETRY_EPSILON {
        return (point.x - start.x).hypot(point.y - start.y);
    }
    let projection =
        (((point.x - start.x) * dx + (point.y - start.y) * dy) / length_squared).clamp(0.0, 1.0);
    let closest = Point {
        x: start.x + projection * dx,
        y: start.y + projection * dy,
    };
    (point.x - closest.x).hypot(point.y - closest.y)
}

fn mirror_polygons(polygons: &mut [Vec<Point>], board: &[Point]) {
    let min_x = board
        .iter()
        .map(|point| point.x)
        .fold(f64::INFINITY, f64::min);
    let max_x = board
        .iter()
        .map(|point| point.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let centre_x = (min_x + max_x) / 2.0;
    for polygon in polygons {
        for point in polygon {
            point.x = 2.0 * centre_x - point.x;
        }
    }
}

fn primitive_polygons(layer: &ParsedLayer) -> Vec<Vec<Point>> {
    let mut polygons = Vec::new();
    for primitive in &layer.primitives {
        let primitive_shapes = match primitive {
            Primitive::Flash(point, aperture) => aperture_polygons(*point, aperture),
            Primitive::Region(points) if points.len() >= 3 => vec![points.clone()],
            Primitive::Stroke(start, end, aperture) => stroke_polygons(*start, *end, aperture),
            _ => Vec::new(),
        };
        polygons.extend(union_polygons(&primitive_shapes));
    }
    polygons
}

fn aperture_polygons(center: Point, aperture: &Aperture) -> Vec<Vec<Point>> {
    match aperture {
        Aperture::Circle(diameter) => vec![regular_polygon(center, *diameter, CURVE_SEGMENTS, 0.0)],
        Aperture::Rectangle(width, height) => vec![rectangle(center, *width, *height)],
        Aperture::Obround(width, height) => vec![obround(center, *width, *height)],
        Aperture::Polygon(diameter, sides, rotation) => {
            vec![regular_polygon(
                center,
                *diameter,
                *sides,
                rotation.to_radians(),
            )]
        }
        Aperture::Composite(shapes) => shapes
            .iter()
            .filter_map(|shape| match shape {
                MacroShape::Circle {
                    diameter,
                    center: offset,
                } => Some(regular_polygon(
                    Point {
                        x: center.x + offset.x,
                        y: center.y + offset.y,
                    },
                    *diameter,
                    CURVE_SEGMENTS,
                    0.0,
                )),
                MacroShape::Polygon(points) => Some(
                    points
                        .iter()
                        .map(|point| Point {
                            x: center.x + point.x,
                            y: center.y + point.y,
                        })
                        .collect(),
                ),
                MacroShape::Stroke { start, end, width } => stroke_polygon(*start, *end, *width)
                    .into_iter()
                    .next()
                    .map(|polygon| {
                        polygon
                            .into_iter()
                            .map(|point| Point {
                                x: center.x + point.x,
                                y: center.y + point.y,
                            })
                            .collect()
                    }),
            })
            .collect(),
    }
}

fn stroke_polygons(start: Point, end: Point, aperture: &Aperture) -> Vec<Vec<Point>> {
    if points_are_close(start, end) {
        return aperture_polygons(start, aperture);
    }
    let width = match aperture {
        Aperture::Circle(diameter) | Aperture::Polygon(diameter, _, _) => *diameter,
        Aperture::Rectangle(width, _) | Aperture::Obround(width, _) => *width,
        Aperture::Composite(shapes) => shapes
            .iter()
            .filter_map(|shape| match shape {
                MacroShape::Circle { diameter, .. } => Some(*diameter),
                MacroShape::Polygon(_) => None,
                MacroShape::Stroke { width, .. } => Some(*width),
            })
            .fold(0.0, f64::max),
    };
    stroke_polygon(start, end, width)
}

fn stroke_polygon(start: Point, end: Point, width: f64) -> Vec<Vec<Point>> {
    if width <= f64::EPSILON {
        return Vec::new();
    }
    let line = LineString::from(vec![
        Coord {
            x: start.x,
            y: start.y,
        },
        Coord { x: end.x, y: end.y },
    ]);
    line.buffer(width / 2.0)
        .0
        .into_iter()
        .map(|polygon| points_from_linestring(polygon.exterior()))
        .collect()
}

fn union_polygons(polygons: &[Vec<Point>]) -> Vec<Vec<Point>> {
    union_polygon_groups(polygons, 0.0)
}

fn union_polygon_groups(polygons: &[Vec<Point>], margin: f64) -> Vec<Vec<Point>> {
    let polygons = polygons
        .iter()
        .filter(|points| points.len() >= 3)
        .cloned()
        .collect::<Vec<_>>();
    let groups = polygon_groups(&polygons, margin);
    let mut result = Vec::new();
    for group in groups {
        let shapes = group
            .into_iter()
            .filter_map(|index| polygon_from_points(&polygons[index]))
            .collect::<Vec<_>>();
        if !shapes.is_empty() {
            result.extend(rings_from_multi_polygon(unary_union(shapes.iter())));
        }
    }
    result
}

fn polygon_groups(polygons: &[Vec<Point>], margin: f64) -> Vec<Vec<usize>> {
    let bounds = polygons
        .iter()
        .filter_map(|points| polygon_bounds(points))
        .collect::<Vec<_>>();
    let mut groups = Vec::<Vec<usize>>::new();
    let mut assigned = vec![false; bounds.len()];
    for start in 0..bounds.len() {
        if assigned[start] {
            continue;
        }
        assigned[start] = true;
        let mut group = vec![start];
        let mut cursor = 0;
        while cursor < group.len() {
            let current = group[cursor];
            for candidate in 0..bounds.len() {
                if !assigned[candidate]
                    && bounds_overlap(bounds[current], bounds[candidate], margin)
                {
                    assigned[candidate] = true;
                    group.push(candidate);
                }
            }
            cursor += 1;
        }
        groups.push(group);
    }
    groups
}

#[derive(Clone, Copy)]
struct PolygonBounds {
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
}

fn polygon_bounds(points: &[Point]) -> Option<PolygonBounds> {
    let first = points.first()?;
    Some(points.iter().skip(1).fold(
        PolygonBounds {
            min_x: first.x,
            max_x: first.x,
            min_y: first.y,
            max_y: first.y,
        },
        |bounds, point| PolygonBounds {
            min_x: bounds.min_x.min(point.x),
            max_x: bounds.max_x.max(point.x),
            min_y: bounds.min_y.min(point.y),
            max_y: bounds.max_y.max(point.y),
        },
    ))
}

fn bounds_overlap(left: PolygonBounds, right: PolygonBounds, margin: f64) -> bool {
    left.min_x <= right.max_x + margin
        && left.max_x + margin >= right.min_x
        && left.min_y <= right.max_y + margin
        && left.max_y + margin >= right.min_y
}

fn map_opening_sources(source_openings: &[Vec<Point>], openings: &[Vec<Point>]) -> Vec<Vec<usize>> {
    openings
        .iter()
        .filter_map(|points| polygon_bounds(points))
        .map(|opening_bounds| {
            source_openings
                .iter()
                .enumerate()
                .filter_map(|(index, source)| {
                    polygon_bounds(source)
                        .filter(|source_bounds| {
                            bounds_overlap(*source_bounds, opening_bounds, 1e-6)
                        })
                        .map(|_| index)
                })
                .collect()
        })
        .collect()
}

fn offset_polygons(polygons: &[Vec<Point>], distance: f64) -> Vec<Vec<Point>> {
    if distance.abs() <= GEOMETRY_EPSILON {
        return polygons.to_vec();
    }
    let shapes = polygons
        .iter()
        .filter_map(|points| polygon_from_points(points))
        .collect::<Vec<_>>();
    if shapes.is_empty() {
        return Vec::new();
    }
    rings_from_multi_polygon(unary_union(shapes.iter()).buffer(distance))
}

fn compensate_for_nozzle(
    polygons: &[Vec<Point>],
    nozzle_diameter: f64,
    merge_close_pads: bool,
    fill_unprintable_grids: bool,
) -> (Vec<Vec<Point>>, bool) {
    if nozzle_diameter <= GEOMETRY_EPSILON {
        return (polygons.to_vec(), false);
    }

    let mut compensated = Vec::new();
    let mut changed = false;
    for polygon in polygons {
        let Some(shape) = polygon_from_points(polygon) else {
            continue;
        };
        let bounds = shape.exterior().0.iter().fold(
            (
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ),
            |(min_x, max_x, min_y, max_y), coordinate| {
                (
                    min_x.min(coordinate.x),
                    max_x.max(coordinate.x),
                    min_y.min(coordinate.y),
                    max_y.max(coordinate.y),
                )
            },
        );
        let narrowest = (bounds.1 - bounds.0).min(bounds.3 - bounds.2);
        let widen = ((nozzle_diameter - narrowest) / 2.0).max(0.0);
        if widen > GEOMETRY_EPSILON {
            changed = true;
            compensated.extend(
                shape
                    .buffer(widen)
                    .0
                    .into_iter()
                    .map(|polygon| points_from_linestring(polygon.exterior())),
            );
        } else {
            compensated.push(polygon.clone());
        }
    }
    if compensated.is_empty() {
        return (Vec::new(), changed);
    }
    let mut combined = union_polygons(&compensated);
    if merge_close_pads {
        let radius = nozzle_diameter / 2.0;
        let merged = morphological_close(&combined, radius);
        if merged.len() != combined.len() {
            changed = true;
        }
        combined = merged;
    }

    if fill_unprintable_grids && !merge_close_pads {
        let filled = morphological_close(&combined, nozzle_diameter / 2.0);
        if filled.len() != combined.len() {
            changed = true;
        }
        combined = filled;
    }
    (combined, changed)
}

fn morphological_close(polygons: &[Vec<Point>], radius: f64) -> Vec<Vec<Point>> {
    if radius <= GEOMETRY_EPSILON {
        return polygons.to_vec();
    }
    let mut result = Vec::new();
    for group in polygon_groups(polygons, radius * 2.0 + GEOMETRY_EPSILON) {
        let shapes = group
            .into_iter()
            .filter_map(|index| polygon_from_points(&polygons[index]))
            .collect::<Vec<_>>();
        if shapes.is_empty() {
            continue;
        }
        let expanded = shapes
            .into_iter()
            .flat_map(|shape| {
                shape
                    .buffer(radius)
                    .0
                    .into_iter()
                    .map(|polygon| points_from_linestring(polygon.exterior()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        result.extend(union_polygons(&expanded));
    }
    result
}

fn clip_openings(
    openings: &[Vec<Point>],
    boundary: &[Point],
) -> Result<(Vec<Vec<Point>>, bool), String> {
    let opening_shapes = openings
        .iter()
        .filter_map(|points| polygon_from_points(points))
        .collect::<Vec<_>>();
    let boundary = polygon_from_points(boundary)
        .ok_or_else(|| "The printable plate outline is degenerate.".to_string())?;
    if openings.iter().all(|opening| {
        opening
            .iter()
            .all(|point| boundary.contains(&GeoPoint::new(point.x, point.y)))
    }) {
        return Ok((openings.to_vec(), false));
    }
    let source = unary_union(opening_shapes.iter());
    let source_area = source.unsigned_area();
    let clipped = source.intersection(&boundary);
    let clipped_area = clipped.unsigned_area();
    Ok((
        rings_from_multi_polygon(clipped),
        source_area - clipped_area > GEOMETRY_EPSILON,
    ))
}

fn rings_from_multi_polygon(multi_polygon: MultiPolygon<f64>) -> Vec<Vec<Point>> {
    multi_polygon
        .0
        .into_iter()
        .map(|polygon| points_from_linestring(polygon.exterior()))
        .filter(|points| points.len() >= 3 && signed_area(points).abs() > GEOMETRY_EPSILON)
        .map(ensure_ccw)
        .collect()
}

fn validate_openings(plate: &[Point], openings: &[Vec<Point>]) -> Result<(), String> {
    let plate = polygon_from_points(plate)
        .ok_or_else(|| "The printable plate outline is degenerate.".to_string())?;
    let safety_margin = plate.buffer(0.000001);
    for (index, opening) in openings.iter().enumerate() {
        if !opening
            .iter()
            .all(|point| safety_margin.contains(&GeoPoint::new(point.x, point.y)))
        {
            return Err(format!(
                "Paste opening {} extends outside the board clearance area. Check that the paste and Edge.Cuts files use the same coordinates.",
                index + 1
            ));
        }
    }
    Ok(())
}

fn regular_polygon(center: Point, diameter: f64, sides: usize, rotation: f64) -> Vec<Point> {
    (0..sides)
        .map(|index| {
            let angle = rotation + TAU * index as f64 / sides as f64;
            Point {
                x: center.x + diameter / 2.0 * angle.cos(),
                y: center.y + diameter / 2.0 * angle.sin(),
            }
        })
        .collect()
}

fn rectangle(center: Point, width: f64, height: f64) -> Vec<Point> {
    let (half_width, half_height) = (width / 2.0, height / 2.0);
    vec![
        Point {
            x: center.x - half_width,
            y: center.y - half_height,
        },
        Point {
            x: center.x + half_width,
            y: center.y - half_height,
        },
        Point {
            x: center.x + half_width,
            y: center.y + half_height,
        },
        Point {
            x: center.x - half_width,
            y: center.y + half_height,
        },
    ]
}

fn obround(center: Point, width: f64, height: f64) -> Vec<Point> {
    let (radius, straight, vertical) = if width >= height {
        (height / 2.0, width - height, false)
    } else {
        (width / 2.0, height - width, true)
    };
    (0..CURVE_SEGMENTS)
        .map(|index| {
            let angle = TAU * index as f64 / CURVE_SEGMENTS as f64;
            let x_offset = if vertical {
                0.0
            } else if angle.sin() >= 0.0 {
                straight / 2.0
            } else {
                -straight / 2.0
            };
            let y_offset = if !vertical {
                0.0
            } else if angle.cos() >= 0.0 {
                straight / 2.0
            } else {
                -straight / 2.0
            };
            Point {
                x: center.x + x_offset + radius * angle.cos(),
                y: center.y + y_offset + radius * angle.sin(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gerber::bounds;

    fn layer(name: &str, points: Vec<Point>) -> ParsedLayer {
        ParsedLayer {
            filename: name.into(),
            units: "mm".into(),
            primitives: vec![Primitive::Region(points)],
            warnings: Vec::new(),
        }
    }

    fn square(x: f64, y: f64, size: f64) -> Vec<Point> {
        vec![
            Point { x, y },
            Point { x: x + size, y },
            Point {
                x: x + size,
                y: y + size,
            },
            Point { x, y: y + size },
        ]
    }

    fn options(
        clearance: f64,
        wall_thickness: f64,
        nozzle_diameter: f64,
        enable_slotify: bool,
        drop_unprintable_grids: bool,
        mirror_back: bool,
    ) -> GeometryOptions {
        GeometryOptions {
            clearance,
            wall_thickness,
            shrink: 0.0,
            nozzle_diameter,
            enable_slotify,
            drop_unprintable_grids,
            mirror_back,
        }
    }

    #[test]
    fn builds_plate_wall_and_openings() {
        let paste = layer("paste.gtp", square(2.0, 2.0, 1.0));
        let edge = layer("edge.gm1", square(0.0, 0.0, 10.0));
        let geometry =
            StencilGeometry::from_layers(&paste, &edge, options(1.0, 2.0, 0.4, true, true, false))
                .unwrap();

        assert_eq!(geometry.openings.len(), 1);
        let plate_bounds = bounds(&geometry.plate).unwrap();
        let outer_wall_bounds = bounds(&geometry.outer_wall).unwrap();
        assert!((plate_bounds.width() - outer_wall_bounds.width()).abs() < 1e-9);
        assert!((plate_bounds.height() - outer_wall_bounds.height()).abs() < 1e-9);
        assert!(outer_wall_bounds.width() > 10.0);
        assert!(bounds(&geometry.inner_wall).unwrap().width() > 10.0);
    }

    #[test]
    fn compensates_small_openings_and_merges_unprintable_gaps() {
        let paste = ParsedLayer {
            filename: "paste.gtp".into(),
            units: "mm".into(),
            primitives: vec![
                Primitive::Region(square(2.0, 2.0, 0.2)),
                Primitive::Region(square(2.3, 2.0, 0.2)),
            ],
            warnings: Vec::new(),
        };
        let edge = layer("edge.gm1", square(0.0, 0.0, 10.0));

        let geometry =
            StencilGeometry::from_layers(&paste, &edge, options(0.0, 1.0, 0.4, true, true, false))
                .unwrap();

        assert_eq!(geometry.openings.len(), 1);
        assert!(geometry
            .warnings
            .iter()
            .any(|warning| warning.contains("Nozzle compensation")));
    }

    #[test]
    fn merges_only_gaps_smaller_than_the_selected_nozzle() {
        let paste = ParsedLayer {
            filename: "paste.gtp".into(),
            units: "mm".into(),
            primitives: vec![
                Primitive::Region(square(1.0, 1.0, 1.0)),
                Primitive::Region(square(2.39, 1.0, 1.0)),
                Primitive::Region(square(1.0, 4.0, 1.0)),
                Primitive::Region(square(2.41, 4.0, 1.0)),
            ],
            warnings: Vec::new(),
        };
        let edge = layer("edge.gm1", square(0.0, 0.0, 10.0));
        let geometry =
            StencilGeometry::from_layers(&paste, &edge, options(0.0, 1.0, 0.4, true, false, false))
                .unwrap();

        assert_eq!(geometry.selection_openings.len(), 4);
        assert_eq!(geometry.openings.len(), 3);
    }

    #[test]
    fn clips_paste_outside_the_board_clearance() {
        let paste = layer("paste.gtp", square(9.5, 4.0, 2.0));
        let edge = layer("edge.gm1", square(0.0, 0.0, 10.0));
        let geometry = StencilGeometry::from_layers(
            &paste,
            &edge,
            options(0.0, 1.0, 0.4, false, false, false),
        )
        .unwrap();
        assert!(geometry
            .warnings
            .iter()
            .any(|warning| warning.contains("clipped")));
        assert!(geometry.openings[0].iter().all(|point| point.x <= 10.0001));
    }

    #[test]
    fn mirrors_back_paste_around_board_centre() {
        let paste = layer("paste.gtp", square(1.0, 2.0, 1.0));
        let edge = layer("edge.gm1", square(0.0, 0.0, 10.0));
        let geometry =
            StencilGeometry::from_layers(&paste, &edge, options(0.0, 1.0, 0.0, false, false, true))
                .unwrap();
        let bounds = crate::gerber::bounds(&geometry.openings[0]).unwrap();
        assert!((bounds.min_x - 8.0).abs() < 1e-6);
        assert!((bounds.max_x - 9.0).abs() < 1e-6);
    }
}
