// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The workbook as the agent reads it: its sheets in the workbook's order, each shown one with its id, and the cells
//! with a value or a formula, by row. The ids are the editor's (a sheet's is its page's): they stay while the workbook is
//! open, and are what the [operations](super::Op) name.
//!
//! A big workbook would flood the agent, so the summary lists at most [`SHEETS`] sheets, [`CELLS_PER_SHEET`] cells and
//! [`MERGED_PER_SHEET`] merged ranges of a sheet, and [`TEXT_CHARACTERS`] characters of a cell's value and shown text;
//! each says what it left out.

use ibis_doc_model::model::Presentation;
use ibis_doc_model::sheet::{CellRange, CellValue, SheetState, WorkbookSheet, cell_name};
use ibis_doc_model::sheet_calc::sheet_of_slide;
use serde::Serialize;

pub const SHEETS: usize = 100;
/// The first rows first.
pub const CELLS_PER_SHEET: usize = 2000;
pub const MERGED_PER_SHEET: usize = 200;
/// A formula is never cut: it is what the agent writes back.
pub const TEXT_CHARACTERS: usize = 1000;

#[derive(Debug, Serialize)]
pub struct Inspection {
    pub name: String,
    /// Whether there are edits not saved yet.
    pub dirty: bool,
    pub sheets: Vec<SheetInfo>,
    /// How many sheets past [`SHEETS`] were left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more_sheets: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SheetInfo {
    /// A shown worksheet's; a hidden one and a chart sheet have none, and are not edited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub name: String,
    /// 1-based, in the workbook's order (hidden sheets counted): `add_sheet`'s and `move_sheet`'s `index` is this less 1.
    pub number: usize,
    /// Hidden (非表示, or hidden so that only a macro shows it again).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Not a worksheet (a chart sheet, a dialog or macro sheet): no cells, and not edited.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub other: bool,
    /// Protected (シートの保護): read, its cells not changed.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub protected: bool,
    /// The range from the first row and column with something in them to the last (`A1:F42`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<String>,
    pub cells: Vec<CellInfo>,
    /// How many cells past [`CELLS_PER_SHEET`] were left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub merged: Vec<String>,
    /// How many merged ranges past [`MERGED_PER_SHEET`] were left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more_merged: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct CellInfo {
    pub at: String,
    /// A string, a number or a logical value; an error is its text (`#DIV/0!`), and a formula with no value `null`.
    pub value: serde_json::Value,
    /// With its `=`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    /// What the cell shows, when that is not the value written out (a number format, a date).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The value or the shown text is longer than [`TEXT_CHARACTERS`], and only its start is given.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cut: bool,
}

pub fn inspect(presentation: &Presentation, name: &str, dirty: bool) -> Inspection {
    let sheets = presentation.workbook.as_ref().map_or(&[][..], |w| &w.sheets[..]);
    Inspection {
        name: name.to_string(),
        dirty,
        sheets: sheets.iter().take(SHEETS).enumerate().map(|(i, s)| sheet_info(presentation, s, i + 1)).collect(),
        more_sheets: left_out(sheets.len(), SHEETS),
    }
}

fn left_out(count: usize, cap: usize) -> Option<usize> {
    count.checked_sub(cap).filter(|&n| n > 0)
}

fn sheet_info(presentation: &Presentation, entry: &WorkbookSheet, number: usize) -> SheetInfo {
    let shown = entry.slide.and_then(|id| presentation.slide(id)).and_then(sheet_of_slide).filter(|_| !entry.other);
    let sheet = shown.or(entry.hidden.as_ref()).filter(|_| !entry.other);
    let mut info = SheetInfo {
        id: shown.and(entry.slide).map(|id| id.0),
        name: entry.name.clone(),
        number,
        hidden: entry.state != SheetState::Visible,
        other: entry.other,
        protected: sheet.is_some_and(|s| s.protection.is_some()),
        used: None,
        cells: Vec::new(),
        more: None,
        merged: Vec::new(),
        more_merged: None,
    };
    let Some(sheet) = sheet else { return info };
    let filled: Vec<_> = sheet.cells.iter().filter(|c| !c.value.is_empty() || c.formula.is_some()).collect();
    info.used = filled
        .iter()
        .fold(None, |used: Option<CellRange>, c| {
            Some(match used {
                None => CellRange { first_row: c.row, first_column: c.column, last_row: c.row, last_column: c.column },
                Some(r) => CellRange { first_row: r.first_row.min(c.row), first_column: r.first_column.min(c.column), last_row: r.last_row.max(c.row), last_column: r.last_column.max(c.column) },
            })
        })
        .map(|r| range_name(&r));
    info.more = left_out(filled.len(), CELLS_PER_SHEET);
    info.cells = filled
        .iter()
        .take(CELLS_PER_SHEET)
        .map(|c| {
            let written = written(&c.value);
            let (value, value_cut) = match &c.value {
                CellValue::Text(t) => {
                    let (t, cut) = start_of(t);
                    (serde_json::Value::String(t), cut)
                }
                other => (value_json(other), false),
            };
            let (text, text_cut) = if c.text != written {
                let (t, cut) = start_of(&c.text);
                (Some(t), cut)
            } else {
                (None, false)
            };
            CellInfo { at: cell_name(c.column, c.row), value, formula: c.formula.as_ref().map(|f| format!("={}", sheet.formula_shown(c.row, c.column, f))), text, cut: value_cut || text_cut }
        })
        .collect();
    info.merged = sheet.merges.iter().take(MERGED_PER_SHEET).map(range_name).collect();
    info.more_merged = left_out(sheet.merges.len(), MERGED_PER_SHEET);
    info
}

/// The first [`TEXT_CHARACTERS`] of `text`, and whether that left some out.
fn start_of(text: &str) -> (String, bool) {
    match text.char_indices().nth(TEXT_CHARACTERS) {
        Some((byte, _)) => (text[..byte].to_string(), true),
        None => (text.to_string(), false),
    }
}

/// `A1:C3`, or `B2` for one cell.
pub(crate) fn range_name(r: &CellRange) -> String {
    let first = cell_name(r.first_column, r.first_row);
    if r.first_row == r.last_row && r.first_column == r.last_column { first } else { format!("{first}:{}", cell_name(r.last_column, r.last_row)) }
}

fn value_json(value: &CellValue) -> serde_json::Value {
    match value {
        CellValue::Empty => serde_json::Value::Null,
        // A whole number as one, so `5` is not read back as `5.0`.
        CellValue::Number(n) if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 => serde_json::Value::from(*n as i64),
        CellValue::Number(n) => serde_json::Number::from_f64(*n).map_or(serde_json::Value::Null, serde_json::Value::Number),
        CellValue::Text(t) => serde_json::Value::String(t.clone()),
        CellValue::Bool(b) => serde_json::Value::Bool(*b),
        CellValue::Error(e) => serde_json::Value::String(e.as_str().to_string()),
    }
}

/// The value as a cell of General shows it: the shown text is told only when it differs.
fn written(value: &CellValue) -> String {
    match value {
        CellValue::Empty => String::new(),
        CellValue::Number(n) => ibis_calc::eval::number_to_text(*n),
        CellValue::Text(t) => t.clone(),
        CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        CellValue::Error(e) => e.as_str().to_string(),
    }
}
