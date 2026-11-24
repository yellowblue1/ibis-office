// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The operations an agent edits a workbook with, and how they become the editor's commands.
//!
//! Cells and ranges are in A1 notation (`B2`, `A1:C3`), rows by their numbers (`1:3`), columns by their letters
//! (`A:C`), colours `#RRGGBB`. A sheet is named by the id [`crate::inspect`] gives, by its name, or, for one made earlier
//! in the same list, by the name its `as` gave it. Only the shown sheets are edited (a hidden one is as it is in the
//! editor, where it is shown again first), and what the editor refuses the person is refused here: a protected sheet's or
//! workbook's changes, a pivot table's cells, a value its cell's data validation stops.

use std::collections::HashMap;

use ibis_calc::formula::parser::{column_index, parse_cell};
use ibis_calc::formula::{MAX_COLUMN, MAX_ROW};
use ibis_calc::input::{Typed, parse_imported};
use ibis_doc_model::command::{Command, EditError};
use ibis_doc_model::geometry::EMU_PER_POINT;
use ibis_doc_model::model::{Presentation, ShapeId, SlideId};
use ibis_doc_model::sheet::{CellAlign, CellRange, CellValue, Sheet, Workbook, cell_name, referenced_cell, replace_ignoring_case};
use ibis_doc_model::sheet_calc::sheet_of_slide;
use ibis_doc_model::sheet_command::{Axis, CellEdit, CellInput, SheetCommand, StyleChange};
use ibis_doc_model::sheet_ops::valid_sheet_name;
use ibis_doc_model::sheet_validation::{Breach, breach, formula_value, list_items, rule_at};
use ibis_doc_model::Document;
use ibis_pptx::Source;
use serde::{Deserialize, Deserializer, Serialize};

use super::inspect::range_name;
use crate::shared::rgb;
use crate::{OpError, Planned};

/// The points of one character of a column's width, as the editor's 列の幅 counts them (`ibis-office-xlsx`'s
/// `row_col_size.rs`), and the widest and highest Excel takes.
const POINTS_PER_CHARACTER: f64 = 6.4;
const MAX_WIDTH: f64 = 255.0;
const MAX_HEIGHT: f64 = 409.0;

/// A sheet: its id, or its name (as Excel, in any case), or the `as` name of one made earlier in the same list.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum SheetRef {
    Id(u64),
    Name(String),
}

/// What is put in a cell; `null` empties it.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Number(f64),
    Text(String),
}

/// Rows by their numbers: `3`, `"3"` or `"1:3"`.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Rows {
    One(u32),
    Text(String),
}

/// A cell format: what is given is set, the rest is left as it is.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellFormat {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    /// In points.
    pub size: Option<f64>,
    pub color: Option<String>,
    pub font: Option<String>,
    /// `null` takes the fill off.
    #[serde(default, deserialize_with = "present")]
    pub fill: Option<Option<String>>,
    /// Excel's code: `#,##0`, `0.0%`, `yyyy/mm/dd`, `@` (text), `General`…
    pub number_format: Option<String>,
    pub align: Option<Align>,
    pub wrap: Option<bool>,
}

