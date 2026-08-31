use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{BufReader, Cursor, Read},
};
use zip::ZipArchive;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LayerSource {
    pub data: String,
    pub filename: String,
    pub is_zip: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum LayerKind {
    Paste,
    Edge,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PasteSide {
    #[default]
    Front,
    Back,
}

impl LayerKind {
    fn label(self) -> &'static str {
        match self {
            Self::Paste => "paste",
            Self::Edge => "edge outline",
        }
    }

    fn archive_score(self, path: &str, paste_side: PasteSide) -> Option<i32> {
        let path = path.to_ascii_lowercase();
        let is_gerber = [".gbr", ".ger", ".gtp", ".gbp", ".gko", ".gm1", ".pho"]
            .iter()
            .any(|extension| path.ends_with(extension));
        if !is_gerber {
            return None;
        }

        let looks_like_edge = [
            "edge.cuts",
            "edge_cuts",
            "outline",
            "profile",
            ".gko",
            ".gm1",
        ]
        .iter()
        .any(|term| path.contains(term));
        let looks_like_paste = ["paste", "cream", "f_paste", "b_paste"]
            .iter()
            .any(|term| path.contains(term));
        match self {
            Self::Paste if looks_like_edge => return None,
            Self::Edge if looks_like_paste => return None,
            _ => {}
        }

        let score = match self {
            Self::Paste => {
                let preferred = ["paste", "cream"]
                    .iter()
                    .filter(|term| path.contains(*term))
                    .count() as i32
                    * 50;
                let front = ["top", "front", "f_paste", ".gtp"]
                    .iter()
                    .filter(|term| path.contains(*term))
                    .count() as i32
                    * 30;
                let back = ["bottom", "back", "b_paste", ".gbp"]
                    .iter()
                    .filter(|term| path.contains(*term))
                    .count() as i32
                    * 30;
                1 + preferred
                    + match paste_side {
                        PasteSide::Front => front - back,
                        PasteSide::Back => back - front,
                    }
            }
            Self::Edge => {
                let named = ["edge.cuts", "edge_cuts", "outline", "profile"]
                    .iter()
                    .filter(|term| path.contains(*term))
                    .count() as i32
                    * 60;
                let extension = i32::from(path.ends_with(".gko") || path.ends_with(".gm1")) * 25;
                1 + named + extension
            }
        };
        Some(score)
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug)]
pub(crate) enum MacroShape {
    Circle {
        diameter: f64,
        center: Point,
    },
    Polygon(Vec<Point>),
    Stroke {
        start: Point,
        end: Point,
        width: f64,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum Aperture {
    Circle(f64),
    Rectangle(f64, f64),
    Obround(f64, f64),
    Polygon(f64, usize, f64),
    Composite(Vec<MacroShape>),
}

#[derive(Clone, Debug)]
pub(crate) enum Primitive {
    Flash(Point, Aperture),
    Stroke(Point, Point, Aperture),
    Region(Vec<Point>),
}

#[derive(Clone, Debug)]
pub(crate) struct ParsedLayer {
    pub filename: String,
    pub units: String,
    pub primitives: Vec<Primitive>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Bounds {
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
}

impl Bounds {
    pub fn width(self) -> f64 {
        self.max_x - self.min_x
    }

    pub fn height(self) -> f64 {
        self.max_y - self.min_y
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LayerStats {
    filename: String,
    units: String,
    primitives: usize,
    flashes: usize,
    strokes: usize,
    regions: usize,
    width_mm: f64,
    height_mm: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
}

impl ParsedLayer {
    pub fn stats(&self) -> LayerStats {
        let size = bounds(&self.points()).unwrap_or(Bounds {
            min_x: 0.0,
            max_x: 0.0,
            min_y: 0.0,
            max_y: 0.0,
        });
        LayerStats {
            filename: self.filename.clone(),
            units: self.units.clone(),
            primitives: self.primitives.len(),
            flashes: self
                .primitives
                .iter()
                .filter(|item| matches!(item, Primitive::Flash(..)))
                .count(),
            strokes: self
                .primitives
                .iter()
                .filter(|item| matches!(item, Primitive::Stroke(..)))
                .count(),
            regions: self
                .primitives
                .iter()
                .filter(|item| matches!(item, Primitive::Region(..)))
                .count(),
            width_mm: size.width(),
            height_mm: size.height(),
            warning: self.warnings.first().cloned(),
        }
    }

    pub fn points(&self) -> Vec<Point> {
        self.primitives
            .iter()
            .flat_map(|primitive| match primitive {
                Primitive::Flash(point, _) => vec![*point],
                Primitive::Stroke(start, end, _) => vec![*start, *end],
                Primitive::Region(points) => points.clone(),
            })
            .collect()
    }
}

#[derive(Clone, Copy)]
struct CoordinateFormat {
    integer: usize,
    decimal: usize,
    factor: f64,
}

#[derive(Clone, Debug)]
enum MacroStatement {
    Circle {
        diameter: String,
        x: String,
        y: String,
    },
    Polygon {
        points: Vec<(String, String)>,
        rotation: String,
    },
    RegularPolygon {
        vertices: usize,
        diameter: String,
        center: (String, String),
        rotation: String,
    },
    Stroke {
        width: String,
        start: (String, String),
        end: (String, String),
        rotation: String,
    },
    Rectangle {
        width: String,
        height: String,
        center: (String, String),
        rotation: String,
    },
}

impl Default for CoordinateFormat {
    fn default() -> Self {
        Self {
            integer: 2,
            decimal: 4,
            factor: 1.0,
        }
    }
}

fn parse_aperture_macros(source: &str) -> HashMap<String, Vec<MacroStatement>> {
    let mut macros = HashMap::new();
    let mut current: Option<String> = None;

    for raw in source.split(['*', '%']) {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        if token.trim_matches('%').is_empty() {
            current = None;
            continue;
        }
        let without_percent = token.trim_matches('%').trim();
        let upper = without_percent.to_ascii_uppercase();
        if upper.starts_with("AM") {
            let name = without_percent[2..].trim().to_ascii_uppercase();
            if !name.is_empty() {
                current = Some(name.clone());
                macros.entry(name).or_insert_with(Vec::new);
            }
            continue;
        }
        let Some(name) = current.as_ref() else {
            continue;
        };
        if let Some(statement) = parse_macro_statement(without_percent) {
            macros.entry(name.clone()).or_default().push(statement);
        }
    }

    macros
}

fn parse_macro_statement(token: &str) -> Option<MacroStatement> {
    let fields = token.split(',').map(str::trim).collect::<Vec<_>>();
    let code = fields.first()?.parse::<u32>().ok()?;
    match code {
        1 if fields.len() >= 5 => Some(MacroStatement::Circle {
            diameter: fields[2].into(),
            x: fields[3].into(),
            y: fields[4].into(),
        }),
        4 => {
            let count = fields.get(2)?.parse::<usize>().ok()?;
            let coordinate_end = 3 + count * 2;
            if fields.len() <= coordinate_end {
                return None;
            }
            let points = (0..count)
                .map(|index| (fields[3 + index * 2].into(), fields[4 + index * 2].into()))
                .collect();
            Some(MacroStatement::Polygon {
                points,
                rotation: fields[coordinate_end].into(),
            })
        }
        5 if fields.len() >= 7 => Some(MacroStatement::RegularPolygon {
            vertices: fields[2].parse::<usize>().ok()?.max(3),
            diameter: fields[5].into(),
            center: (fields[3].into(), fields[4].into()),
            rotation: fields[6].into(),
        }),
        20 if fields.len() >= 8 => Some(MacroStatement::Stroke {
            width: fields[2].into(),
            start: (fields[3].into(), fields[4].into()),
            end: (fields[5].into(), fields[6].into()),
            rotation: fields[7].into(),
        }),
        21 | 22 if fields.len() >= 7 => Some(MacroStatement::Rectangle {
            width: fields[2].into(),
            height: fields[3].into(),
            center: if code == 21 {
                (fields[4].into(), fields[5].into())
            } else {
                (
                    format!("({})+({})/2", fields[4], fields[2]),
                    format!("({})+({})/2", fields[5], fields[3]),
                )
            },
            rotation: fields[6].into(),
        }),
        _ => None,
    }
}

fn evaluate_expression(expression: &str, parameters: &[f64]) -> Option<f64> {
    let mut parser = ExpressionParser {
        input: expression.chars().collect(),
        position: 0,
        parameters,
    };
    let value = parser.parse_sum()?;
    parser.skip_whitespace();
    (parser.position == parser.input.len()).then_some(value)
}

struct ExpressionParser<'a> {
    input: Vec<char>,
    position: usize,
    parameters: &'a [f64],
}

impl ExpressionParser<'_> {
    fn skip_whitespace(&mut self) {
        while self
            .input
            .get(self.position)
            .is_some_and(|character| character.is_whitespace())
        {
            self.position += 1;
        }
    }

    fn parse_sum(&mut self) -> Option<f64> {
        let mut value = self.parse_product()?;
        loop {
            self.skip_whitespace();
            let operator = self.input.get(self.position).copied();
            if !matches!(operator, Some('+') | Some('-')) {
                return Some(value);
            }
            self.position += 1;
            let right = self.parse_product()?;
            value = if operator == Some('+') {
                value + right
            } else {
                value - right
            };
        }
    }

    fn parse_product(&mut self) -> Option<f64> {
        let mut value = self.parse_primary()?;
        loop {
            self.skip_whitespace();
            let operator = self.input.get(self.position).copied();
            if !matches!(operator, Some('*') | Some('x') | Some('X') | Some('/')) {
                return Some(value);
            }
            self.position += 1;
            let right = self.parse_primary()?;
            value = if matches!(operator, Some('*') | Some('x') | Some('X')) {
                value * right
            } else {
                value / right
            };
        }
    }

    fn parse_primary(&mut self) -> Option<f64> {
        self.skip_whitespace();
        if matches!(self.input.get(self.position), Some('+') | Some('-')) {
            let negative = self.input[self.position] == '-';
            self.position += 1;
            let value = self.parse_primary()?;
            return Some(if negative { -value } else { value });
        }
        if self.input.get(self.position) == Some(&'(') {
            self.position += 1;
            let value = self.parse_sum()?;
            self.skip_whitespace();
            if self.input.get(self.position) != Some(&')') {
                return None;
            }
            self.position += 1;
            return Some(value);
        }
        if self.input.get(self.position) == Some(&'$') {
            self.position += 1;
            let start = self.position;
            while self
                .input
                .get(self.position)
                .is_some_and(|character| character.is_ascii_digit())
            {
                self.position += 1;
            }
            let index = self.input[start..self.position]
                .iter()
                .collect::<String>()
                .parse::<usize>()
                .ok()?;
            return parameters_get(self.parameters, index);
        }

        let start = self.position;
        let mut exponent = false;
        while let Some(character) = self.input.get(self.position).copied() {
            if character.is_ascii_digit() || character == '.' {
                self.position += 1;
            } else if matches!(character, 'e' | 'E') && !exponent {
                exponent = true;
                self.position += 1;
            } else if matches!(character, '+' | '-')
                && self.position > start
                && matches!(self.input.get(self.position - 1), Some('e' | 'E'))
            {
                self.position += 1;
            } else {
                break;
            }
        }
        (self.position > start).then(|| {
            self.input[start..self.position]
                .iter()
                .collect::<String>()
                .parse::<f64>()
                .ok()
        })?
    }
}

fn parameters_get(parameters: &[f64], index: usize) -> Option<f64> {
    index
        .checked_sub(1)
        .and_then(|index| parameters.get(index).copied())
}

fn resolve_macro(statements: &[MacroStatement], parameters: &[f64]) -> Vec<MacroShape> {
    statements
        .iter()
        .filter_map(|statement| match statement {
            MacroStatement::Circle { diameter, x, y } => Some(MacroShape::Circle {
                diameter: evaluate_expression(diameter, parameters)?.abs(),
                center: Point {
                    x: evaluate_expression(x, parameters)?,
                    y: evaluate_expression(y, parameters)?,
                },
            }),
            MacroStatement::Polygon { points, rotation } => {
                let rotation = evaluate_expression(rotation, parameters)?.to_radians();
                let points = points
                    .iter()
                    .map(|(x, y)| {
                        Some(rotate_point(
                            Point {
                                x: evaluate_expression(x, parameters)?,
                                y: evaluate_expression(y, parameters)?,
                            },
                            rotation,
                        ))
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(MacroShape::Polygon(points))
            }
            MacroStatement::RegularPolygon {
                vertices,
                diameter,
                center,
                rotation,
            } => {
                let diameter = evaluate_expression(diameter, parameters)?.abs();
                let center = Point {
                    x: evaluate_expression(&center.0, parameters)?,
                    y: evaluate_expression(&center.1, parameters)?,
                };
                let rotation = evaluate_expression(rotation, parameters)?.to_radians();
                Some(MacroShape::Polygon(
                    (0..*vertices)
                        .map(|index| {
                            let angle =
                                rotation + std::f64::consts::TAU * index as f64 / *vertices as f64;
                            Point {
                                x: center.x + diameter / 2.0 * angle.cos(),
                                y: center.y + diameter / 2.0 * angle.sin(),
                            }
                        })
                        .collect(),
                ))
            }
            MacroStatement::Stroke {
                width,
                start,
                end,
                rotation,
            } => {
                let rotation = evaluate_expression(rotation, parameters)?.to_radians();
                Some(MacroShape::Stroke {
                    start: rotate_point(
                        Point {
                            x: evaluate_expression(&start.0, parameters)?,
                            y: evaluate_expression(&start.1, parameters)?,
                        },
                        rotation,
                    ),
                    end: rotate_point(
                        Point {
                            x: evaluate_expression(&end.0, parameters)?,
                            y: evaluate_expression(&end.1, parameters)?,
                        },
                        rotation,
                    ),
                    width: evaluate_expression(width, parameters)?.abs(),
                })
            }
            MacroStatement::Rectangle {
                width,
                height,
                center,
                rotation,
            } => {
                let width = evaluate_expression(width, parameters)?.abs();
                let height = evaluate_expression(height, parameters)?.abs();
                let center = Point {
                    x: evaluate_expression(&center.0, parameters)?,
                    y: evaluate_expression(&center.1, parameters)?,
                };
                let rotation = evaluate_expression(rotation, parameters)?.to_radians();
                let corners = [
                    Point {
                        x: -width / 2.0,
                        y: -height / 2.0,
                    },
                    Point {
                        x: width / 2.0,
                        y: -height / 2.0,
                    },
                    Point {
                        x: width / 2.0,
                        y: height / 2.0,
                    },
                    Point {
                        x: -width / 2.0,
                        y: height / 2.0,
                    },
                ];
                Some(MacroShape::Polygon(
                    corners
                        .into_iter()
                        .map(|point| {
                            let rotated = rotate_point(point, rotation);
                            Point {
                                x: center.x + rotated.x,
                                y: center.y + rotated.y,
                            }
                        })
                        .collect(),
                ))
            }
        })
        .collect()
}

fn rotate_point(point: Point, angle: f64) -> Point {
    Point {
        x: point.x * angle.cos() - point.y * angle.sin(),
        y: point.x * angle.sin() + point.y * angle.cos(),
    }
}

pub(crate) fn parse_source(
    source: &LayerSource,
    kind: LayerKind,
    paste_side: PasteSide,
) -> Result<ParsedLayer, String> {
    let candidates = resolve_layer_data(source, kind, paste_side)?;
    let mut errors = Vec::new();
    for (data, filename) in candidates {
        match parse_layer(&data, &filename) {
            Ok(layer) => return Ok(layer),
            Err(error) => errors.push(format!("{filename}: {error}")),
        }
    }
    Err(format!(
        "No drawable {} Gerber geometry found in {}. Tried: {}",
        kind.label(),
        source.filename,
        errors.join("; ")
    ))
}

fn parse_layer(source: &str, filename: &str) -> Result<ParsedLayer, String> {
    match parse_layer_with_library(source, filename) {
        Ok(layer) => Ok(layer),
        Err(library_error) => {
            let mut layer = parse_layer_legacy(source, filename)?;
            layer.warnings.insert(
                0,
                format!("Compatibility parser used for {filename}: {library_error}"),
            );
            Ok(layer)
        }
    }
}

fn parse_layer_with_library(source: &str, filename: &str) -> Result<ParsedLayer, String> {
    use gerber_parser::gerber_types::{
        Aperture as ParsedAperture, Command, DCode, ExtendedCode, FunctionCode, GCode, Operation,
    };

    let document = gerber_parser::parse(BufReader::new(source.as_bytes()))
        .map_err(|(_, error)| format!("off-the-shelf parser rejected the file ({error:?})"))?;
    if let Some(error) = document.errors().first() {
        return Err(format!("off-the-shelf parser reported {error:?}"));
    }
    if document
        .apertures
        .values()
        .any(|aperture| matches!(aperture, ParsedAperture::Macro(..)))
    {
        return Err("aperture macros use the compatibility resolver".into());
    }

    let factor = match document.units {
        Some(gerber_parser::gerber_types::Unit::Inches) => 25.4,
        _ => 1.0,
    };
    let units = if factor == 1.0 { "mm" } else { "inch → mm" };
    let mut current_aperture = 10;
    let mut current = Point { x: 0.0, y: 0.0 };
    let mut region_points = Vec::new();
    let mut primitives = Vec::new();
    let mut warnings = Vec::new();
    let mut in_region = false;
    let mut incremental = document.format_specification.is_some_and(|format| {
        matches!(
            format.coordinate_mode,
            gerber_parser::gerber_types::CoordinateMode::Incremental
        )
    });
    let mut circular_interpolation = false;
    let mut clockwise_interpolation = false;

    for command in document.commands() {
        match command {
            Command::ExtendedCode(ExtendedCode::Unit(_)) => {}
            Command::ExtendedCode(ExtendedCode::ApertureDefinition(_)) => {}
            Command::FunctionCode(FunctionCode::DCode(DCode::SelectAperture(code))) => {
                current_aperture = *code;
            }
            Command::FunctionCode(FunctionCode::GCode(GCode::RegionMode(enabled))) => {
                if *enabled {
                    in_region = true;
                    region_points.clear();
                } else {
                    if region_points.len() >= 3 {
                        primitives.push(Primitive::Region(std::mem::take(&mut region_points)));
                    }
                    in_region = false;
                }
            }
            Command::FunctionCode(FunctionCode::GCode(GCode::CoordinateMode(mode))) => {
                incremental = matches!(
                    mode,
                    gerber_parser::gerber_types::CoordinateMode::Incremental
                );
            }
            Command::FunctionCode(FunctionCode::GCode(GCode::InterpolationMode(mode))) => {
                circular_interpolation = matches!(
                    mode,
                    gerber_parser::gerber_types::InterpolationMode::ClockwiseCircular
                        | gerber_parser::gerber_types::InterpolationMode::CounterclockwiseCircular
                );
                clockwise_interpolation = matches!(
                    mode,
                    gerber_parser::gerber_types::InterpolationMode::ClockwiseCircular
                );
            }
            Command::FunctionCode(FunctionCode::DCode(DCode::Operation(operation))) => {
                let previous = current;
                match operation {
                    Operation::Move(coords) => {
                        update_position(&mut current, coords, factor, incremental);
                        if in_region && region_points.is_empty() {
                            region_points.push(current);
                        }
                    }
                    Operation::Interpolate(coords, offset) => {
                        update_position(&mut current, coords, factor, incremental);
                        if (circular_interpolation || offset.is_some()) && warnings.is_empty() {
                            warnings.push(
                                "Circular interpolation is approximated as line segments in this build."
                                    .into(),
                            );
                        }
                        let path = if circular_interpolation {
                            offset
                                .as_ref()
                                .and_then(|offset| {
                                    circular_path(
                                        previous,
                                        current,
                                        offset,
                                        factor,
                                        clockwise_interpolation,
                                    )
                                })
                                .unwrap_or_else(|| vec![previous, current])
                        } else {
                            vec![previous, current]
                        };
                        if in_region {
                            region_points.extend(path.into_iter().skip(1));
                        } else if let Some(aperture) = document
                            .apertures
                            .get(&current_aperture)
                            .and_then(|aperture| standard_aperture(aperture, factor))
                        {
                            for points in path.windows(2) {
                                primitives.push(Primitive::Stroke(
                                    points[0],
                                    points[1],
                                    aperture.clone(),
                                ));
                            }
                        }
                    }
                    Operation::Flash(coords) => {
                        update_position(&mut current, coords, factor, incremental);
                        if let Some(aperture) = document
                            .apertures
                            .get(&current_aperture)
                            .and_then(|aperture| standard_aperture(aperture, factor))
                        {
                            primitives.push(Primitive::Flash(current, aperture));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if primitives.is_empty() {
        return Err(format!("No drawable Gerber geometry found in {filename}."));
    }
    Ok(ParsedLayer {
        filename: filename.into(),
        units: units.into(),
        primitives,
        warnings,
    })
}

fn update_position(
    current: &mut Point,
    coordinates: &Option<gerber_parser::gerber_types::Coordinates>,
    factor: f64,
    incremental: bool,
) {
    let Some(coordinates) = coordinates else {
        return;
    };
    if let Some(x) = coordinates.x {
        let value: f64 = x.into();
        if incremental {
            current.x += value * factor;
        } else {
            current.x = value * factor;
        }
    }
    if let Some(y) = coordinates.y {
        let value: f64 = y.into();
        if incremental {
            current.y += value * factor;
        } else {
            current.y = value * factor;
        }
    }
}

fn circular_path(
    start: Point,
    end: Point,
    offset: &gerber_parser::gerber_types::CoordinateOffset,
    factor: f64,
    clockwise: bool,
) -> Option<Vec<Point>> {
    let offset_x: f64 = offset.x?.into();
    let offset_y: f64 = offset.y?.into();
    let center = Point {
        x: start.x + offset_x * factor,
        y: start.y + offset_y * factor,
    };
    let radius = (start.x - center.x).hypot(start.y - center.y);
    if radius <= f64::EPSILON {
        return None;
    }

    let start_angle = (start.y - center.y).atan2(start.x - center.x);
    let end_angle = (end.y - center.y).atan2(end.x - center.x);
    let mut sweep = end_angle - start_angle;
    if clockwise {
        while sweep >= 0.0 {
            sweep -= std::f64::consts::TAU;
        }
    } else {
        while sweep <= 0.0 {
            sweep += std::f64::consts::TAU;
        }
    }
    if (start.x - end.x).hypot(start.y - end.y) <= 1e-9 {
        sweep = if clockwise {
            -std::f64::consts::TAU
        } else {
            std::f64::consts::TAU
        };
    }
    let segments = (sweep.abs() / (std::f64::consts::PI / 16.0))
        .ceil()
        .clamp(8.0, 512.0) as usize;
    let mut points = (0..=segments)
        .map(|index| {
            let angle = start_angle + sweep * index as f64 / segments as f64;
            Point {
                x: center.x + radius * angle.cos(),
                y: center.y + radius * angle.sin(),
            }
        })
        .collect::<Vec<_>>();
    if let Some(last) = points.last_mut() {
        *last = end;
    }
    Some(points)
}

fn standard_aperture(
    aperture: &gerber_parser::gerber_types::Aperture,
    factor: f64,
) -> Option<Aperture> {
    use gerber_parser::gerber_types::Aperture as ParsedAperture;

    match aperture {
        ParsedAperture::Circle(circle) => Some(Aperture::Circle(circle.diameter * factor)),
        ParsedAperture::Rectangle(rectangle) => Some(Aperture::Rectangle(
            rectangle.x * factor,
            rectangle.y * factor,
        )),
        ParsedAperture::Obround(rectangle) => Some(Aperture::Obround(
            rectangle.x * factor,
            rectangle.y * factor,
        )),
        ParsedAperture::Polygon(polygon) => Some(Aperture::Polygon(
            polygon.diameter * factor,
            polygon.vertices as usize,
            polygon.rotation.unwrap_or_default(),
        )),
        ParsedAperture::Macro(..) => None,
    }
}

fn resolve_layer_data(
    source: &LayerSource,
    kind: LayerKind,
    paste_side: PasteSide,
) -> Result<Vec<(String, String)>, String> {
    if !source.is_zip {
        return Ok(vec![(source.data.clone(), source.filename.clone())]);
    }

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&source.data)
        .map_err(|error| format!("Could not decode {}: {error}", source.filename))?;
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|error| {
        format!(
            "Could not open {} as a ZIP archive: {error}",
            source.filename
        )
    })?;
    let mut candidates = Vec::new();

    for index in 0..archive.len() {
        let Ok(entry) = archive.by_index(index) else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        let path = entry.name().to_owned();
        let Some(score) = kind.archive_score(&path, paste_side) else {
            continue;
        };
        candidates.push((score, index, path));
    }

    if candidates.is_empty() {
        return Err(format!(
            "No suitable {} Gerber was found inside {}.",
            kind.label(),
            source.filename
        ));
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0));
    candidates
        .into_iter()
        .map(|(_, index, member)| {
            let mut entry = archive.by_index(index).map_err(|error| {
                format!("Could not read {member} from {}: {error}", source.filename)
            })?;
            let mut text = String::new();
            entry.read_to_string(&mut text).map_err(|error| {
                format!("Could not read {member} from {}: {error}", source.filename)
            })?;
            Ok((text, format!("{} → {member}", source.filename)))
        })
        .collect()
}

fn parse_layer_legacy(source: &str, filename: &str) -> Result<ParsedLayer, String> {
    let mut format = CoordinateFormat::default();
    let macro_definitions = parse_aperture_macros(source);
    let mut apertures = HashMap::new();
    let mut current_aperture = 10;
    let mut current = Point { x: 0.0, y: 0.0 };
    let mut region_points = Vec::new();
    let mut primitives = Vec::new();
    let mut warnings = Vec::new();
    let mut in_region = false;

    for raw in source.split(['*', '%']) {
        let token = raw.trim().to_ascii_uppercase();
        if token.is_empty() || token.starts_with("G04") || token.starts_with("M02") {
            continue;
        }
        if token.starts_with("FS") {
            update_coordinate_format(&token, &mut format);
            continue;
        }
        if token.starts_with("MOMM") {
            format.factor = 1.0;
            continue;
        }
        if token.starts_with("MOIN") {
            format.factor = 25.4;
            continue;
        }
        if ["LP", "LM", "LR", "LS"]
            .iter()
            .any(|prefix| token.starts_with(prefix))
        {
            continue;
        }
        if token.starts_with("ADD") {
            let (code, aperture) = parse_aperture(&token, format, &macro_definitions);
            apertures.insert(code, aperture);
            continue;
        }
        if token == "G36" {
            in_region = true;
            region_points.clear();
            continue;
        }
        if token == "G37" {
            if region_points.len() >= 3 {
                primitives.push(Primitive::Region(std::mem::take(&mut region_points)));
            }
            in_region = false;
            continue;
        }
        if !token.contains('D') {
            continue;
        }

        let previous = current;
        if let Some(x) = axis_value(&token, 'X', format) {
            current.x = x;
        }
        if let Some(y) = axis_value(&token, 'Y', format) {
            current.y = y;
        }
        if warnings.is_empty() && (token.contains("G02") || token.contains("G03")) {
            warnings.push(
                "Circular interpolation is approximated as line segments in this build.".into(),
            );
        }

        match d_code(&token) {
            Some(1) if in_region => region_points.push(current),
            Some(1) => {
                if let Some(aperture) = apertures.get(&current_aperture).cloned() {
                    primitives.push(Primitive::Stroke(previous, current, aperture));
                }
            }
            Some(2) if in_region && region_points.is_empty() => region_points.push(current),
            Some(3) => {
                if let Some(aperture) = apertures.get(&current_aperture).cloned() {
                    primitives.push(Primitive::Flash(current, aperture));
                }
            }
            Some(code) if code >= 10 => current_aperture = code,
            _ => {}
        }
    }

    if primitives.is_empty() {
        return Err(format!("No drawable Gerber geometry found in {filename}."));
    }
    Ok(ParsedLayer {
        filename: filename.into(),
        units: if format.factor == 1.0 {
            "mm".into()
        } else {
            "inch → mm".into()
        },
        primitives,
        warnings,
    })
}

fn update_coordinate_format(token: &str, format: &mut CoordinateFormat) {
    let Some(x) = token.find('X') else {
        return;
    };
    let digits = token[x + 1..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    if digits.len() >= 2 {
        format.integer = digits[..digits.len() - 1].parse().unwrap_or(2);
        format.decimal = digits[digits.len() - 1..].parse().unwrap_or(4);
    }
}

fn parse_aperture(
    token: &str,
    format: CoordinateFormat,
    macro_definitions: &HashMap<String, Vec<MacroStatement>>,
) -> (u32, Aperture) {
    let body = &token[3..];
    let digits = body
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    let code = digits.parse().unwrap_or(10);
    let shape = &body[digits.len()..];
    let (kind, dimensions) = shape.split_once(',').unwrap_or((shape, "1"));
    let values = dimensions
        .split(['X', 'x'])
        .filter_map(|value| value.parse::<f64>().ok())
        .collect::<Vec<_>>();
    let value =
        |index: usize, default: f64| values.get(index).copied().unwrap_or(default) * format.factor;
    let kind_upper = kind.to_ascii_uppercase();
    let aperture = match kind_upper.as_str() {
        "R" => Aperture::Rectangle(value(0, 1.0), value(1, 1.0)),
        "O" => Aperture::Obround(value(0, 1.0), value(1, 1.0)),
        "P" => Aperture::Polygon(
            value(0, 1.0),
            values.get(1).copied().unwrap_or(6.0).round().max(3.0) as usize,
            values.get(2).copied().unwrap_or_default(),
        ),
        "C" => Aperture::Circle(value(0, 1.0)),
        _ => Aperture::Composite(
            macro_definitions
                .get(&kind_upper)
                .map(|definition| {
                    resolve_macro(definition, &values)
                        .into_iter()
                        .map(|shape| scale_macro_shape(shape, format.factor))
                        .collect()
                })
                .unwrap_or_default(),
        ),
    };
    (code, aperture)
}

fn scale_macro_shape(shape: MacroShape, factor: f64) -> MacroShape {
    match shape {
        MacroShape::Circle { diameter, center } => MacroShape::Circle {
            diameter: diameter * factor,
            center: Point {
                x: center.x * factor,
                y: center.y * factor,
            },
        },
        MacroShape::Polygon(points) => MacroShape::Polygon(
            points
                .into_iter()
                .map(|point| Point {
                    x: point.x * factor,
                    y: point.y * factor,
                })
                .collect(),
        ),
        MacroShape::Stroke { start, end, width } => MacroShape::Stroke {
            start: Point {
                x: start.x * factor,
                y: start.y * factor,
            },
            end: Point {
                x: end.x * factor,
                y: end.y * factor,
            },
            width: width * factor,
        },
    }
}

fn d_code(token: &str) -> Option<u32> {
    let start = token.rfind('D')? + 1;
    token[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

fn axis_value(token: &str, axis: char, format: CoordinateFormat) -> Option<f64> {
    let rest = &token[token.find(axis)? + 1..];
    let end = rest
        .find(|character: char| character.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let raw = &rest[..end];
    if raw.is_empty() {
        return None;
    }
    if raw.contains('.') {
        return raw.parse::<f64>().ok().map(|value| value * format.factor);
    }

    let sign = if raw.starts_with('-') { -1.0 } else { 1.0 };
    let unsigned = raw.trim_start_matches(['+', '-']);
    let padded = format!(
        "{:0>width$}",
        unsigned,
        width = format.integer + format.decimal
    );
    let split = padded.len().saturating_sub(format.decimal);
    let value = format!("{}.{}", &padded[..split], &padded[split..])
        .parse::<f64>()
        .ok()?;
    Some(sign * value * format.factor)
}

pub(crate) fn bounds(points: &[Point]) -> Option<Bounds> {
    let first = points.first()?;
    Some(points[1..].iter().fold(
        Bounds {
            min_x: first.x,
            max_x: first.x,
            min_y: first.y,
            max_y: first.y,
        },
        |mut bounds, point| {
            bounds.min_x = bounds.min_x.min(point.x);
            bounds.max_x = bounds.max_x.max(point.x);
            bounds.min_y = bounds.min_y.min(point.y);
            bounds.max_y = bounds.max_y.max(point.y);
            bounds
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_GERBER: &str =
        "%FSLAX24Y24*%\n%MOMM*%\n%ADD10C,1.0*%\nD10*\nX10000Y20000D03*\nM02*";

    #[test]
    fn parses_a_flash_and_coordinates() {
        let layer = parse_layer(SIMPLE_GERBER, "paste.gtp").unwrap();

        assert_eq!(layer.primitives.len(), 1);
        let Primitive::Flash(point, Aperture::Circle(diameter)) = &layer.primitives[0] else {
            panic!("expected a circular flash");
        };
        assert_eq!((point.x, point.y, *diameter), (1.0, 2.0, 1.0));
        assert!(layer.warnings.is_empty());
    }

    #[test]
    fn resolves_a_legacy_aperture_macro_when_the_library_path_defers() {
        let source = "%FSLAX24Y24*%\n%MOMM*%\n%AMRECT*\n21,1,$1,$2,0,0,0*\n%\n%ADD10RECT,2X1*%\nD10*\nX10000Y20000D03*\nM02*";
        let layer = parse_layer(source, "paste.gbr").unwrap();

        assert!(layer
            .warnings
            .first()
            .is_some_and(|warning| warning.contains("Compatibility parser used")));
        assert!(matches!(
            layer.primitives.first(),
            Some(Primitive::Flash(_, Aperture::Composite(_)))
        ));
    }

    #[test]
    fn rejects_empty_geometry() {
        let error = parse_layer("%FSLAX24Y24*%MOMM*%M02*", "empty.gbr").unwrap_err();
        assert!(error.contains("No drawable Gerber geometry"));
    }

    #[test]
    fn scores_expected_archive_members() {
        assert!(
            LayerKind::Paste.archive_score("board-F_Paste.gtp", PasteSide::Front)
                > LayerKind::Paste.archive_score("board-B_Paste.gbr", PasteSide::Front)
        );
        assert!(
            LayerKind::Edge.archive_score("board-Edge_Cuts.gm1", PasteSide::Front)
                > LayerKind::Edge.archive_score("copper.gbr", PasteSide::Front)
        );
        assert!(LayerKind::Paste
            .archive_score("board-Edge_Cuts.gm1", PasteSide::Front)
            .is_none());
        assert!(LayerKind::Edge
            .archive_score("board-F_Paste.gtp", PasteSide::Front)
            .is_none());
    }

    #[test]
    fn imports_the_supplied_nested_gerber_zip() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("gerber-sample")
            .join("prod_main.zip");
        let bytes = std::fs::read(&path).unwrap_or_else(|error| {
            panic!("could not read {}: {error}", path.display());
        });
        let source = LayerSource {
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
            filename: "prod_main.zip".into(),
            is_zip: true,
        };

        let paste = parse_source(&source, LayerKind::Paste, PasteSide::Front).unwrap();
        let back_paste = parse_source(&source, LayerKind::Paste, PasteSide::Back).unwrap();
        let edge = parse_source(&source, LayerKind::Edge, PasteSide::Front).unwrap();

        assert!(paste.filename.ends_with("main_i2c-F_Paste.gtp"));
        assert!(back_paste.filename.ends_with("main_i2c-B_Paste.gbp"));
        assert!(edge.filename.ends_with("main_i2c-Edge_Cuts.gm1"));
        assert!(!paste.primitives.is_empty());
        assert!(!edge.primitives.is_empty());
    }
}
