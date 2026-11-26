// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The presentation as the agent reads it: its slides in order, each shape with its id, place (in points), text and,
//! for a table, its cells. The ids are the editor's: they stay while the document is open, and are what [`Op`](crate::presentation::Op)s
//! name.

use ibis_doc_model::drawing::Fill;
use ibis_doc_model::geometry::EMU_PER_POINT;
use ibis_doc_model::model::{Presentation, Shape, ShapeKind, Slide};
use serde::Serialize;

use super::text::{ParagraphText, read_body as paragraphs};

#[derive(Debug, Serialize)]
pub struct Inspection {
    pub name: String,
    /// Whether there are edits not saved yet.
    pub dirty: bool,
    pub slide_size: Size,
    /// The layouts a new slide can follow, by name (`add_slide`'s `layout`).
    pub layouts: Vec<String>,
    pub slides: Vec<SlideInfo>,
}

#[derive(Debug, Serialize)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Serialize)]
pub struct SlideInfo {
    pub id: u64,
    /// 1-based, in the slide order.
    pub number: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// The speaker notes' paragraphs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    pub shapes: Vec<ShapeInfo>,
}

#[derive(Debug, Serialize)]
pub struct ShapeInfo {
    pub id: u64,
    /// `text_box`, `shape`, `picture`, `line`, `table`, `chart` or `other`.
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Which placeholder of the layout it is (`title`, `body`, `subTitle`, `dt`, `ftr`, `sldNum`…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// The preset outline of a shape (`rect`, `roundRect`, `ellipse`…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry: Option<String>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// Its paragraphs, in the shape [`Op`](crate::presentation::Op)s take text in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Vec<ParagraphText>>,
    /// The size of the text's first character, in points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    /// A table's cells' paragraphs, by row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cells: Option<Vec<Vec<Vec<ParagraphText>>>>,
}

pub fn inspect(presentation: &Presentation, name: &str, dirty: bool) -> Inspection {
    Inspection {
        name: name.to_string(),
        dirty,
        slide_size: Size { w: pt(presentation.slide_size.width), h: pt(presentation.slide_size.height) },
        layouts: presentation.layouts.iter().map(|l| l.name.clone()).collect(),
        slides: presentation.slides.iter().enumerate().map(|(i, slide)| slide_info(presentation, slide, i + 1)).collect(),
    }
}

fn slide_info(presentation: &Presentation, slide: &Slide, number: usize) -> SlideInfo {
    SlideInfo {
        id: slide.id.0,
        number,
        layout: slide.layout.and_then(|l| presentation.layout(l)).map(|l| l.name.clone()),
        hidden: slide.hidden,
        background: color_of(&slide.background),
        notes: if slide.notes.is_empty() { Vec::new() } else { slide.notes.split('\n').map(str::to_string).collect() },
        shapes: slide.shapes.iter().filter(|s| !slide.is_attached(s.id)).map(|s| shape_info(slide, s)).collect(),
    }
}

fn shape_info(slide: &Slide, shape: &Shape) -> ShapeInfo {
    let (kind, geometry, fill) = match &shape.kind {
        ShapeKind::TextBox(_) => ("text_box", None, None),
        ShapeKind::Auto(auto) => ("shape", auto.geometry.preset_name().map(str::to_string), color_of(&auto.fill)),
        ShapeKind::Picture(_) => ("picture", None, None),
        ShapeKind::Line(_) => ("line", None, None),
        ShapeKind::Table(_) => ("table", None, None),
        ShapeKind::Chart(_) => ("chart", None, None),
        _ => ("other", None, None),
    };
    let body = shape.text_body();
    let cells = match &shape.kind {
        ShapeKind::Table(table) => Some(table.rows.iter().map(|row| row.cells.iter().map(|c| paragraphs(&c.text)).collect()).collect()),
        _ => None,
    };
    ShapeInfo {
        id: shape.id.0,
        kind,
        name: slide.shape_names.get(&shape.id).cloned(),
        placeholder: slide.placeholders.get(&shape.id).map(|p| p.placeholder.kind.clone().unwrap_or_else(|| "body".into())),
        geometry,
        x: pt(shape.frame.x),
        y: pt(shape.frame.y),
        w: pt(shape.frame.width),
        h: pt(shape.frame.height),
        fill,
        text: body.map(paragraphs),
        font_size: body.and_then(|b| {
            let first = b.paragraphs.first()?;
            let size = first.runs.first().map_or(first.end_style.font_size, |r| r.style.font_size).unwrap_or(b.style.font_size);
            Some(size as f64 / 100.0)
        }),
        cells,
    }
}

/// EMU in points, to a tenth.
pub(crate) fn pt(emu: i64) -> f64 {
    (emu as f64 / EMU_PER_POINT as f64 * 10.0).round() / 10.0
}

fn color_of(fill: &Fill) -> Option<String> {
    match fill {
        Fill::Solid(color) => Some(format!("#{:06X}", color.rgb.0)),
        _ => None,
    }
}
