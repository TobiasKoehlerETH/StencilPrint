use crate::{geometry::StencilGeometry, gerber::Point};
use brepkit_io::step::write_step as write_brep_step;
use brepkit_math::vec::Point3;
use brepkit_operations::extrude::extrude;
use brepkit_topology::{builder::make_face_from_wire, builder::make_polygon_wire, Topology};

const TOPOLOGY_TOLERANCE: f64 = 1e-7;

/// Builds the two stencil solids and delegates STEP serialization to brepkit.
///
/// The browser-facing geometry remains polygonal because it is also used by
/// the Three.js preview. brepkit turns those clean planar profiles into
/// watertight B-Reps and writes a standard AP203 STEP document.
pub(crate) fn write_step(
    geometry: &StencilGeometry,
    wall_height: f64,
    stencil_thickness: f64,
) -> Result<String, String> {
    let mut topology = Topology::new();
    let plate_face = planar_face_with_holes(&mut topology, &geometry.plate, &geometry.openings)
        .map_err(|error| format!("Could not construct the stencil plate: {error}"))?;
    let wall_face = planar_face_with_holes(
        &mut topology,
        &geometry.outer_wall,
        std::slice::from_ref(&geometry.inner_wall),
    )
    .map_err(|error| format!("Could not construct the registration wall: {error}"))?;

    let plate = extrude(
        &mut topology,
        plate_face,
        brepkit_math::vec::Vec3::new(0.0, 0.0, 1.0),
        stencil_thickness,
    )
    .map_err(|error| format!("Could not extrude the stencil plate: {error}"))?;
    let wall = extrude(
        &mut topology,
        wall_face,
        brepkit_math::vec::Vec3::new(0.0, 0.0, -1.0),
        wall_height,
    )
    .map_err(|error| format!("Could not extrude the registration wall: {error}"))?;

    write_brep_step(&topology, &[plate, wall])
        .map_err(|error| format!("Could not serialize the STEP document: {error}"))
}

fn planar_face_with_holes(
    topology: &mut Topology,
    outer: &[Point],
    holes: &[Vec<Point>],
) -> Result<brepkit_topology::FaceId, String> {
    let outer = oriented_polygon(outer, true)?;
    let outer_wire = make_polygon_wire(
        topology,
        &outer
            .iter()
            .map(|point| Point3::new(point.x, point.y, 0.0))
            .collect::<Vec<_>>(),
        TOPOLOGY_TOLERANCE,
    )
    .map_err(|error| error.to_string())?;
    let face = make_face_from_wire(topology, outer_wire).map_err(|error| error.to_string())?;

    let inner_wires = holes
        .iter()
        .filter(|hole| hole.len() >= 3)
        .map(|hole| {
            let hole = oriented_polygon(hole, false)?;
            let points = hole
                .iter()
                .map(|point| Point3::new(point.x, point.y, 0.0))
                .collect::<Vec<_>>();
            make_polygon_wire(topology, &points, TOPOLOGY_TOLERANCE)
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    topology
        .face_mut(face)
        .map_err(|error| error.to_string())?
        .inner_wires_mut()
        .extend(inner_wires);
    Ok(face)
}

fn oriented_polygon(points: &[Point], counter_clockwise: bool) -> Result<Vec<Point>, String> {
    let mut cleaned = Vec::with_capacity(points.len());
    for point in points.iter().copied() {
        if cleaned.last().is_none_or(|previous: &Point| {
            distance_squared(*previous, point) > TOPOLOGY_TOLERANCE.powi(2)
        }) {
            cleaned.push(point);
        }
    }
    if cleaned.len() > 1
        && distance_squared(cleaned[0], *cleaned.last().unwrap()) <= TOPOLOGY_TOLERANCE.powi(2)
    {
        cleaned.pop();
    }
    if cleaned.len() < 3 {
        return Err("profile has fewer than three distinct points".into());
    }

    let area = signed_area(&cleaned);
    if area.abs() <= TOPOLOGY_TOLERANCE {
        return Err("profile is degenerate".into());
    }
    if (area > 0.0) != counter_clockwise {
        cleaned.reverse();
    }
    Ok(cleaned)
}

fn signed_area(points: &[Point]) -> f64 {
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

fn distance_squared(a: Point, b: Point) -> f64 {
    (a.x - b.x).powi(2) + (a.y - b.y).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(size: f64) -> Vec<Point> {
        vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: size, y: 0.0 },
            Point { x: size, y: size },
            Point { x: 0.0, y: size },
        ]
    }

    #[test]
    fn writes_watertight_step_solids_from_profiles() {
        let geometry = StencilGeometry {
            plate: square(10.0),
            openings: vec![square(1.0)],
            inner_wall: square(10.0),
            outer_wall: square(12.0),
        };

        let step = write_step(&geometry, 2.0, 0.4).expect("profiles should export");
        assert!(step.starts_with("ISO-10303-21;"));
        assert!(step.contains("ADVANCED_BREP_SHAPE_REPRESENTATION"));
        assert_eq!(step.matches("MANIFOLD_SOLID_BREP").count(), 2);
        assert!(step.ends_with("END-ISO-10303-21;\n"));

        let mut imported = Topology::new();
        let solids = brepkit_io::step::read_step(&step, &mut imported)
            .expect("export should round-trip through the STEP reader");
        assert_eq!(solids.len(), 2);
    }

    #[test]
    fn orients_outer_and_inner_profiles_for_extrusion() {
        let clockwise = square(10.0).into_iter().rev().collect::<Vec<_>>();
        let outer = oriented_polygon(&clockwise, true).expect("outer profile should be valid");
        let inner = oriented_polygon(&square(1.0), false).expect("inner profile should be valid");
        assert!(signed_area(&outer) > 0.0);
        assert!(signed_area(&inner) < 0.0);
    }
}