/// A field given (`null` too), told from one left out.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// Put `values` (rows of them) in the cells from `at` right and down. A string is read as the editor reads text
    /// pasted or imported into a cell: `=` starts a formula; a number, a percentage, a currency or a date written out
    /// (`1,234`, `50%`, `¥1,000`, `2026/11/24`) is that number, a cell of General taking the number format Excel gives
    /// it; `TRUE` and `FALSE` are logical values; a leading `'` keeps the rest as text, as any other string is (a
    /// sign-led word such as `-x` too); in a cell formatted as text (`@`) everything is text. A JSON number or logical
    /// value is put as it is, and `null` (or `""`) empties the cell. A formula refers to cells as typed in the editor
    /// does, and takes the number format of the cell it refers to as entering it there does. What depends on the cells is
    /// calculated again.
    SetCells { sheet: SheetRef, at: String, values: Vec<Vec<Option<Value>>> },
    /// Empty the cells of `range` (their formats stay).
    Clear { sheet: SheetRef, range: String },
    /// A new, empty sheet at `index` (0-based, in the workbook's order; the end when not given), named `name` (`SheetN`,
    /// the first not taken, when not given).
    AddSheet {
        name: Option<String>,
        index: Option<usize>,
        #[serde(rename = "as")]
        alias: Option<String>,
    },
    RenameSheet { sheet: SheetRef, name: String },
    /// Delete a sheet (not the last one shown); the formulas that refer to it get `#REF!`.
    DeleteSheet { sheet: SheetRef },
    /// Move a sheet to `index` (0-based, its place after the move).
    MoveSheet { sheet: SheetRef, index: usize },
    /// Insert `count` (1 when not given) empty rows before row `at` (1-based); formulas follow the cells that move.
    /// Past the last row the sheet keeps there is nothing to move, and nothing is done.
    InsertRows { sheet: SheetRef, at: u32, count: Option<u32> },
    DeleteRows { sheet: SheetRef, at: u32, count: Option<u32> },
    /// Insert `count` empty columns before column `at` (its letters).
    InsertColumns { sheet: SheetRef, at: String, count: Option<u32> },
    DeleteColumns { sheet: SheetRef, at: String, count: Option<u32> },
    FormatCells { sheet: SheetRef, range: String, format: CellFormat },
    /// Merge a range: the cells but the top-left one are emptied, as Excel's セルを結合する does.
    Merge { sheet: SheetRef, range: String },
    /// Undo the merged ranges within `range`.
    Unmerge { sheet: SheetRef, range: String },
    /// The width of `columns` (`A:C`, `B`), in characters (0 hides them).
    SetColumnWidth { sheet: SheetRef, columns: String, width: f64 },
    /// The height of `rows` (`1:3`, `2`), in points (0 hides them).
    SetRowHeight { sheet: SheetRef, rows: Rows, height: f64 },
    /// Replace every `find`, in any case (as 検索と置換 does by default), with `replace` in the text of the cells that hold
    /// text (not in formulas, nor in pivot tables), in `sheet` (every shown, unprotected sheet when not given).
    ReplaceText { find: String, replace: String, sheet: Option<SheetRef> },
}

/// What an operation made, for the agent to name it next.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Created {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sheet: Option<u64>,
    /// How many places `replace_text` replaced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaced: Option<usize>,
}

/// Turn `ops` into commands on `current`, opened from `source`. Each operation sees what the ones before it did (the
/// formulas calculated); the result is saved (and the bytes thrown away) to make sure the file can be written with it.
/// `new_id` gives a fresh id (the editor's document's: `Document::ids`), for the sheets made.
pub fn plan(ops: &[Op], current: &Presentation, source: &Source, new_id: &mut dyn FnMut() -> u64) -> Result<Planned, OpError> {
    let mut scratch = Document::new(current.clone());
    let mut names = HashMap::new();
    let mut commands = Vec::new();
    let mut created = Vec::new();
    let mut shown = None;
    for (index, op) in ops.iter().enumerate() {
        let fail = |message: String| OpError { index: Some(index), message };
        let p = scratch.presentation();
        let book = p.workbook.as_deref().ok_or_else(|| fail("the file has no workbook".into()))?;
        let mut planner = Planner { p, book, names: &names, new_id: &mut *new_id, touched: Default::default() };
        let (step, made, alias) = planner.op(op).map_err(fail)?;
        shown = made.sheet.map(SlideId).or(planner.touched.get()).or(shown);
        scratch.execute(step.clone()).map_err(|e| fail(edit_error(e)))?;
        if let (Some(alias), Some(sheet)) = (alias, made.sheet) {
            names.insert(alias, SlideId(sheet));
        }
        commands.extend(step);
        created.push(made);
    }
    crate::check_saves(source, scratch.presentation())?;
    let shown = shown.filter(|&id| scratch.presentation().slide(id).is_some());
    let replaced = created.iter().filter_map(|c| c.replaced).sum();
    let created = created.iter().map(|c| serde_json::to_value(c).expect("serialisable")).collect();
    Ok(Planned { commands, created, replaced, shown })
}

/// A refusal of the model's, in a workbook's words (its pages are sheets).
fn edit_error(e: EditError) -> String {
    match e {
        EditError::NoSuchSlide(id) => format!("no sheet {}", id.0),
        EditError::LastSlide => "the last shown sheet cannot be deleted or hidden".into(),
        EditError::WorkbookProtected => "the workbook's structure is protected (ブックの保護)".into(),
        other => other.to_string(),
    }
}

