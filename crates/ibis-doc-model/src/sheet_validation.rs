// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! データの入力規則 as an entry meets it: whether a value put into a cell keeps the cell's rule, the list's items, the
//! bounds and a custom rule's formula worked out across the workbook (another sheet's range, a name, `OFFSET(…)`). The
//! editor's cell entry and the agent's `set_cells` both ask here; what each does with a breach (a prompt, a refusal) is
//! theirs.

use ibis_calc::eval::Book;

use crate::model::{Presentation, SlideId};
use crate::sheet::{CellValue, Sheet, Validation};
use crate::sheet_calc::{BookView, sheet_of_slide};

/// The rule over a cell (the last one listed wins, as Excel keeps one).
pub fn rule_at(sheet: &Sheet, row: u32, column: u32) -> Option<&Validation> {
    sheet.validations.iter().rev().find(|v| v.ranges.iter().any(|r| (r.first_row..=r.last_row).contains(&row) && (r.first_column..=r.last_column).contains(&column)))
}

/// How a rule meets a value it does not keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Breach {
    /// Not one of a list rule's items: never entered, whatever the rule's error style (Excel's own message, not the rule's).
    NotListed,
    /// 停止: the value is not entered.
    Stop,
    /// 注意: entered when the person says so.
    Warning,
    /// 情報: entered, the person told.
    Information,
}

/// Whether `value` put into (`row`, `column`) of the sheet on `slide` keeps the cell's rule: `None` when it does (or there
/// is no rule, or a rule other than a list's shows no message for a value it refuses), else how the rule meets it and the
/// rule's message (title and text, either empty when the rule has none; both empty for [`Breach::NotListed`]).
pub fn breach(presentation: &Presentation, slide: SlideId, row: u32, column: u32, value: &CellValue) -> Option<(Breach, String, String)> {
    let sheet = presentation.slide(slide).and_then(sheet_of_slide)?;
    let rule = rule_at(sheet, row, column)?;
    if rule.kind == "list" {
        return (!keeps(presentation, slide, rule, value, row, column)).then(|| (Breach::NotListed, String::new(), String::new()));
    }
    let (style, title, text) = rule.error.as_ref()?;
    if keeps(presentation, slide, rule, value, row, column) {
        return None;
    }
    let breach = match style.as_str() {
        "warning" => Breach::Warning,
        "information" => Breach::Information,
        _ => Breach::Stop,
    };
    Some((breach, title.clone(), text.clone()))
}

/// The value a formula entered into (`row`, `column`) gives as things stand, which a rule checks as it does a typed value
/// (as Excel); `None` when the engine cannot work it out.
pub fn formula_value(presentation: &Presentation, slide: SlideId, row: u32, column: u32, formula: &str) -> Option<CellValue> {
    let view = BookView::new(presentation);
    let index = view.index_of_slide(slide)?;
    let (value, unsupported) = ibis_calc::recalc::evaluate(&view, (index, row, column), formula);
    (!unsupported).then_some(value)
}

/// A list rule's items, as the cell's drop-down shows them: the text in quotes split at commas, the shown text of the
/// cells a reference names (on any sheet, through a name or `OFFSET(…)`), or the values a formula gives.
pub fn list_items(presentation: &Presentation, slide: SlideId, rule: &Validation, row: u32, column: u32) -> Vec<String> {
    let Some(f) = rule.formula1.as_deref().map(str::trim) else { return Vec::new() };
    if let Some(inner) = f.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        return inner.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    }
    let view = BookView::new(presentation);
    let Some(index) = view.index_of_slide(slide) else { return Vec::new() };
    let formula = f.trim_start_matches('=').trim();
    if let Some(areas) = ibis_calc::recalc::evaluate_reference(&view, (index, row, column), formula)
        && let Some(items) = shown_texts(&view, &areas)
    {
        return items;
    }
    ibis_calc::recalc::evaluate_list(&view, (index, row, column), formula).unwrap_or_default().iter().map(text_of).filter(|t| !t.is_empty()).collect()
}

