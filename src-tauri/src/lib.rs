mod geometry;
mod gerber;
mod step;
mod stl;

use geometry::{preview_svg, StencilGeometry};
use gerber::{parse_source, LayerKind, LayerSource, LayerStats, ParsedLayer};
use serde::{Deserialize, Serialize};
use step::write_step;
use stl::write_stl;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StencilSettings {
    clearance: f64,
    wall_thickness: f64,
    wall_height: f64,
    stencil_thickness: f64,
}

impl StencilSettings {
    fn validate(self) -> Result<(), String> {
        let dimensions = [
            ("Clearance", self.clearance, true),
            ("Wall thickness", self.wall_thickness, false),
            ("Wall height", self.wall_height, false),
            ("Stencil thickness", self.stencil_thickness, false),
        ];
        for (name, value, allows_zero) in dimensions {
            if !value.is_finite() || value < 0.0 || (!allows_zero && value == 0.0) {
                return Err(format!(
                    "{name} must be {}.",
                    if allows_zero {
                        "zero or positive"
                    } else {
                        "positive"
                    }
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewRequest {
    paste: LayerSource,
    edge: LayerSource,
    settings: StencilSettings,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportRequest {
    #[serde(flatten)]
    input: PreviewRequest,
    #[serde(default)]
    mirror: bool,
    #[serde(default)]
    excluded_openings: Vec<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewResult {
    paste: LayerStats,
    edge: LayerStats,
    preview_svg: String,
    model: PreviewModel,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewModel {
    plate: Vec<gerber::Point>,
    openings: Vec<Vec<gerber::Point>>,
    inner_wall: Vec<gerber::Point>,
    outer_wall: Vec<gerber::Point>,
}

#[derive(Serialize)]
struct ExportResult {
    filename: String,
    step: String,
    summary: Vec<String>,
}

#[derive(Serialize)]
struct StlExportResult {
    filename: String,
    stl: String,
    summary: Vec<String>,
}

fn parse_layers(request: &PreviewRequest) -> Result<(ParsedLayer, ParsedLayer), String> {
    request.settings.validate()?;
    Ok((
        parse_source(&request.paste, LayerKind::Paste)?,
        parse_source(&request.edge, LayerKind::Edge)?,
    ))
}

fn geometry_for(
    request: &PreviewRequest,
) -> Result<(ParsedLayer, ParsedLayer, StencilGeometry), String> {
    let (paste, edge) = parse_layers(request)?;
    let geometry = StencilGeometry::from_layers(
        &paste,
        &edge,
        request.settings.clearance,
        request.settings.wall_thickness,
    )?;
    Ok((paste, edge, geometry))
}

#[tauri::command]
fn preview_stencil(request: PreviewRequest) -> Result<PreviewResult, String> {
    let (paste, edge, geometry) = geometry_for(&request)?;
    Ok(PreviewResult {
        paste: paste.stats(),
        edge: edge.stats(),
        preview_svg: preview_svg(&geometry),
        model: PreviewModel {
            plate: geometry.plate.clone(),
            openings: geometry.openings.clone(),
            inner_wall: geometry.inner_wall.clone(),
            outer_wall: geometry.outer_wall.clone(),
        },
    })
}

#[tauri::command]
fn export_stencil_step(request: ExportRequest) -> Result<ExportResult, String> {
    let (paste, _, mut geometry) = geometry_for(&request.input)?;
    exclude_openings(&mut geometry, &request.excluded_openings);
    if request.mirror {
        geometry.mirror_x();
    }
    let settings = request.input.settings;
    let step = write_step(&geometry, settings.wall_height, settings.stencil_thickness)?;
    let stem = export_stem(&request.input.paste.filename);
    Ok(ExportResult {
        filename: format!("{stem}_stencil.step"),
        step,
        summary: vec![
            format!("{} paste objects", paste.primitives.len()),
            format!("{} mm wall", settings.wall_height),
            format!("{} mm clearance", settings.clearance),
        ],
    })
}

#[tauri::command]
fn export_stencil_stl(request: ExportRequest) -> Result<StlExportResult, String> {
    let (paste, _, mut geometry) = geometry_for(&request.input)?;
    exclude_openings(&mut geometry, &request.excluded_openings);
    if request.mirror {
        geometry.mirror_x();
    }
    let settings = request.input.settings;
    let stl = write_stl(&geometry, settings.wall_height, settings.stencil_thickness)?;
    let stem = export_stem(&request.input.paste.filename);
    Ok(StlExportResult {
        filename: format!("{stem}_stencil.stl"),
        stl,
        summary: vec![
            format!("{} paste objects", paste.primitives.len()),
            format!("{} mm wall", settings.wall_height),
            format!("{} mm clearance", settings.clearance),
        ],
    })
}

fn export_stem(filename: &str) -> &str {
    filename.rsplit_once('.').map_or(filename, |(stem, _)| stem)
}

fn exclude_openings(geometry: &mut StencilGeometry, excluded: &[usize]) {
    if excluded.is_empty() {
        return;
    }
    geometry.openings = geometry
        .openings
        .drain(..)
        .enumerate()
        .filter_map(|(index, opening)| (!excluded.contains(&index)).then_some(opening))
        .collect();
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            preview_stencil,
            export_stencil_step,
            export_stencil_stl
        ])
        .run(tauri::generate_context!())
        .expect("error while running StencilPrint");
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn settings() -> StencilSettings {
        StencilSettings {
            clearance: 0.3,
            wall_thickness: 1.0,
            wall_height: 1.0,
            stencil_thickness: 0.4,
        }
    }

    fn sample_zip_source() -> LayerSource {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("gerber-sample")
            .join("prod_main.zip");
        let bytes = std::fs::read(&path).unwrap_or_else(|error| {
            panic!("could not read {}: {error}", path.display());
        });
        LayerSource {
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
            filename: "prod_main.zip".into(),
            is_zip: true,
        }
    }

    fn inline_source(filename: &str, data: &str) -> LayerSource {
        LayerSource {
            data: data.into(),
            filename: filename.into(),
            is_zip: false,
        }
    }

    #[test]
    fn accepts_valid_settings() {
        assert!(settings().validate().is_ok());
    }

    #[test]
    fn rejects_non_positive_dimensions() {
        let mut invalid = settings();
        invalid.wall_height = 0.0;
        assert_eq!(
            invalid.validate().unwrap_err(),
            "Wall height must be positive."
        );
    }

    #[test]
    fn loads_sample_gerber_zip_and_builds_preview_and_step() {
        let preview = preview_stencil(PreviewRequest {
            paste: sample_zip_source(),
            edge: sample_zip_source(),
            settings: settings(),
        })
        .expect("sample ZIP should build a preview");

        assert!(preview.preview_svg.starts_with("<svg"));
        assert!(preview.preview_svg.contains("<polygon class=\"opening\""));
        assert!(preview.model.plate.len() >= 4);
        assert!(!preview.model.openings.is_empty());
        assert!(preview
            .model
            .openings
            .iter()
            .all(|opening| opening.len() >= 3));
        assert!(preview.model.openings.iter().all(|opening| {
            opening
                .windows(2)
                .any(|pair| (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y) > 1e-6)
        }));
        assert!(preview.model.inner_wall.len() > 100);
        assert!(preview.model.outer_wall.len() > 100);

        let export = export_stencil_step(ExportRequest {
            input: PreviewRequest {
                paste: sample_zip_source(),
                edge: sample_zip_source(),
                settings: settings(),
            },
            mirror: false,
            excluded_openings: Vec::new(),
        })
        .expect("sample ZIP should export a STEP stencil");

        assert_eq!(export.filename, "prod_main_stencil.step");
        assert!(export.step.starts_with("ISO-10303-21;"));
        assert!(export.step.contains("ADVANCED_BREP_SHAPE_REPRESENTATION"));
        assert_eq!(export.step.matches("MANIFOLD_SOLID_BREP").count(), 2);
        assert!(export.step.ends_with("END-ISO-10303-21;\n"));

        let stl = export_stencil_stl(ExportRequest {
            input: PreviewRequest {
                paste: sample_zip_source(),
                edge: sample_zip_source(),
                settings: settings(),
            },
            mirror: false,
            excluded_openings: Vec::new(),
        })
        .expect("sample ZIP should export an STL stencil");
        assert_eq!(stl.filename, "prod_main_stencil.stl");
        assert!(stl.stl.starts_with("solid stencil_print\n"));
        assert!(stl.stl.contains("facet normal"));
        assert!(stl.stl.ends_with("endsolid stencil_print\n"));
    }

    #[test]
    fn exports_without_selected_openings() {
        let paste = inline_source(
            "paste.gtp",
            "%FSLAX24Y24*%\n%MOMM*%\n%ADD10R,2X2*%\nD10*\nX50000Y50000D03*\nM02*",
        );
        let edge = inline_source(
            "Edge.Cuts.gm1",
            "%FSLAX24Y24*%\n%MOMM*%\n%ADD10C,0.05*%\nD10*\nX00000Y00000D02*\nX100000Y00000D01*\nX100000Y100000D01*\nX00000Y100000D01*\nX00000Y00000D01*\nM02*",
        );
        let input = PreviewRequest {
            paste,
            edge,
            settings: settings(),
        };
        let full = export_stencil_stl(ExportRequest {
            input,
            mirror: false,
            excluded_openings: Vec::new(),
        })
        .expect("inline Gerbers should export");
        let removed = export_stencil_stl(ExportRequest {
            input: PreviewRequest {
                paste: inline_source(
                    "paste.gtp",
                    "%FSLAX24Y24*%\n%MOMM*%\n%ADD10R,2X2*%\nD10*\nX50000Y50000D03*\nM02*",
                ),
                edge: inline_source(
                    "Edge.Cuts.gm1",
                    "%FSLAX24Y24*%\n%MOMM*%\n%ADD10C,0.05*%\nD10*\nX00000Y00000D02*\nX100000Y00000D01*\nX100000Y100000D01*\nX00000Y100000D01*\nX00000Y00000D01*\nM02*",
                ),
                settings: settings(),
            },
            mirror: false,
            excluded_openings: vec![0],
        })
        .expect("excluding an opening should still export");

        assert!(
            removed.stl.matches("facet normal").count() < full.stl.matches("facet normal").count()
        );
    }
}