struct Planner<'a> {
    p: &'a Presentation,
    book: &'a Workbook,
    names: &'a HashMap<String, SlideId>,
    new_id: &'a mut dyn FnMut() -> u64,
    /// The last sheet an operation named.
    touched: std::cell::Cell<Option<SlideId>>,
}

type Step = (Vec<Command>, Created, Option<String>);

impl Planner<'_> {
    fn op(&mut self, op: &Op) -> Result<Step, String> {
        let none = |commands: Vec<SheetCommand>| Ok((commands.into_iter().map(Command::Sheet).collect(), Created::default(), None));
        match op {
            Op::SetCells { sheet, at, values } => {
                let slide = self.editable(sheet)?;
                none(self.set_cells(slide, at, values)?)
            }
            Op::Clear { sheet, range: wanted } => {
                let slide = self.editable(sheet)?;
                let r = range(wanted)?;
                self.outside_pivots(slide, &r, wanted)?;
                let edits: Vec<CellEdit> = self
                    .content(slide)?
                    .cells
                    .iter()
                    .filter(|c| inside(&r, c.row, c.column) && (!c.value.is_empty() || c.formula.is_some()))
                    .map(|c| CellEdit { row: c.row, column: c.column, input: CellInput::Clear })
                    .collect();
                none(if edits.is_empty() { Vec::new() } else { vec![SheetCommand::SetCells { slide, edits }] })
            }
            Op::AddSheet { name, index, alias } => {
                self.structure_unlocked()?;
                let count = self.book.sheets.len();
                let index = index.unwrap_or(count);
                if index > count {
                    return Err(format!("index {index} is past the end ({count} sheets)"));
                }
                let name = match name {
                    Some(name) => {
                        self.check_name(name, None)?;
                        name.clone()
                    }
                    None => (count + 1..=count + 1 + count).map(|n| format!("Sheet{n}")).find(|n| valid_sheet_name(self.p, n, None)).ok_or("no SheetN name is free: give the sheet a name")?,
                };
                if let Some(alias) = alias {
                    self.check_alias(alias, &name)?;
                }
                let (slide, shape) = (SlideId((self.new_id)()), ShapeId((self.new_id)()));
                let commands = vec![Command::Sheet(SheetCommand::AddSheet { index, name, slide, shape })];
                Ok((commands, Created { sheet: Some(slide.0), ..Default::default() }, alias.clone()))
            }
            Op::RenameSheet { sheet, name } => {
                let slide = self.sheet(sheet)?;
                self.structure_unlocked()?;
                self.check_name(name, self.position(slide))?;
                if let Some((alias, _)) = self.names.iter().find(|(alias, id)| alias.to_lowercase() == name.to_lowercase() && **id != slide) {
                    return Err(format!("{alias:?} names another sheet made earlier in the list: a sheet named so would be ambiguous"));
                }
                none(vec![SheetCommand::RenameSheet { slide, name: name.clone() }])
            }
            Op::DeleteSheet { sheet } => {
                let slide = self.sheet(sheet)?;
                self.structure_unlocked()?;
                if self.book.sheets.iter().filter(|s| s.slide.is_some()).count() <= 1 {
                    return Err("the last shown sheet cannot be deleted: add another first".into());
                }
                none(vec![SheetCommand::DeleteSheet { slide }])
            }
            Op::MoveSheet { sheet, index } => {
                let slide = self.sheet(sheet)?;
                self.structure_unlocked()?;
                let count = self.book.sheets.len();
                if *index >= count {
                    return Err(format!("index {index} is past the end ({count} sheets)"));
                }
                none(vec![SheetCommand::MoveSheet { slide, to: *index }])
            }
            Op::InsertRows { sheet, at, count } | Op::DeleteRows { sheet, at, count } => {
                let slide = self.editable(sheet)?;
                if *at == 0 {
                    return Err("rows are numbered from 1".into());
                }
                let insert = matches!(op, Op::InsertRows { .. });
                none(self.lines(slide, Axis::Rows, insert, at - 1, count.unwrap_or(1))?)
            }
            Op::InsertColumns { sheet, at, count } | Op::DeleteColumns { sheet, at, count } => {
                let slide = self.editable(sheet)?;
                let at = column(at)?;
                let insert = matches!(op, Op::InsertColumns { .. });
                none(self.lines(slide, Axis::Columns, insert, at, count.unwrap_or(1))?)
            }
            Op::FormatCells { sheet, range: wanted, format } => {
                let slide = self.editable(sheet)?;
                let r = range(wanted)?;
                let changes = style_changes(format)?;
                if changes.is_empty() {
                    return Err("the format sets nothing".into());
                }
                none(changes.into_iter().map(|change| SheetCommand::SetFormat { slide, ranges: vec![r], change }).collect())
            }
            Op::Merge { sheet, range: wanted } => {
                let slide = self.editable(sheet)?;
                let r = range(wanted)?;
                if r.first_row == r.last_row && r.first_column == r.last_column {
                    return Err(format!("{wanted} is one cell: a merge takes two or more"));
                }
                self.outside_pivots(slide, &r, wanted)?;
                none(vec![SheetCommand::Merge { slide, range: r, center: false }])
            }
            Op::Unmerge { sheet, range: wanted } => {
                let slide = self.editable(sheet)?;
                let r = range(wanted)?;
                let within = |m: &CellRange| r.first_row <= m.first_row && m.last_row <= r.last_row && r.first_column <= m.first_column && m.last_column <= r.last_column;
                let merges = &self.content(slide)?.merges;
                if !merges.iter().any(within) {
                    let all: Vec<String> = merges.iter().take(20).map(range_name).collect();
                    return Err(format!("no merged range within {wanted} (the sheet's: {})", if all.is_empty() { "none".into() } else { all.join(", ") }));
                }
                none(vec![SheetCommand::Unmerge { slide, range: r }])
            }
            Op::SetColumnWidth { sheet, columns, width } => {
                let slide = self.editable(sheet)?;
                let (first, last) = column_span(columns)?;
                if !(0.0..=MAX_WIDTH).contains(width) {
                    return Err(format!("width {width} is not 0 to {MAX_WIDTH} characters"));
                }
                let size = (width * POINTS_PER_CHARACTER * EMU_PER_POINT as f64).round() as i64;
                none(vec![SheetCommand::SetSize { slide, axis: Axis::Columns, first, last, size }])
            }
            Op::SetRowHeight { sheet, rows, height } => {
                let slide = self.editable(sheet)?;
                let (first, last) = row_span(rows)?;
                if !(0.0..=MAX_HEIGHT).contains(height) {
                    return Err(format!("height {height} is not 0 to {MAX_HEIGHT} points"));
                }
                let size = (height * EMU_PER_POINT as f64).round() as i64;
                none(vec![SheetCommand::SetSize { slide, axis: Axis::Rows, first, last, size }])
            }
            Op::ReplaceText { find, replace, sheet } => {
                if find.is_empty() {
                    return Err("find is empty".into());
                }
                let slides: Vec<SlideId> = match sheet {
                    Some(sheet) => vec![self.editable(sheet)?],
                    None => self.book.sheets.iter().filter_map(|s| s.slide).filter(|&id| self.content(id).is_ok_and(|s| s.protection.is_none())).collect(),
                };
                let mut commands = Vec::new();
                let mut count = 0;
                for slide in slides {
                    let sheet = self.content(slide)?;
                    let in_pivot = |row, column| sheet.pivots.iter().any(|p| p.touches(&one(row, column)));
                    let edits: Vec<CellEdit> = sheet
                        .cells
                        .iter()
                        .filter_map(|c| match (&c.formula, &c.value) {
                            (None, CellValue::Text(t)) if !in_pivot(c.row, c.column) => {
                                let (text, n) = replace_ignoring_case(t, find, replace)?;
                                count += n;
                                Some(CellEdit { row: c.row, column: c.column, input: CellInput::Value(CellValue::Text(text)) })
                            }
                            _ => None,
                        })
                        .collect();
                    if !edits.is_empty() {
                        commands.push(Command::Sheet(SheetCommand::SetCells { slide, edits }));
                    }
                }
                if count == 0 {
                    return Err(format!("{find:?} is not in the text of any cell"));
                }
                Ok((commands, Created { replaced: Some(count), ..Default::default() }, None))
            }
        }
    }

    fn set_cells(&self, slide: SlideId, at: &str, values: &[Vec<Option<Value>>]) -> Result<Vec<SheetCommand>, String> {
        let (row, column) = cell(at)?;
        if values.iter().all(Vec::is_empty) {
            return Err("values has no value".into());
        }
        let sheet = self.content(slide)?;
        let sheets: Vec<&str> = self.book.sheets.iter().map(|s| s.name.as_str()).collect();
        let mut edits = Vec::new();
        // The number formats this operation gives, for a formula after them that refers to their cells.
        let mut given: HashMap<(u32, u32), String> = HashMap::new();
        let mut formats: Vec<(String, Vec<CellRange>)> = Vec::new();
        for (dr, line) in values.iter().enumerate() {
            for (dc, value) in line.iter().enumerate() {
                let (row, column) = (row as u64 + dr as u64, column as u64 + dc as u64);
                if row > MAX_ROW as u64 || column > MAX_COLUMN as u64 {
                    return Err(format!("the values from {at} go past the sheet's last row or column"));
                }
                let (row, column) = (row as u32, column as u32);
                let fail = |e: String| format!("{}: {e}", cell_name(column, row));
                if sheet.pivots.iter().any(|p| p.touches(&one(row, column))) {
                    return Err(fail("the cell is in a pivot table, which its field list changes".into()));
                }
                let (input, format) = entry(sheet, row, column, value.as_ref(), self.book.date1904, &sheets, &given).map_err(fail)?;
                self.validated(slide, sheet, row, column, &input).map_err(fail)?;
                if let Some(code) = format {
                    given.insert((row, column), code.clone());
                    match formats.iter_mut().find(|(c, _)| *c == code) {
                        Some((_, ranges)) => ranges.push(one(row, column)),
                        None => formats.push((code, vec![one(row, column)])),
                    }
                }
                edits.push(CellEdit { row, column, input });
            }
        }
        let mut commands = vec![SheetCommand::SetCells { slide, edits }];
        commands.extend(formats.into_iter().map(|(code, ranges)| SheetCommand::SetFormat { slide, ranges, change: StyleChange::NumberFormat(code) }));
        Ok(commands)
    }

    /// Refuse `input` where the cell's data validation stops it, as the editor's cell entry does (a formula by its value
    /// as things stand). A rule's warning, which asks the person whether to go on, is a refusal too: no one is there to
    /// answer it; its information lets the value in.
    fn validated(&self, slide: SlideId, sheet: &Sheet, row: u32, column: u32, input: &CellInput) -> Result<(), String> {
        let Some(rule) = rule_at(sheet, row, column) else { return Ok(()) };
        let value = match input {
            CellInput::Value(v) => v.clone(),
            CellInput::Clear => CellValue::Empty,
            CellInput::Formula(f) => match formula_value(self.p, slide, row, column, f) {
                Some(value) => value,
                None => return Ok(()),
            },
        };
        let Some((how, title, text)) = breach(self.p, slide, row, column, &value) else { return Ok(()) };
        if how == Breach::Information {
            return Ok(());
        }
        let mut message = "the value breaks the cell's data validation".to_string();
        if how == Breach::NotListed {
            let items = list_items(self.p, slide, rule, row, column);
            message += &format!(" (one of: {})", items.iter().take(20).map(|i| format!("{i:?}")).collect::<Vec<_>>().join(", "));
        }
        for part in [title, text].into_iter().filter(|p| !p.is_empty()) {
            message += &format!(": {part}");
        }
        Err(message)
    }

    /// Refuse a change of `r`'s cells that would change a pivot table, as the editor does.
    fn outside_pivots(&self, slide: SlideId, r: &CellRange, wanted: &str) -> Result<(), String> {
        if self.content(slide)?.pivots.iter().any(|p| p.touches(r)) {
            return Err(format!("{wanted} reaches into a pivot table, which its field list changes"));
        }
        Ok(())
    }

    /// Rows or columns inserted or deleted. Past the sheet's last one there is nothing to move: rows or columns deleted
    /// there are only those up to it, and none are inserted.
    fn lines(&self, slide: SlideId, axis: Axis, insert: bool, at: u32, count: u32) -> Result<Vec<SheetCommand>, String> {
        if count == 0 {
            return Err("count is 0".into());
        }
        let sheet = self.content(slide)?;
        let rows = axis == Axis::Rows;
        let (len, last) = if rows { (sheet.rows.len() as u32, MAX_ROW) } else { (sheet.columns.len() as u32, MAX_COLUMN) };
        if at as u64 + count as u64 > last as u64 + 1 {
            return Err(format!("that goes past the sheet's last {} ({})", if rows { "row" } else { "column" }, last + 1));
        }
        let before = insert.then_some(at);
        if sheet.pivots.iter().any(|p| p.crossed_by_lines(rows, at, at + count - 1, before)) {
            return Err(format!("the {} go through a pivot table, which its field list changes", if rows { "rows" } else { "columns" }));
        }
        if insert {
            // What the last `count` rows (columns) hold would be pushed off the sheet.
            let kept = last + 1 - count;
            let off = sheet.cells.iter().any(|c| (if rows { c.row } else { c.column }) >= kept && (!c.value.is_empty() || c.formula.is_some()));
            if off {
                return Err("cannot shift nonblank cells off the worksheet: clear or delete the last rows or columns first".into());
            }
        }
        let count = if insert { count } else { count.min(len.saturating_sub(at)) };
        if at >= len || count == 0 {
            return Ok(Vec::new());
        }
        Ok(vec![if insert { SheetCommand::Insert { slide, axis, at, count } } else { SheetCommand::Delete { slide, axis, at, count } }])
    }

    fn content(&self, slide: SlideId) -> Result<&Sheet, String> {
        self.p.slide(slide).and_then(sheet_of_slide).map(|s| &**s).ok_or_else(|| format!("no sheet {}", slide.0))
    }

    fn position(&self, slide: SlideId) -> Option<usize> {
        self.book.sheets.iter().position(|s| s.slide == Some(slide))
    }

    fn name_of(&self, slide: SlideId) -> String {
        self.position(slide).and_then(|i| self.book.sheets.get(i)).map(|s| s.name.clone()).unwrap_or_default()
    }

    fn sheet(&self, r: &SheetRef) -> Result<SlideId, String> {
        let id = self.resolve_sheet(r)?;
        self.touched.set(Some(id));
        Ok(id)
    }

    /// A sheet whose cells, rows and columns are changed. A protected one is refused whole, its unlocked cells too: the
    /// person protected it, and the agent does not pick through what the protection lets through.
    fn editable(&self, r: &SheetRef) -> Result<SlideId, String> {
        let id = self.sheet(r)?;
        if self.content(id)?.protection.is_some() {
            return Err(format!("{:?} is protected (シートの保護): its cells are not changed until the protection is taken off", self.name_of(id)));
        }
        Ok(id)
    }

    fn structure_unlocked(&self) -> Result<(), String> {
        if self.book.protection.as_ref().is_some_and(|p| p.lock_structure) {
            return Err("the workbook's structure is protected (ブックの保護): sheets are not added, renamed, moved or deleted".into());
        }
        Ok(())
    }

    fn resolve_sheet(&self, r: &SheetRef) -> Result<SlideId, String> {
        let entry = match r {
            SheetRef::Id(id) => self.book.sheets.iter().find(|s| s.slide == Some(SlideId(*id))).ok_or_else(|| format!("no sheet {id} (the sheets: {})", self.listing()))?,
            SheetRef::Name(name) => match self.names.get(name) {
                Some(&id) => {
                    return match self.content(id) {
                        Ok(_) => Ok(id),
                        Err(_) => Err(format!("{name:?} named a sheet an earlier operation deleted")),
                    };
                }
                None => self.book.sheet_index(name).and_then(|i| self.book.sheets.get(i)).ok_or_else(|| format!("no sheet named {name:?} (the sheets: {})", self.listing()))?,
            },
        };
        match entry.slide.filter(|&id| self.content(id).is_ok()) {
            Some(id) => Ok(id),
            None if entry.other => Err(format!("{:?} is not a worksheet (a chart sheet): it has no cells to edit", entry.name)),
            None => Err(format!("{:?} is hidden: only the shown sheets are edited", entry.name)),
        }
    }

    /// The sheets, as an error names them: their names and the shown ones' ids.
    fn listing(&self) -> String {
        let mut list: Vec<String> = self
            .book
            .sheets
            .iter()
            .take(50)
            .map(|s| match s.slide {
                Some(id) => format!("{:?} (id {})", s.name, id.0),
                None => format!("{:?} (not edited)", s.name),
            })
            .collect();
        if self.book.sheets.len() > list.len() {
            list.push(format!("and {} more", self.book.sheets.len() - list.len()));
        }
        list.join(", ")
    }

    /// Refuse a name Excel does not take for the sheet at `except` (`None`: a new one).
    fn check_name(&self, name: &str, except: Option<usize>) -> Result<(), String> {
        if valid_sheet_name(self.p, name, except) {
            return Ok(());
        }
        let taken = self.book.sheets.iter().enumerate().any(|(i, s)| Some(i) != except && s.name.to_lowercase() == name.to_lowercase());
        Err(if taken {
            format!("a sheet named {name:?} is there already")
        } else {
            format!("{name:?} is not a sheet name: 1 to 31 characters, none of [ ] : * ? / \\, not starting or ending with ', and not History (Excel keeps that name)")
        })
    }

    /// Refuse an `as` name for the new sheet `name` that could be taken for another sheet.
    fn check_alias(&self, alias: &str, name: &str) -> Result<(), String> {
        if self.names.contains_key(alias) {
            return Err(format!("{alias:?} names a sheet made earlier in the list already"));
        }
        if alias.to_lowercase() != name.to_lowercase() && self.book.sheet_index(alias).is_some() {
            return Err(format!("{alias:?} is the name of a sheet: an `as` name that is one would be ambiguous"));
        }
        Ok(())
    }
}