/// The text the cells of `areas` show, row by row, empty ones left out; whole columns or rows end at the sheet's data.
fn shown_texts(view: &BookView, areas: &[ibis_calc::eval::Area]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for area in areas {
        let sheet = view.sheet(area.sheet)?;
        let (rows, columns) = sheet.cells.iter().fold((0, 0), |(r, c), x| (r.max(x.row + 1), c.max(x.column + 1)));
        let last_row = area.last_row.min(rows.saturating_sub(1).max(area.first_row));
        let last_column = area.last_column.min(columns.saturating_sub(1).max(area.first_column));
        for row in area.first_row..=last_row {
            for column in area.first_column..=last_column {
                if let Some(c) = sheet.cell(row, column)
                    && !c.text.is_empty()
                {
                    out.push(c.text.clone());
                }
            }
        }
    }
    Some(out)
}

/// Whether `value` put into (`row`, `column`) of the sheet on `slide` keeps `rule`.
pub fn keeps(presentation: &Presentation, slide: SlideId, rule: &Validation, value: &CellValue, row: u32, column: u32) -> bool {
    if value.is_empty() {
        return rule.allow_blank || rule.kind == "none";
    }
    let view = BookView::new(presentation);
    let Some(sheet) = view.index_of_slide(slide) else { return true };
    // A number, the first cell of a range a bound names, or what its formula gives.
    let bound = |f: &Option<String>| -> Option<f64> {
        let f = f.as_deref()?.trim();
        if let Ok(n) = f.parse::<f64>() {
            return Some(n);
        }
        let f = f.trim_start_matches('=').trim();
        let value = match ibis_calc::recalc::evaluate_reference(&view, (sheet, row, column), f).as_deref() {
            Some([first, ..]) => view.value(first.sheet, first.first_row, first.first_column),
            _ => ibis_calc::recalc::evaluate(&view, (sheet, row, column), f).0,
        };
        match value {
            CellValue::Number(n) => Some(n),
            _ => None,
        }
    };
    let compare = |v: f64| -> bool {
        let (a, b) = (bound(&rule.formula1), bound(&rule.formula2));
        match (rule.operator.as_str(), a) {
            ("between", Some(a)) => b.is_some_and(|b| v >= a.min(b) && v <= a.max(b)),
            ("notBetween", Some(a)) => b.is_some_and(|b| v < a.min(b) || v > a.max(b)),
            ("equal", Some(a)) => v == a,
            ("notEqual", Some(a)) => v != a,
            ("greaterThan", Some(a)) => v > a,
            ("lessThan", Some(a)) => v < a,
            ("greaterThanOrEqual", Some(a)) => v >= a,
            ("lessThanOrEqual", Some(a)) => v <= a,
            _ => true,
        }
    };
    match rule.kind.as_str() {
        "list" => {
            let text = text_of(value);
            let half = ibis_calc::functions::to_half_width(&text);
            list_items(presentation, slide, rule, row, column).iter().any(|i| i.eq_ignore_ascii_case(&text) || i.eq_ignore_ascii_case(&half))
        }
        "whole" => matches!(value, CellValue::Number(n) if n.fract() == 0.0 && compare(*n)),
        "decimal" | "date" | "time" => matches!(value, CellValue::Number(n) if compare(*n)),
        "textLength" => {
            let len = match value {
                CellValue::Text(t) => t.chars().count(),
                CellValue::Number(n) => ibis_calc::eval::number_to_text(*n).chars().count(),
                _ => 0,
            };
            compare(len as f64)
        }
        "custom" => keeps_custom(presentation, slide, rule, value, row, column),
        _ => true,
    }
}

