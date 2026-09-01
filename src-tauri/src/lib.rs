mod geometry;
mod gerber;
mod step;
mod stl;

use geometry::{GeometryOptions, StencilGeometry};
use gerber::{parse_source, LayerKind, LayerSource, LayerStats, ParsedLayer, PasteSide};
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
    shrink: f64,
    nozzle_diameter: f64,
    enable_slotify: bool,
    drop_unprintable_grids: bool,
}

impl StencilSettings {
    fn validate(self) -> Result<(), String> {
        let dimensions = [
            ("Clearance", self.clearance, true),
            ("Wall thickness", self.wall_thickness, false),
            ("Wall height", self.wall_height, false),
            ("Stencil thickness", self.stencil_thickness, false),
            ("Nozzle diameter", self.nozzle_diameter, false),
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
        if !(0.1..=0.8).contains(&self.nozzle_diameter) {
            return Err("Nozzle diameter must be between 0.1 and 0.8 mm.".into());
        }
        if !(0.1..=0.6).contains(&self.stencil_thickness) {
            return Err("Stencil thickness must be between 0.1 and 0.6 mm.".into());
        }
        if !(-0.2..=0.3).contains(&self.shrink) {
            return Err("Shrink must be between -0.2 and 0.3 mm.".into());
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
    #[serde(default)]
    paste_side: PasteSide,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportRequest {
    #[serde(flatten)]
    input: PreviewRequest,
    #[serde(default)]
    excluded_openings: Vec<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewResult {
    paste: LayerStats,
    edge: LayerStats,
    model: PreviewModel,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewModel {
    plate: Vec<gerber::Point>,
    openings: Vec<Vec<gerber::Point>>,
    selection_openings: Vec<Vec<gerber::Point>>,
    opening_sources: Vec<Vec<usize>>,
    inner_wall: Vec<gerber::Point>,
    outer_wall: Vec<gerber::Point>,
    warnings: Vec<String>,
}

struct StepExport {
    filename: String,
    step: String,
}

struct StlExport {
    filename: String,
    stl: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveResult {
    saved: bool,
    path: Option<String>,
}

fn parse_layers(request: &PreviewRequest) -> Result<(ParsedLayer, ParsedLayer), String> {
    request.settings.validate()?;
    Ok((
        parse_source(&request.paste, LayerKind::Paste, request.paste_side)?,
        parse_source(&request.edge, LayerKind::Edge, request.paste_side)?,
    ))
}

fn geometry_for(
    request: &PreviewRequest,
    excluded_openings: &[usize],
) -> Result<(ParsedLayer, ParsedLayer, StencilGeometry), String> {
    let (paste, edge) = parse_layers(request)?;
    let settings = request.settings;
    let options = GeometryOptions {
        clearance: settings.clearance,
        wall_thickness: settings.wall_thickness,
        shrink: settings.shrink,
        nozzle_diameter: settings.nozzle_diameter,
        enable_slotify: settings.enable_slotify,
        drop_unprintable_grids: settings.drop_unprintable_grids,
        mirror_back: matches!(request.paste_side, PasteSide::Back),
    };
    let geometry = if excluded_openings.is_empty() {
        StencilGeometry::from_layers(&paste, &edge, options)?
    } else {
        StencilGeometry::from_layers_excluding(&paste, &edge, options, excluded_openings)?
    };
    Ok((paste, edge, geometry))
}

#[tauri::command]
fn preview_stencil(request: PreviewRequest) -> Result<PreviewResult, String> {
    let (paste, edge, geometry) = geometry_for(&request, &[])?;
    Ok(PreviewResult {
        paste: paste.stats(),
        edge: edge.stats(),
        model: PreviewModel {
            plate: geometry.plate,
            openings: geometry.openings,
            selection_openings: geometry.selection_openings,
            opening_sources: geometry.opening_sources,
            inner_wall: geometry.inner_wall,
            outer_wall: geometry.outer_wall,
            warnings: geometry.warnings,
        },
    })
}

#[tauri::command]
fn save_stencil_step(request: ExportRequest) -> Result<SaveResult, String> {
    let export = build_step_export(&request)?;
    save_export(
        "Save STEP stencil",
        "STEP files",
        &["step", "stp"],
        &export.filename,
        &export.step,
        "step",
    )
}

fn save_export(
    title: &str,
    filter: &str,
    extensions: &[&str],
    filename: &str,
    content: &str,
    default_extension: &str,
) -> Result<SaveResult, String> {
    let Some(path) = rfd::FileDialog::new()
        .set_title(title)
        .add_filter(filter, extensions)
        .set_file_name(filename)
        .save_file()
    else {
        return Ok(SaveResult {
            saved: false,
            path: None,
        });
    };

    let path = if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extensions
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        }) {
        path
    } else {
        path.with_extension(default_extension)
    };
    std::fs::write(&path, content.as_bytes())
        .map_err(|error| format!("Could not save export to {}: {error}", path.display()))?;
    Ok(SaveResult {
        saved: true,
        path: Some(path.to_string_lossy().into_owned()),
    })
}

fn build_step_export(request: &ExportRequest) -> Result<StepExport, String> {
    let (_, _, geometry) = geometry_for(&request.input, &request.excluded_openings)?;
    let settings = request.input.settings;
    let step = write_step(&geometry, settings.wall_height, settings.stencil_thickness)?;
    let stem = export_stem(&request.input.paste.filename);
    Ok(StepExport {
        filename: format!("{stem}_stencil.step"),
        step,
    })
}

fn build_stl_export(request: &ExportRequest) -> Result<StlExport, String> {
    let (_, _, geometry) = geometry_for(&request.input, &request.excluded_openings)?;
    let settings = request.input.settings;
    let stl = write_stl(&geometry, settings.wall_height, settings.stencil_thickness)?;
    let stem = export_stem(&request.input.paste.filename);
    Ok(StlExport {
        filename: format!("{stem}_stencil.stl"),
        stl,
    })
}

#[tauri::command]
fn save_stencil_stl(request: ExportRequest) -> Result<SaveResult, String> {
    let export = build_stl_export(&request)?;
    save_export(
        "Save STL stencil",
        "STL files",
        &["stl"],
        &export.filename,
        &export.stl,
        "stl",
    )
}

fn export_stem(filename: &str) -> &str {
    filename.rsplit_once('.').map_or(filename, |(stem, _)| stem)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            preview_stencil,
            save_stencil_step,
            save_stencil_stl
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
            shrink: 0.0,
            nozzle_diameter: 0.2,
            enable_slotify: true,
            drop_unprintable_grids: true,
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
            paste_side: PasteSide::Front,
        })
        .expect("sample ZIP should build a preview");

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

        let back_preview = preview_stencil(PreviewRequest {
            paste: sample_zip_source(),
            edge: sample_zip_source(),
            settings: settings(),
            paste_side: PasteSide::Back,
        })
        .expect("sample ZIP should build a back-paste preview");
        assert!(!back_preview.model.openings.is_empty());

        let export = build_step_export(&ExportRequest {
            input: PreviewRequest {
                paste: sample_zip_source(),
                edge: sample_zip_source(),
                settings: settings(),
                paste_side: PasteSide::Front,
            },
            excluded_openings: Vec::new(),
        })
        .expect("sample ZIP should export a STEP stencil");

        assert_eq!(export.filename, "prod_main_stencil.step");
        assert!(export.step.starts_with("ISO-10303-21;"));
        assert!(export.step.contains("ADVANCED_BREP_SHAPE_REPRESENTATION"));
        assert_eq!(export.step.matches("MANIFOLD_SOLID_BREP").count(), 6);
        assert!(export.step.ends_with("END-ISO-10303-21;\n"));

        let back_step = build_step_export(&ExportRequest {
            input: PreviewRequest {
                paste: sample_zip_source(),
                edge: sample_zip_source(),
                settings: settings(),
                paste_side: PasteSide::Back,
            },
            excluded_openings: Vec::new(),
        })
        .expect("sample ZIP should export a back-paste STEP stencil");
        assert_eq!(back_step.step.matches("MANIFOLD_SOLID_BREP").count(), 6);

        let stl = build_stl_export(&ExportRequest {
            input: PreviewRequest {
                paste: sample_zip_source(),
                edge: sample_zip_source(),
                settings: settings(),
                paste_side: PasteSide::Front,
            },
            excluded_openings: Vec::new(),
        })
        .expect("sample ZIP should export an STL stencil");
        assert_eq!(stl.filename, "prod_main_stencil.stl");
        assert!(stl.stl.starts_with("solid stencil_print\n"));
        assert!(stl.stl.contains("facet normal"));
        assert!(stl.stl.ends_with("endsolid stencil_print\n"));

        let back_stl = build_stl_export(&ExportRequest {
            input: PreviewRequest {
                paste: sample_zip_source(),
                edge: sample_zip_source(),
                settings: settings(),
                paste_side: PasteSide::Back,
            },
            excluded_openings: Vec::new(),
        })
        .expect("sample ZIP should export a back-paste STL stencil");
        assert!(back_stl.stl.contains("facet normal"));
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
            paste_side: PasteSide::Front,
        };
        let full = build_stl_export(&ExportRequest {
            input,
            excluded_openings: Vec::new(),
        })
        .expect("inline Gerbers should export");
        let removed = build_stl_export(&ExportRequest {
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
                paste_side: PasteSide::Front,
            },
            excluded_openings: vec![0],
        })
        .expect("excluding an opening should still export");

        assert!(
            removed.stl.matches("facet normal").count() < full.stl.matches("facet normal").count()
        );
    }
}