/// What putting `value` into the cell at (`row`, `column`) puts there, and the number format the cell then takes (only
/// a cell of General takes one), as the editor's cell entry (`ibis-office-xlsx`'s `commit_cell_edit`) makes them from text
/// read as data (`parse_imported`). `given` holds the formats earlier cells of the same entry take. The editor's AutoCorrect,
/// its links made of typed addresses and its 数式オートコンプリート are the person's typing aids, and are not applied.
fn entry(sheet: &Sheet, row: u32, column: u32, value: Option<&Value>, date1904: bool, sheets: &[&str], given: &HashMap<(u32, u32), String>) -> Result<(CellInput, Option<String>), String> {
    let code = |row, column| sheet.cell(row, column).and_then(|c| sheet.styles.get(c.style as usize)).map_or("General", |s| s.number_format.as_str());
    let own = code(row, column);
    let (input, format) = match value {
        None => (CellInput::Clear, None),
        Some(Value::Bool(b)) => (CellInput::Value(CellValue::Bool(*b)), None),
        Some(Value::Number(n)) if n.is_finite() => (CellInput::Value(CellValue::Number(*n)), None),
        Some(Value::Number(n)) => return Err(format!("{n} is not a number a cell holds")),
        Some(Value::Text(t)) if t.is_empty() => (CellInput::Clear, None),
        // A cell formatted as text keeps what is typed as text, a date or a formula too, as Excel does.
        Some(Value::Text(t)) if own == "@" => (CellInput::Value(CellValue::Text(t.strip_prefix('\'').unwrap_or(t).to_string())), None),
        // Read as data, not as typing: a sign-led word (`-x`) stays text, so only `=` makes a formula.
        Some(Value::Text(t)) => match parse_imported(t, date1904) {
            Typed::Formula(f) => {
                ibis_calc::formula::parse(&f).map_err(|e| format!("the formula ={f} does not read: {e}"))?;
                let f = ibis_calc::formula::normalize_case(&f, sheets);
                let format = match referenced_cell(&f) {
                    Some(at) if given.contains_key(&at) => given.get(&at).cloned(),
                    _ => sheet.referenced_format(&f),
                };
                (CellInput::Formula(f), format)
            }
            Typed::Value(v, format) => (CellInput::Value(v), format.map(str::to_string)),
        },
    };
    Ok((input, format.filter(|_| own == "General")))
}