/// A custom rule's formula worked out with the value in the cell (its references moved from the rule's first cell to this
/// one): kept unless it gives FALSE, 0 or an error, as Excel.
fn keeps_custom(presentation: &Presentation, slide: SlideId, rule: &Validation, value: &CellValue, row: u32, column: u32) -> bool {
    let (Some(formula), Some(base)) = (rule.formula1.as_deref(), rule.base()) else { return true };
    let mut with = presentation.clone();
    let Ok(sheet) = crate::sheet_command::sheet_mut(&mut with, slide) else { return true };
    let mut cell = sheet.cell(row, column).cloned().unwrap_or(crate::sheet::SheetCell { runs: None, phonetic: None, row, column, value: CellValue::Empty, text: String::new(), kind: crate::sheet::CellKind::Empty, formula: None, style: 0 });
    cell.value = value.clone();
    cell.formula = None;
    sheet.put_cell(row, column, Some(cell));
    let shifted = ibis_calc::formula::shift(formula, row as i64 - base.0 as i64, column as i64 - base.1 as i64);
    let view = BookView::new(&with);
    let Some(index) = view.index_of_slide(slide) else { return true };
    match ibis_calc::recalc::evaluate(&view, (index, row, column), &shifted).0 {
        CellValue::Bool(b) => b,
        CellValue::Number(n) => n != 0.0,
        CellValue::Error(_) => false,
        _ => true,
    }
}

