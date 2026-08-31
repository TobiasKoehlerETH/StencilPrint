use crate::gerber::{Aperture, MacroShape, ParsedLayer, Point, Primitive};
use geo::{unary_union, Area, Buffer, Contains, Coord, LineString, Point as GeoPoint, Polygon};
use std::f64::consts::TAU;

const CURVE_SEGMENTS: usize = 32;
const OUTLINE_TOLERANCE: f64 = 0.01;

#[derive(Clone)]
pub(crate) struct StencilGeometry {
    pub plate: Vec<Point>,
    pub openings: Vec<Vec<Point>>,
    pub inner_wall: Vec<Point>,
    pub outer_wall: Vec<Point>,
}

impl StencilGeometry {
    pub fn from_layers(
        paste: &ParsedLayer,
        edge: &ParsedLayer,
        clearance: f64,
        wall_thickness: f64,
    ) -> Result<Self, String> {
        let board = outline_from_edge(edge, &paste.points())?;
        let inner_wall = offset_polygon(&board, clearance)?;
        let outer_wall = offset_polygon(&board, clearance + wall_thickness)?;
        let openings = union_polygons(&primitive_polygons(paste));
        if openings.is_empty() {
            return Err("The paste layer contains no printable pad openings.".into());
        }
        validate_openings(&inner_wall, &openings)?;
        Ok(Self {
            plate: inner_wall.clone(),
            openings,
            outer_wall,
            inner_wall,
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
        .filter(|points| polygon_area(points).abs() > 1e-9)
        .max_by(|left, right| {
            polygon_area(left)
                .abs()
                .partial_cmp(&polygon_area(right).abs())
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
    if polygon_area(&polygon) < 0.0 {
        polygon.reverse();
    }
    polygon
}

fn polygon_area(points: &[Point]) -> f64 {
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
        .filter(|points| polygon_area(points).abs() > 1e-9)
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
    points
}

fn primitive_polygons(layer: &ParsedLayer) -> Vec<Vec<Point>> {
    let mut polygons = Vec::new();
    for primitive in &layer.primitives {
        match primitive {
            Primitive::Flash(point, aperture) => {
                polygons.extend(aperture_polygons(*point, aperture));
            }
            Primitive::Region(points) if points.len() >= 3 => polygons.push(points.clone()),
            Primitive::Stroke(start, end, aperture) => {
                polygons.extend(stroke_polygons(*start, *end, aperture));
            }
            _ => {}
        }
    }
    polygons
        .into_iter()
        .filter(|polygon| polygon.len() >= 3 && polygon_area(polygon).abs() > 1e-9)
        .collect()
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
    let shapes = polygons
        .iter()
        .filter_map(|points| polygon_from_points(points))
        .collect::<Vec<_>>();
    if shapes.is_empty() {
        return Vec::new();
    }
    unary_union(shapes.iter())
        .0
        .into_iter()
        .map(|polygon| points_from_linestring(polygon.exterior()))
        .filter(|points| points.len() >= 3 && polygon_area(points).abs() > 1e-9)
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

    #[test]
    fn builds_plate_wall_and_openings() {
        let paste = layer("paste.gtp", square(2.0, 2.0, 1.0));
        let edge = layer("edge.gm1", square(0.0, 0.0, 10.0));
        let geometry = StencilGeometry::from_layers(&paste, &edge, 1.0, 2.0).unwrap();

        assert_eq!(geometry.openings.len(), 1);
        assert!(bounds(&geometry.outer_wall).unwrap().width() > 10.0);
        assert!(bounds(&geometry.inner_wall).unwrap().width() > 10.0);
    }
}
