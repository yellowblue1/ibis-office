// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The operations an agent edits a presentation with, and how they become the editor's commands.
//!
//! Places and sizes are in points, colours `#RRGGBB`, and slides and shapes are named by the ids [`crate::inspect`]
//! gives, or, for one made earlier in the same list, by the name its `as` gave it.

use std::collections::HashMap;

use ibis_doc_model::command::Command;
use ibis_doc_model::drawing::{Color, Fill, Rgb, Stroke};
use ibis_doc_model::geometry::{EMU_PER_POINT, EmuRect};
use ibis_doc_model::model::{AutoShape, Presentation, Shape, ShapeId, ShapeKind, Slide, SlideId};
use ibis_doc_model::text::{CharacterFormat, ListKind, Paragraph, Run, TextAlign, TextBody, TextStyle, VerticalAnchor};
use ibis_doc_model::{Document, Table};
use ibis_pptx::Source;
use serde::{Deserialize, Serialize};

/// A slide or a shape: its id, or the `as` name of one made earlier in the same list; a shape also as the placeholder
/// of its slide it is (`{"placeholder": "title"}`: `title`, `body`, `subTitle`…), for the slide just added.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Ref {
    Id(u64),
    Name(String),
    Placeholder { placeholder: String },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A character format: what is given is set, the rest is left as it is.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextFormat {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    /// In points.
    pub size: Option<f64>,
    pub color: Option<String>,
    pub font: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum List {
    None,
    Bullet,
    Numbered,
}

/// A paragraph format: what is given is set, the rest is left as it is.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParagraphFormat {
    pub align: Option<Align>,
    pub list: Option<List>,
    /// The outline level, 0 to 8.
    pub level: Option<u8>,
    /// In points.
    pub space_before: Option<f64>,
    pub space_after: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// A new slide at `index` (0-based; the end when not given) following layout `layout` (by name; the layout of the
    /// slide before it when not given), with the layout's placeholders empty.
    AddSlide {
        index: Option<usize>,
        layout: Option<String>,
        #[serde(rename = "as")]
        name: Option<String>,
    },
    DeleteSlide { slide: Ref },
    /// Move a slide to `index` (0-based, its place after the move).
    MoveSlide { slide: Ref, index: usize },
    /// A copy of a slide at `index` (right after it when not given).
    DuplicateSlide {
        slide: Ref,
        index: Option<usize>,
        #[serde(rename = "as")]
        name: Option<String>,
    },
    SetNotes { slide: Ref, text: String },
    SetBackground { slide: Ref, color: String },
    AddTextBox {
        slide: Ref,
        frame: Rect,
        text: String,
        format: Option<TextFormat>,
        #[serde(rename = "as")]
        name: Option<String>,
    },
    /// A shape of preset `geometry` (`rect`, `roundRect`, `ellipse`, `triangle`, `rightArrow`…).
    AddShape {
        slide: Ref,
        geometry: String,
        frame: Rect,
        fill: Option<String>,
        line: Option<String>,
        text: Option<String>,
        #[serde(rename = "as")]
        name: Option<String>,
    },
    /// A table of `rows` (each a list of the cells' text); centred on the slide when `frame` is not given.
    AddTable {
        slide: Ref,
        frame: Option<Rect>,
        rows: Vec<Vec<String>>,
        #[serde(rename = "as")]
        name: Option<String>,
    },
    DeleteShape { slide: Ref, shape: Ref },
    /// Move or resize a shape: what is given is set.
    SetFrame { slide: Ref, shape: Ref, x: Option<f64>, y: Option<f64>, w: Option<f64>, h: Option<f64> },
    /// Replace a shape's text (paragraphs separated by `\n`), each paragraph keeping the format of the one it replaces.
    SetText { slide: Ref, shape: Ref, text: String },
    /// Replace every `find` with `replace`, in the shapes and table cells of `slide` (all slides when not given) or
    /// of `shape` only, keeping the format of the text replaced.
    ReplaceText { find: String, replace: String, slide: Option<Ref>, shape: Option<Ref> },
    /// Give a shape's text (or each `match` in it; the text of cell `cell` [row, column] of a table) a character format.
    FormatText {
        slide: Ref,
        shape: Ref,
        #[serde(rename = "match")]
        matching: Option<String>,
        cell: Option<[usize; 2]>,
        format: TextFormat,
    },
    /// Give a shape's paragraphs (those of `paragraphs`, 0-based, when given) a paragraph format.
    FormatParagraphs { slide: Ref, shape: Ref, paragraphs: Option<Vec<usize>>, cell: Option<[usize; 2]>, format: ParagraphFormat },
    /// Fill a shape with `color`, or with nothing (`null`).
    SetFill { slide: Ref, shape: Ref, color: Option<String> },
    /// A shape's outline: `color` and `width` (points; 0.75 when not given), or none (`color` `null`).
    SetLine { slide: Ref, shape: Ref, color: Option<String>, width: Option<f64> },
    /// Replace the text of the cell at `row`, `column` (0-based) of a table.
    SetCell { slide: Ref, shape: Ref, row: usize, column: usize, text: String },
    /// A row inserted into a table at `at` (0-based; the end when not given), formatted as the table's style does.
    InsertTableRow { slide: Ref, shape: Ref, at: Option<usize> },
    DeleteTableRow { slide: Ref, shape: Ref, at: usize },
}

