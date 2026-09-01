use crate::{
    geometry::{distance_squared, signed_area, StencilGeometry},
    gerber::Point,
};

const TRIANGLE_EPSILON: f64 = 1e-10;
const RING_EPSILON: f64 = 1e-7;

#[derive(Clone, Copy)]
struct Triangle {
    a: [f64; 3],
    b: [f64; 3],
    c: [f64; 3],
}

/// Serializes the plate and registration wall as an ASCII STL.
///
/// STL is deliberately generated from the same planar profiles as STEP. The
/// The stencil plate occupies z=0..thickness and the registration wall hangs
/// below it at z=-height..0. Top and bottom surfaces are triangulated with the
/// paste openings as holes, while the profile edges become the vertical faces.
/// This produces a file that can be opened directly by common slicers without
/// a CAD conversion step.
pub(crate) fn write_stl(
    geometry: &StencilGeometry,
    wall_height: f64,
    stencil_thickness: f64,
) -> Result<String, String> {
    let mut triangles = Vec::new();
    add_surface(
        &mut triangles,
        &geometry.plate,
        &geometry.openings,
        stencil_thickness,
        true,
    )?;
    add_surface(
        &mut triangles,
        &geometry.plate,
        &geometry.openings,
        0.0,
        false,
    )?;
    add_side_faces(
        &mut triangles,
        &clean_ring(&geometry.plate, true)?,
        0.0,
        stencil_thickness,
    );
    add_opening_sides(&mut triangles, &geometry.openings, 0.0, stencil_thickness)?;

    add_surface(
        &mut triangles,
        &geometry.outer_wall,
        std::slice::from_ref(&geometry.inner_wall),
        -wall_height,
        false,
    )?;
    add_side_faces(
        &mut triangles,
        &clean_ring(&geometry.outer_wall, true)?,
        -wall_height,
        0.0,
    );
    add_side_faces(
        &mut triangles,
        &clean_ring(&geometry.inner_wall, false)?,
        -wall_height,
        0.0,
    );
    if triangles.is_empty() {
        return Err("The stencil produced no printable triangles.".into());
    }

    let mut output = String::from("solid stencil_print\n");
    for triangle in triangles {
        let normal = normal(triangle);
        output.push_str(&format!(
            "  facet normal {:.7} {:.7} {:.7}\n    outer loop\n",
            normal[0], normal[1], normal[2]
        ));
        for point in [triangle.a, triangle.b, triangle.c] {
            output.push_str(&format!(
                "      vertex {:.7} {:.7} {:.7}\n",
                point[0], point[1], point[2]
            ));
        }
        output.push_str("    endloop\n  endfacet\n");
    }
    output.push_str("endsolid stencil_print\n");
    Ok(output)
}

fn add_surface(
    triangles: &mut Vec<Triangle>,
    outer: &[Point],
    holes: &[Vec<Point>],
    z: f64,
    upward: bool,
) -> Result<(), String> {
    let outer = clean_ring(outer, true)?;
    let holes = holes
        .iter()
        .map(|hole| clean_ring(hole, false))
        .collect::<Result<Vec<_>, _>>()?;
    let surface_triangles = triangulate(&outer, &holes)?;

    for [a, b, c] in surface_triangles {
        add_oriented_surface_triangle(triangles, a, b, c, z, upward);
    }
    Ok(())
}

fn add_opening_sides(
    triangles: &mut Vec<Triangle>,
    openings: &[Vec<Point>],
    bottom: f64,
    top: f64,
) -> Result<(), String> {
    for opening in openings {
        add_side_faces(triangles, &clean_ring(opening, false)?, bottom, top);
    }
    Ok(())
}

fn clean_ring(points: &[Point], counter_clockwise: bool) -> Result<Vec<Point>, String> {
    let mut cleaned = Vec::with_capacity(points.len());
    for point in points.iter().copied() {
        if cleaned
            .last()
            .is_none_or(|previous| distance_squared(*previous, point) > RING_EPSILON.powi(2))
        {
            cleaned.push(point);
        }
    }
    if cleaned.len() > 1
        && distance_squared(cleaned[0], *cleaned.last().unwrap()) <= RING_EPSILON.powi(2)
    {
        cleaned.pop();
    }
    if cleaned.len() < 3 {
        return Err("A printable profile has fewer than three distinct points.".into());
    }
    let area = signed_area(&cleaned);
    if area.abs() <= TRIANGLE_EPSILON {
        return Err("A printable profile is degenerate.".into());
    }
    if (area > 0.0) != counter_clockwise {
        cleaned.reverse();
    }
    Ok(cleaned)
}