fn style_changes(format: &CellFormat) -> Result<Vec<StyleChange>, String> {
    let mut changes = Vec::new();
    if let Some(v) = format.bold {
        changes.push(StyleChange::Bold(v));
    }
    if let Some(v) = format.italic {
        changes.push(StyleChange::Italic(v));
    }
    if let Some(v) = format.underline {
        changes.push(StyleChange::Underline(v));
    }
    if let Some(s) = format.size {
        if !(1.0..=409.0).contains(&s) {
            return Err(format!("size {s} is not a font size (1 to 409 points)"));
        }
        changes.push(StyleChange::FontSize((s * 100.0).round() as u32));
    }
    if let Some(c) = &format.color {
        changes.push(StyleChange::Color(rgb(c)?));
    }
    if let Some(f) = &format.font {
        changes.push(StyleChange::Font((!f.is_empty()).then(|| f.clone())));
    }
    if let Some(fill) = &format.fill {
        changes.push(StyleChange::Fill(fill.as_deref().map(rgb).transpose()?));
    }
    if let Some(code) = &format.number_format {
        if !ibis_calc::numfmt::is_complete_code(code) {
            return Err(format!("{code:?} is not a number format code (up to 4 sections and 255 characters, quotes and brackets closed)"));
        }
        changes.push(StyleChange::NumberFormat(code.clone()));
    }
    if let Some(a) = format.align {
        changes.push(StyleChange::Align(match a {
            Align::Left => CellAlign::Left,
            Align::Center => CellAlign::Center,
            Align::Right => CellAlign::Right,
        }));
    }
    if let Some(v) = format.wrap {
        changes.push(StyleChange::Wrap(v));
    }
    Ok(changes)
}