/// What an operation made, for the agent to name it next.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Created {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slide: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<u64>,
    /// How many places `replace_text` replaced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaced: Option<usize>,
}

/// Why a list of operations was refused (nothing of it was applied). `index` is the operation's, when one failed.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OpError {
    pub index: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.index {
            Some(i) => write!(f, "operation {i}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// The commands for a list of operations, for the editor to execute as one step, and what each operation made.
#[derive(Debug)]
pub struct Planned {
    pub commands: Vec<Command>,
    pub created: Vec<Created>,
    /// The slide the last operation worked on, when it is still there: what the editor shows after.
    pub shown: Option<SlideId>,
}

/// Turn `ops` into commands on `current`, opened from `source`. Each operation sees what the ones before it did; the
/// result is saved (and the bytes thrown away) to make sure the file can be written with it. `new_id` gives a fresh id
/// (the editor's document's: `Document::ids`), for the slides and shapes made.
pub fn plan(ops: &[Op], current: &Presentation, source: &Source, new_id: &mut dyn FnMut() -> u64) -> Result<Planned, OpError> {
    let mut scratch = Document::new(current.clone());
    let mut names = HashMap::new();
    let mut commands = Vec::new();
    let mut created = Vec::new();
    let mut shown = None;
    for (index, op) in ops.iter().enumerate() {
        let fail = |message: String| OpError { index: Some(index), message };
        let mut planner = Planner { p: scratch.presentation(), source, names: &names, new_id: &mut *new_id, touched: Default::default() };
        let (step, made, name) = planner.op(op).map_err(fail)?;
        shown = made.slide.map(SlideId).or(planner.touched.get()).or(shown);
        scratch.execute(step.clone()).map_err(|e| fail(e.to_string()))?;
        if let Some(name) = name {
            let target = match (made.slide, made.shape) {
                (Some(slide), Some(shape)) => Target::Shape(SlideId(slide), ShapeId(shape)),
                (Some(slide), None) => Target::Slide(SlideId(slide)),
                _ => unreachable!("only what makes a slide or a shape is named"),
            };
            names.insert(name, target);
        }
        commands.extend(step);
        created.push(made);
    }
    source
        .save(scratch.presentation(), ibis_pptx::SaveOptions::default())
        .map_err(|e| OpError { index: None, message: format!("the file cannot be saved with this change: {e}") })?;
    let shown = shown.filter(|&id| scratch.presentation().slide(id).is_some());
    Ok(Planned { commands, created, shown })
}

#[derive(Clone, Copy, Debug)]
enum Target {
    Slide(SlideId),
    Shape(SlideId, ShapeId),
}

struct Planner<'a> {
    p: &'a Presentation,
    source: &'a Source,
    names: &'a HashMap<String, Target>,
    new_id: &'a mut dyn FnMut() -> u64,
    /// The last slide an operation named.
    touched: std::cell::Cell<Option<SlideId>>,
}

type Step = (Vec<Command>, Created, Option<String>);

impl Planner<'_> {
    fn op(&mut self, op: &Op) -> Result<Step, String> {
        let p = self.p;
        let none = |commands: Vec<Command>| Ok((commands, Created::default(), None));
        match op {
            Op::AddSlide { index, layout, name } => {
                let index = index.unwrap_or(p.slides.len());
                if index > p.slides.len() {
                    return Err(format!("index {index} is past the end ({} slides)", p.slides.len()));
                }
                let layout = match layout {
                    Some(wanted) => Some(
                        p.layouts
                            .iter()
                            .find(|l| &l.name == wanted)
                            .map(|l| l.id)
                            .ok_or_else(|| format!("no layout named {wanted:?} (the layouts: {})", p.layouts.iter().map(|l| format!("{:?}", l.name)).collect::<Vec<_>>().join(", ")))?,
                    ),
                    None => index.checked_sub(1).or((!p.slides.is_empty()).then_some(0)).and_then(|i| p.slides[i].layout),
                };
                let id = SlideId((self.new_id)());
                let slide = match layout {
                    Some(layout) => {
                        let new_id = &mut *self.new_id;
                        self.source.new_slide(p, index, id, layout, || ShapeId(new_id())).map_err(|e| e.to_string())?
                    }
                    None => Slide::new(id),
                };
                Ok((vec![Command::InsertSlide { index, slide }], Created { slide: Some(id.0), ..Default::default() }, name.clone()))
            }
            Op::DeleteSlide { slide } => none(vec![Command::RemoveSlide { slide: self.slide(slide)? }]),
            Op::MoveSlide { slide, index } => {
                if *index >= p.slides.len() {
                    return Err(format!("index {index} is past the end ({} slides)", p.slides.len()));
                }
                none(vec![Command::MoveSlide { slide: self.slide(slide)?, index: *index }])
            }
            Op::DuplicateSlide { slide, index, name } => {
                let from = self.slide(slide)?;
                let index = index.unwrap_or_else(|| p.slide_index(from).map_or(0, |i| i + 1));
                if index > p.slides.len() {
                    return Err(format!("index {index} is past the end ({} slides)", p.slides.len()));
                }
                let id = SlideId((self.new_id)());
                let copy = p.duplicate_slide(from, id).ok_or("no such slide")?;
                Ok((vec![Command::InsertSlide { index, slide: copy }], Created { slide: Some(id.0), ..Default::default() }, name.clone()))
            }
            Op::SetNotes { slide, text } => none(vec![Command::SetNotes { slide: self.slide(slide)?, notes: text.clone() }]),
            Op::SetBackground { slide, color } => none(vec![Command::SetBackground { slide: self.slide(slide)?, fill: Fill::Solid(Color::opaque(rgb(color)?)) }]),
            Op::AddTextBox { slide, frame, text, format, name } => {
                let slide = self.slide(slide)?;
                let id = ShapeId((self.new_id)());
                let shape = Shape::text_box(id, emu_rect(frame)?, text.as_str());
                let mut commands = vec![self.insert(slide, shape)];
                if let Some(format) = format {
                    let len = text.chars().count();
                    commands.push(Command::SetCharacterFormat { slide, shape: id, start: 0, end: len, format: character_format(format)? });
                }
                Ok((commands, Created { slide: Some(slide.0), shape: Some(id.0), ..Default::default() }, name.clone()))
            }
            Op::AddShape { slide, geometry, frame, fill, line, text, name } => {
                let slide = self.slide(slide)?;
                let outline = ibis_pptx::preset_geometry(geometry).ok_or_else(|| format!("no preset shape {geometry:?}"))?;
                let fill = match fill {
                    Some(color) => Fill::Solid(Color::opaque(rgb(color)?)),
                    None => Fill::Solid(Color::opaque(Rgb(0x4472C4))),
                };
                let stroke = match line {
                    Some(color) => Some(Stroke::solid(Color::opaque(rgb(color)?), Stroke::DEFAULT_WIDTH)),
                    None => None,
                };
                let mut auto = AutoShape::new(outline, fill, stroke);
                auto.text = text.as_deref().map(shape_text);
                let id = ShapeId((self.new_id)());
                let shape = Shape::new(id, emu_rect(frame)?, ShapeKind::Auto(auto));
                Ok((vec![self.insert(slide, shape)], Created { slide: Some(slide.0), shape: Some(id.0), ..Default::default() }, name.clone()))
            }
            Op::AddTable { slide, frame, rows, name } => {
                let slide = self.slide(slide)?;
                let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
                if rows.is_empty() || columns == 0 {
                    return Err("a table needs at least one row and one column".into());
                }
                let frame = match frame {
                    Some(frame) => emu_rect(frame)?,
                    None => {
                        let size = p.size_of(p.slide(slide).expect("resolved"));
                        let width = (columns as i64 * 1_219_200).min(size.width * 8 / 10);
                        let height = rows.len() as i64 * 370_840;
                        EmuRect::new((size.width - width) / 2, (size.height - height) / 2, width, height)
                    }
                };
                let mut table = Table::new_grid(rows.len(), columns, frame.width, frame.height);
                for (r, line) in rows.iter().enumerate() {
                    for (c, text) in line.iter().enumerate() {
                        let cell = &mut table.rows[r].cells[c];
                        let len = cell.text.text().chars().count();
                        cell.text.splice(0, len, text);
                    }
                }
                let id = ShapeId((self.new_id)());
                let shape = Shape::new(id, frame, ShapeKind::Table(Box::new(table)));
                Ok((vec![self.insert(slide, shape)], Created { slide: Some(slide.0), shape: Some(id.0), ..Default::default() }, name.clone()))
            }
            Op::DeleteShape { slide, shape } => {
                let (slide, shape) = self.shape(slide, shape)?;
                none(vec![Command::RemoveShape { slide, shape }])
            }
            Op::SetFrame { slide, shape, x, y, w, h } => {
                let (slide, id) = self.shape(slide, shape)?;
                let old = p.slide(slide).and_then(|s| s.shape(id)).expect("resolved").frame;
                let wanted = Rect { x: x.unwrap_or(pt(old.x)), y: y.unwrap_or(pt(old.y)), w: w.unwrap_or(pt(old.width)), h: h.unwrap_or(pt(old.height)) };
                let frame = EmuRect { x: x.map_or(old.x, emu), y: y.map_or(old.y, emu), width: w.map_or(old.width, emu), height: h.map_or(old.height, emu) };
                emu_rect(&wanted)?;
                none(vec![Command::SetFrame { slide, shape: id, frame }])
            }
            Op::SetText { slide, shape, text } => {
                let (slide, id) = self.shape(slide, shape)?;
                let target = p.slide(slide).and_then(|s| s.shape(id)).expect("resolved");
                match (&target.kind, target.text_body()) {
                    (_, Some(body)) => none(vec![Command::SetTextBody { slide, shape: id, body: replaced(body, text) }]),
                    (ShapeKind::Auto(auto), None) => {
                        let mut auto = auto.clone();
                        auto.text = Some(shape_text(text));
                        none(vec![Command::SetKind { slide, shape: id, kind: Box::new(ShapeKind::Auto(auto)) }])
                    }
                    _ => Err(format!("shape {} has no text (a table's cells are set with set_cell)", id.0)),
                }
            }
            Op::ReplaceText { find, replace, slide, shape } => {
                if find.is_empty() {
                    return Err("find is empty".into());
                }
                let slides: Vec<SlideId> = match slide {
                    Some(slide) => vec![self.slide(slide)?],
                    None if shape.is_some() => return Err("a shape is named with its slide".into()),
                    None => p.slides.iter().map(|s| s.id).collect(),
                };
                let only = match (slide, shape) {
                    (Some(slide), Some(shape)) => Some(self.shape(slide, shape)?.1),
                    _ => None,
                };
                let mut commands = Vec::new();
                let mut count = 0;
                for slide_id in slides {
                    let slide = p.slide(slide_id).expect("listed");
                    for target in slide.shapes.iter().filter(|s| only.is_none_or(|o| o == s.id)) {
                        let edit = |at, remove, insert: &str| Command::ReplaceText { slide: slide_id, shape: target.id, at, remove, insert: insert.to_string() };
                        if let Some(body) = target.text_body() {
                            for at in occurrences(&body.text(), find).into_iter().rev() {
                                commands.push(edit(at, find.chars().count(), replace));
                                count += 1;
                            }
                        }
                        if let ShapeKind::Table(table) = &target.kind {
                            for (r, row) in table.rows.iter().enumerate() {
                                for (c, cell) in row.cells.iter().enumerate() {
                                    for at in occurrences(&cell.text.text(), find).into_iter().rev() {
                                        commands.push(Command::InCell { row: r, column: c, command: Box::new(edit(at, find.chars().count(), replace)) });
                                        count += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                if count == 0 {
                    return Err(format!("{find:?} is not in the text"));
                }
                Ok((commands, Created { replaced: Some(count), ..Default::default() }, None))
            }
            Op::FormatText { slide, shape, matching, cell, format } => {
                let (slide, id) = self.shape(slide, shape)?;
                let body = self.body(slide, id, *cell)?;
                let text = body.text();
                let ranges: Vec<(usize, usize)> = match matching {
                    Some(m) if m.is_empty() => return Err("match is empty".into()),
                    Some(m) => occurrences(&text, m).into_iter().map(|at| (at, at + m.chars().count())).collect(),
                    None => vec![(0, text.chars().count())],
                };
                if ranges.is_empty() {
                    return Err(format!("{:?} is not in the text", matching.as_deref().unwrap_or_default()));
                }
                let format = character_format(format)?;
                none(ranges.into_iter().map(|(start, end)| in_cell(*cell, Command::SetCharacterFormat { slide, shape: id, start, end, format: format.clone() })).collect())
            }
            Op::FormatParagraphs { slide, shape, paragraphs, cell, format } => {
                let (slide, id) = self.shape(slide, shape)?;
                let body = self.body(slide, id, *cell)?;
                let spans = paragraph_spans(body);
                let chosen: Vec<(usize, usize)> = match paragraphs {
                    Some(list) => list.iter().map(|&i| spans.get(i).copied().ok_or_else(|| format!("no paragraph {i} (there are {})", spans.len()))).collect::<Result<_, _>>()?,
                    None => vec![(0, spans.last().map_or(0, |s| s.1))],
                };
                let format = paragraph_format(format)?;
                none(chosen.into_iter().map(|(start, end)| in_cell(*cell, Command::SetParagraphFormat { slide, shape: id, start, end, format: format.clone() })).collect())
            }
            Op::SetFill { slide, shape, color } => {
                let (slide, id) = self.shape(slide, shape)?;
                let fill = match color {
                    Some(color) => Fill::Solid(Color::opaque(rgb(color)?)),
                    None => Fill::None,
                };
                none(vec![Command::SetFill { slide, shape: id, fill }])
            }
            Op::SetLine { slide, shape, color, width } => {
                let (slide, id) = self.shape(slide, shape)?;
                let stroke = match color {
                    Some(color) => Some(Stroke::solid(Color::opaque(rgb(color)?), width.map_or(Ok(Stroke::DEFAULT_WIDTH), positive)?)),
                    None => None,
                };
                none(vec![Command::SetStroke { slide, shape: id, stroke }])
            }
            Op::SetCell { slide, shape, row, column, text } => {
                let (slide, id) = self.shape(slide, shape)?;
                let body = self.body(slide, id, Some([*row, *column]))?;
                none(vec![in_cell(Some([*row, *column]), Command::SetTextBody { slide, shape: id, body: replaced(body, text) })])
            }
            Op::InsertTableRow { slide, shape, at } => {
                let (slide, id) = self.shape(slide, shape)?;
                let (target, table) = self.table(slide, id)?;
                let at = at.unwrap_or(table.rows.len());
                let height = table.rows.get(at.saturating_sub(1).min(table.rows.len() - 1)).map_or(0, |r| r.height);
                let new = table.with_row_inserted(at).ok_or("a row cannot be inserted there (a table with merged cells, or past the end)")?;
                self.change_table(slide, target, new, height)
            }
            Op::DeleteTableRow { slide, shape, at } => {
                let (slide, id) = self.shape(slide, shape)?;
                let (target, table) = self.table(slide, id)?;
                let height = table.rows.get(*at).map_or(0, |r| r.height);
                let new = table.with_row_removed(*at).ok_or("that row cannot be deleted (a table with merged cells, its only row, or past the end)")?;
                self.change_table(slide, target, new, -height)
            }
        }
    }

    fn slide(&self, r: &Ref) -> Result<SlideId, String> {
        let id = self.resolve_slide(r)?;
        self.touched.set(Some(id));
        Ok(id)
    }

    fn resolve_slide(&self, r: &Ref) -> Result<SlideId, String> {
        match r {
            Ref::Id(id) => self.p.slide(SlideId(*id)).map(|s| s.id).ok_or_else(|| format!("no slide {id}")),
            Ref::Name(name) => match self.names.get(name) {
                Some(Target::Slide(id)) => Ok(*id),
                Some(Target::Shape(..)) => Err(format!("{name:?} is a shape, not a slide")),
                None => Err(format!("nothing named {name:?} was made before")),
            },
            Ref::Placeholder { .. } => Err("a slide is named by its id or its `as` name".into()),
        }
    }

    fn shape(&self, slide: &Ref, r: &Ref) -> Result<(SlideId, ShapeId), String> {
        let slide = self.slide(slide)?;
        let on = self.p.slide(slide).expect("resolved");
        let id = match r {
            Ref::Id(id) => ShapeId(*id),
            Ref::Name(name) => match self.names.get(name) {
                Some(Target::Shape(s, id)) if *s == slide => *id,
                Some(Target::Shape(..)) => return Err(format!("{name:?} is on another slide")),
                Some(Target::Slide(_)) => return Err(format!("{name:?} is a slide, not a shape")),
                None => return Err(format!("nothing named {name:?} was made before")),
            },
            Ref::Placeholder { placeholder } => {
                let wanted = ibis_doc_model::model::PlaceholderRef { kind: Some(placeholder.clone()), index: None };
                let mut found = on.shapes.iter().filter(|s| on.placeholders.get(&s.id).is_some_and(|p| p.placeholder.normalized() == wanted.normalized()));
                match (found.next(), found.next()) {
                    (Some(shape), None) => shape.id,
                    (Some(_), Some(_)) => return Err(format!("slide {} has more than one {placeholder} placeholder: name it by its id", slide.0)),
                    (None, _) => return Err(format!("slide {} has no {placeholder} placeholder", slide.0)),
                }
            }
        };
        if on.shape(id).is_none() || on.is_attached(id) {
            return Err(format!("no shape {} on slide {}", id.0, slide.0));
        }
        Ok((slide, id))
    }

    /// The text of a shape, or of its table's cell `cell`.
    fn body(&self, slide: SlideId, id: ShapeId, cell: Option<[usize; 2]>) -> Result<&TextBody, String> {
        let shape = self.p.slide(slide).and_then(|s| s.shape(id)).expect("resolved");
        match (cell, &shape.kind) {
            (Some([r, c]), ShapeKind::Table(table)) => table.rows.get(r).and_then(|row| row.cells.get(c)).map(|cell| &cell.text).ok_or_else(|| format!("no cell [{r}, {c}] (the table is {} by {})", table.rows.len(), table.columns.len())),
            (Some(_), _) => Err(format!("shape {} is not a table", id.0)),
            (None, ShapeKind::Table(_)) => Err(format!("shape {} is a table: name a cell", id.0)),
            (None, _) => shape.text_body().ok_or_else(|| format!("shape {} has no text", id.0)),
        }
    }

    fn table(&self, slide: SlideId, id: ShapeId) -> Result<(&Shape, &Table), String> {
        let shape = self.p.slide(slide).and_then(|s| s.shape(id)).expect("resolved");
        match &shape.kind {
            ShapeKind::Table(table) => Ok((shape, table)),
            _ => Err(format!("shape {} is not a table", id.0)),
        }
    }

    /// A table's new rows and the frame grown by `grow` for them, its new cells as the table's style makes them (read
    /// back through the file, as the editor's 行の挿入 does).
    fn change_table(&self, slide: SlideId, shape: &Shape, table: Table, grow: i64) -> Result<Step, String> {
        let mut changed = shape.clone();
        changed.kind = ShapeKind::Table(Box::new(table));
        changed.frame.height = (changed.frame.height + grow).max(1);
        let mut presentation = self.p.clone();
        let i = presentation.slide_index(slide).expect("resolved");
        if let Some(target) = presentation.slides[i].shapes.iter_mut().find(|s| s.id == shape.id) {
            *target = changed.clone();
        }
        let kind = self.source.as_read(&presentation, slide, shape.id).map_or(changed.kind, |read| read.kind);
        Ok((vec![Command::SetKind { slide, shape: shape.id, kind: Box::new(kind) }, Command::SetFrame { slide, shape: shape.id, frame: changed.frame }], Created::default(), None))
    }

    /// Put `shape` on top of `slide`.
    fn insert(&self, slide: SlideId, shape: Shape) -> Command {
        let index = self.p.slide(slide).map_or(0, |s| s.shapes.len());
        Command::InsertShape { slide, index, shape }
    }
}

fn in_cell(cell: Option<[usize; 2]>, command: Command) -> Command {
    match cell {
        Some([row, column]) => Command::InCell { row, column, command: Box::new(command) },
        None => command,
    }
}

/// `body` with its text replaced by `text`: paragraph `i` keeps the format of the old paragraph `i` (the last one's, past
/// its end), and its characters that of the old paragraph's first run.
fn replaced(body: &TextBody, text: &str) -> TextBody {
    let old = &body.paragraphs;
    let paragraphs = text
        .split('\n')
        .enumerate()
        .map(|(i, line)| {
            let template = old.get(i).or(old.last()).cloned().unwrap_or_default();
            let style = template.runs.first().map_or_else(|| template.end_style.clone(), |r| r.style.clone());
            Paragraph { runs: if line.is_empty() { Vec::new() } else { vec![Run { text: line.to_string(), style: style.clone() }] }, end_style: style, ..template }
        })
        .collect();
    TextBody { paragraphs, ..body.clone() }
}

/// The text of a new shape: centred in it, as PowerPoint's shapes have theirs.
fn shape_text(text: &str) -> TextBody {
    let mut body = TextBody::plain(text, TextStyle { align: TextAlign::Center, color: Rgb::WHITE, ..TextStyle::default() });
    body.anchor = VerticalAnchor::Middle;
    body
}

/// Where each paragraph's characters start and end in the text (paragraphs joined by `\n`).
fn paragraph_spans(body: &TextBody) -> Vec<(usize, usize)> {
    let mut at = 0;
    body.paragraphs
        .iter()
        .map(|p| {
            let len = p.text().chars().count();
            let span = (at, at + len);
            at += len + 1;
            span
        })
        .collect()
}

/// The character positions of each `find` in `text`, not overlapping.
fn occurrences(text: &str, find: &str) -> Vec<usize> {
    text.match_indices(find).map(|(byte, _)| text[..byte].chars().count()).collect()
}

fn character_format(format: &TextFormat) -> Result<CharacterFormat, String> {
    Ok(CharacterFormat {
        bold: format.bold,
        italic: format.italic,
        underline: format.underline,
        font_size: format.size.map(|s| if s > 0.0 && s <= 4000.0 { Ok((s * 100.0).round() as u32) } else { Err(format!("size {s} is not a font size")) }).transpose()?,
        color: format.color.as_deref().map(rgb).transpose()?,
        font: format.font.clone().map(Some),
        ..Default::default()
    })
}

fn paragraph_format(format: &ParagraphFormat) -> Result<ibis_doc_model::text::ParagraphFormat, String> {
    if format.level.is_some_and(|l| l > 8) {
        return Err("level is 0 to 8".into());
    }
    let points = |v: Option<f64>| v.map(|v| if v >= 0.0 { Ok((v * 100.0).round() as u32) } else { Err(format!("{v} is negative")) }).transpose();
    Ok(ibis_doc_model::text::ParagraphFormat {
        align: format.align.map(|a| match a {
            Align::Left => TextAlign::Left,
            Align::Center => TextAlign::Center,
            Align::Right => TextAlign::Right,
            Align::Justify => TextAlign::Justify,
        }),
        list: format.list.map(|l| match l {
            List::None => ListKind::None,
            List::Bullet => ListKind::Bullet,
            List::Numbered => ListKind::Numbered,
        }),
        level: format.level,
        space_before: points(format.space_before)?,
        space_after: points(format.space_after)?,
        ..Default::default()
    })
}

fn rgb(color: &str) -> Result<Rgb, String> {
    let hex = color.strip_prefix('#').unwrap_or(color);
    if hex.len() != 6 {
        return Err(format!("{color:?} is not a colour (#RRGGBB)"));
    }
    u32::from_str_radix(hex, 16).map(Rgb).map_err(|_| format!("{color:?} is not a colour (#RRGGBB)"))
}

fn emu(points: f64) -> i64 {
    (points * EMU_PER_POINT as f64).round() as i64
}

fn pt(emu: i64) -> f64 {
    emu as f64 / EMU_PER_POINT as f64
}

fn positive(points: f64) -> Result<i64, String> {
    if points > 0.0 && points.is_finite() { Ok(emu(points)) } else { Err(format!("{points} is not a width")) }
}

fn emu_rect(r: &Rect) -> Result<EmuRect, String> {
    if ![r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite()) || r.w <= 0.0 || r.h <= 0.0 {
        return Err(format!("the frame {r:?} needs a width and a height above 0"));
    }
    Ok(EmuRect::new(emu(r.x), emu(r.y), emu(r.w), emu(r.h)))
}