fn triangulate(outer: &[Point], holes: &[Vec<Point>]) -> Result<Vec<[Point; 3]>, String> {
    let mut coordinates = Vec::new();
    let mut hole_indices = Vec::new();
    append_ring(&mut coordinates, outer);
    for hole in holes {
        hole_indices.push(coordinates.len() / 2);
        append_ring(&mut coordinates, hole);
    }

    let indices = earcutr::earcut(&coordinates, &hole_indices, 2)
        .map_err(|error| format!("Could not triangulate a stencil profile: {error:?}"))?;
    Ok(indices
        .chunks_exact(3)
        .filter_map(|triangle| {
            let point = |index: usize| Point {
                x: coordinates[index * 2],
                y: coordinates[index * 2 + 1],
            };
            let points = [point(triangle[0]), point(triangle[1]), point(triangle[2])];
            (signed_area(&points).abs() > TRIANGLE_EPSILON).then_some(points)
        })
        .collect())
}

fn append_ring(coordinates: &mut Vec<f64>, points: &[Point]) {
    coordinates.extend(points.iter().flat_map(|point| [point.x, point.y]));
}

fn add_oriented_surface_triangle(
    triangles: &mut Vec<Triangle>,
    a: Point,
    b: Point,
    c: Point,
    z: f64,
    upward: bool,
) {
    let area = signed_area(&[a, b, c]);
    if (area > 0.0) == upward {
        triangles.push(Triangle {
            a: [a.x, a.y, z],
            b: [b.x, b.y, z],
            c: [c.x, c.y, z],
        });
    } else {
        triangles.push(Triangle {
            a: [a.x, a.y, z],
            b: [c.x, c.y, z],
            c: [b.x, b.y, z],
        });
    }
}

fn add_side_faces(triangles: &mut Vec<Triangle>, points: &[Point], bottom: f64, top: f64) {
    for index in 0..points.len() {
        let next = (index + 1) % points.len();
        let start = points[index];
        let end = points[next];
        let start_bottom = [start.x, start.y, bottom];
        let end_bottom = [end.x, end.y, bottom];
        let end_top = [end.x, end.y, top];
        let start_top = [start.x, start.y, top];
        triangles.push(Triangle {
            a: start_bottom,
            b: end_bottom,
            c: end_top,
        });
        triangles.push(Triangle {
            a: start_bottom,
            b: end_top,
            c: start_top,
        });
    }
}

fn normal(triangle: Triangle) -> [f64; 3] {
    let ab = [
        triangle.b[0] - triangle.a[0],
        triangle.b[1] - triangle.a[1],
        triangle.b[2] - triangle.a[2],
    ];
    let ac = [
        triangle.c[0] - triangle.a[0],
        triangle.c[1] - triangle.a[1],
        triangle.c[2] - triangle.a[2],
    ];
    let cross = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
    if length <= TRIANGLE_EPSILON {
        [0.0, 0.0, 0.0]
    } else {
        [cross[0] / length, cross[1] / length, cross[2] / length]
    }
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
    fn writes_slicer_ready_stl_with_a_cut_through_opening() {
        let geometry = StencilGeometry {
            plate: square(10.0),
            openings: vec![square(1.0)],
            inner_wall: square(10.0),
            outer_wall: square(12.0),
            warnings: Vec::new(),
        };

        let stl = write_stl(&geometry, 2.0, 0.4).expect("profiles should export");
        assert!(stl.starts_with("solid stencil_print\n"));
        assert!(stl.contains("facet normal"));
        assert!(stl.contains("vertex 0.0000000 0.0000000 0.4000000"));
        assert!(stl.ends_with("endsolid stencil_print\n"));
    }
}