fn text_of(value: &CellValue) -> String {
    match value {
        CellValue::Number(n) => ibis_calc::eval::number_to_text(*n),
        CellValue::Text(t) => t.clone(),
        CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        CellValue::Error(e) => e.as_str().to_string(),
        CellValue::Empty => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::geometry::{EmuRect, EmuSize};
    use crate::model::{Shape, ShapeKind, Slide};
    use crate::sheet::{CellKind, CellRange, DefinedName, SheetCell, Workbook, WorkbookSheet};

    fn cell(row: u32, column: u32, value: CellValue, text: &str) -> SheetCell {
        SheetCell { runs: None, phonetic: None, row, column, value, text: text.into(), kind: CellKind::Text, formula: None, style: 0 }
    }

    /// A book of a shown sheet (`Sheet1`, where the rules are), a second one with the cells given, and a hidden third
    /// with the same cells.
    fn book(other: Vec<SheetCell>, rules: Vec<Validation>, names: Vec<DefinedName>) -> (Presentation, SlideId) {
        let mut p = Presentation::empty(EmuSize::WIDESCREEN);
        let mut sheets = Vec::new();
        let mut first = None;
        for (name, sheet) in [("Sheet1", Sheet { validations: rules, ..Sheet::default() }), ("Sheet2", Sheet { cells: other.clone().into(), ..Sheet::default() })] {
            let id = p.new_slide_id();
            first.get_or_insert(id);
            let mut slide = Slide::new(id);
            slide.name = Some(name.into());
            let shape = p.new_shape_id();
            slide.shapes.push(Shape { id: shape, frame: EmuRect::new(0, 0, 1000, 1000), rotation: 0, flip_h: false, flip_v: false, kind: ShapeKind::Sheet(Arc::new(sheet)), floating: None });
            p.slides.push(slide);
            sheets.push(WorkbookSheet { name: name.into(), state: Default::default(), slide: Some(id), hidden: None, other: false });
        }
        sheets.push(WorkbookSheet { name: "Hidden".into(), state: crate::sheet::SheetState::Hidden, slide: None, hidden: Some(Arc::new(Sheet { cells: other.into(), ..Sheet::default() })), other: false });
        p.workbook = Some(Arc::new(Workbook { sheets, names, ..Default::default() }));
        (p, first.unwrap())
    }

    fn rule(kind: &str, operator: &str, formula1: &str, error: Option<&str>) -> Validation {
        Validation {
            ranges: vec![CellRange { first_row: 0, first_column: 0, last_row: 9, last_column: 0 }],
            kind: kind.into(),
            operator: operator.into(),
            formula1: Some(formula1.into()),
            error: error.map(|style| (style.into(), "入力".into(), "範囲外です".into())),
            ..Default::default()
        }
    }

    fn text(t: &str) -> CellValue {
        CellValue::Text(t.into())
    }

    #[test]
    fn lists_and_numbers() {
        let (p, s) = book(Vec::new(), vec![rule("list", "", r#""りんご,みかん""#, None)], Vec::new());
        assert_eq!(breach(&p, s, 0, 0, &text("みかん")), None);
        assert_eq!(breach(&p, s, 0, 0, &text("ぶどう")), Some((Breach::NotListed, String::new(), String::new())));
        let mut whole = rule("whole", "between", "1", Some("stop"));
        whole.formula2 = Some("10".into());
        let (p, s) = book(Vec::new(), vec![whole], Vec::new());
        assert_eq!(breach(&p, s, 0, 0, &CellValue::Number(5.0)), None);
        for refused in [5.5, 11.0] {
            assert_eq!(breach(&p, s, 0, 0, &CellValue::Number(refused)), Some((Breach::Stop, "入力".into(), "範囲外です".into())));
        }
    }

    /// A list from another sheet's cells (shown or hidden, by reference or by a name) offers what they show, a number
    /// format's text and not the number behind it.
    #[test]
    fn a_list_offers_the_cells_shown_text() {
        let cells = vec![cell(0, 0, text("red"), "red"), cell(1, 0, CellValue::Number(46350.0), "2026/11/24"), cell(2, 0, text("blue"), "blue")];
        let names = vec![DefinedName { name: "Colours".into(), sheet: None, formula: "Sheet2!$A$1:$A$3".into(), hidden: false }];
        for source in ["'Sheet2'!$A$1:$A$3", "Colours", "Hidden!A:A", "OFFSET(Sheet2!A1,0,0,3,1)"] {
            let list = rule("list", "", source, None);
            let (p, s) = book(cells.clone(), vec![list.clone()], names.clone());
            assert_eq!(list_items(&p, s, &list, 0, 0), ["red", "2026/11/24", "blue"], "{source}");
            assert_eq!(breach(&p, s, 0, 0, &text("blue")), None, "{source}");
            assert_eq!(breach(&p, s, 0, 0, &text("pink")).map(|b| b.0), Some(Breach::NotListed), "{source}");
        }
    }

    /// A bound may be a name or another sheet's cell, and a rule's error style says how a breach is met; a rule other than
    /// a list's with no message lets every value in.
    #[test]
    fn bounds_and_styles() {
        let cells = vec![cell(0, 0, CellValue::Number(100.0), "100")];
        let names = vec![DefinedName { name: "MaxLimit".into(), sheet: None, formula: "Sheet2!$A$1".into(), hidden: false }];
        for (bound, style, how) in [("MaxLimit", Some("stop"), Some(Breach::Stop)), ("=Sheet2!A1", Some("warning"), Some(Breach::Warning)), ("Sheet2!A1:A3", Some("information"), Some(Breach::Information)), ("MaxLimit", None, None)] {
            let (p, s) = book(cells.clone(), vec![rule("whole", "lessThanOrEqual", bound, style)], names.clone());
            assert_eq!(breach(&p, s, 0, 0, &CellValue::Number(80.0)), None, "{bound}");
            assert_eq!(breach(&p, s, 0, 0, &CellValue::Number(150.0)).map(|b| b.0), how, "{bound}");
        }
    }

    /// A custom rule's formula is worked out with the value in the cell, its references moved to that cell.
    #[test]
    fn a_custom_rule_sees_the_value() {
        let (p, s) = book(vec![cell(3, 0, CellValue::Number(10.0), "10")], vec![rule("custom", "", "A1<=Sheet2!$A$4", Some("stop"))], Vec::new());
        assert_eq!(breach(&p, s, 2, 0, &CellValue::Number(9.0)), None);
        assert_eq!(breach(&p, s, 2, 0, &CellValue::Number(11.0)).map(|b| b.0), Some(Breach::Stop));
        assert_eq!(formula_value(&p, s, 2, 0, "Sheet2!A4*2"), Some(CellValue::Number(20.0)));
    }
}