fn one(row: u32, column: u32) -> CellRange {
    CellRange { first_row: row, first_column: column, last_row: row, last_column: column }
}

fn inside(r: &CellRange, row: u32, column: u32) -> bool {
    (r.first_row..=r.last_row).contains(&row) && (r.first_column..=r.last_column).contains(&column)
}

/// A cell in A1 notation, as (row, column), 0-based.
fn cell(text: &str) -> Result<(u32, u32), String> {
    parse_cell(text.trim()).map(|c| (c.row, c.column)).ok_or_else(|| format!("{text:?} is not a cell (A1 notation, such as B2)"))
}

/// `A1:C3` (either corner first), or one cell.
fn range(text: &str) -> Result<CellRange, String> {
    let bad = || format!("{text:?} is not a range of cells (A1 notation, such as A1:C3 or B2)");
    let (a, b) = match text.split_once(':') {
        Some((a, b)) => (cell(a).map_err(|_| bad())?, cell(b).map_err(|_| bad())?),
        None => {
            let one = cell(text).map_err(|_| bad())?;
            (one, one)
        }
    };
    Ok(CellRange { first_row: a.0.min(b.0), first_column: a.1.min(b.1), last_row: a.0.max(b.0), last_column: a.1.max(b.1) })
}

fn column(text: &str) -> Result<u32, String> {
    let letters = text.trim().trim_start_matches('$');
    column_index(letters).filter(|_| !letters.is_empty()).ok_or_else(|| format!("{text:?} is not a column (its letters, such as C)"))
}

/// `A:C` or `B`, as (first, last).
fn column_span(text: &str) -> Result<(u32, u32), String> {
    let (a, b) = match text.split_once(':') {
        Some((a, b)) => (column(a)?, column(b)?),
        None => (column(text)?, column(text)?),
    };
    Ok((a.min(b), a.max(b)))
}

/// `1:3`, `2` or 2, as 0-based (first, last).
fn row_span(rows: &Rows) -> Result<(u32, u32), String> {
    let bad = |text: &str| format!("{text:?} is not rows (their numbers, such as 1:3 or 2)");
    let number = |text: &str| text.trim().trim_start_matches('$').parse::<u32>().ok().filter(|&n| n >= 1 && n <= MAX_ROW + 1).ok_or_else(|| bad(text));
    let (a, b) = match rows {
        Rows::One(n) => {
            let n = number(&n.to_string())?;
            (n, n)
        }
        Rows::Text(text) => match text.split_once(':') {
            Some((a, b)) => (number(a).map_err(|_| bad(text))?, number(b).map_err(|_| bad(text))?),
            None => (number(text)?, number(text)?),
        },
    };
    Ok((a.min(b) - 1, a.max(b) - 1))
}
