// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! データの入力規則 as an entry meets it: whether a value put into a cell keeps the cell's rule, the list's items, the
//! bounds and a custom rule's formula worked out across the workbook (another sheet's range, a name, `OFFSET(…)`).

use crate::model::{Presentation, SlideId};
use crate::sheet::{CellValue, Sheet, Validation};
use crate::sheet_calc::{BookView, sheet_of_slide};

/// The rule over a cell (the last one listed wins, as Excel keeps one).
pub fn rule_at(sheet: &Sheet, row: u32, column: u32) -> Option<&Validation> {
    sheet.validations.iter().rev().find(|v| v.ranges.iter().any(|r| (r.first_row..=r.last_row).contains(&row) && (r.first_column..=r.last_column).contains(&column)))
}

/// How a rule meets a value it does not keep (its error style).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Breach {
    /// 停止, and a list's rule always: the value is not entered.
    Stop,
    /// 注意: entered when the person says so.
    Warning,
    /// 情報: entered, the person told.
    Information,
}

/// Whether `value` put into (`row`, `column`) of the sheet on `slide` keeps the cell's rule: `None` when it does (or there
/// is no rule, or the rule shows no message for a value it refuses), else how the rule meets it and its message (title
/// and text, either empty when the rule has none).
pub fn breach(presentation: &Presentation, slide: SlideId, row: u32, column: u32, value: &CellValue) -> Option<(Breach, String, String)> {
    let sheet = presentation.slide(slide).and_then(sheet_of_slide)?;
    let rule = rule_at(sheet, row, column)?;
    let shown = rule.error.as_ref().map(|(style, title, text)| (style.as_str(), title.clone(), text.clone()));
    if rule.kind != "list" && shown.is_none() {
        return None;
    }
    if keeps(presentation, slide, rule, value, row, column) {
        return None;
    }
    let (style, title, text) = shown.unwrap_or(("stop", String::new(), String::new()));
    let breach = match style {
        _ if rule.kind == "list" => Breach::Stop,
        "warning" => Breach::Warning,
        "information" => Breach::Information,
        _ => Breach::Stop,
    };
    Some((breach, title, text))
}

/// A list rule's items, as the cell's drop-down shows them.
pub fn list_items(presentation: &Presentation, slide: SlideId, rule: &Validation, row: u32, column: u32) -> Vec<String> {
    let Some(f) = rule.formula1.as_deref().map(str::trim) else { return Vec::new() };
    if let Some(inner) = f.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        return inner.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    }
    let view = BookView::new(presentation);
    let Some(sheet) = view.index_of_slide(slide) else { return Vec::new() };
    ibis_calc::recalc::evaluate_list(&view, (sheet, row, column), f.trim_start_matches('=')).unwrap_or_default().iter().map(text_of).filter(|t| !t.is_empty()).collect()
}

fn keeps(presentation: &Presentation, slide: SlideId, rule: &Validation, value: &CellValue, row: u32, column: u32) -> bool {
    if value.is_empty() {
        return rule.allow_blank || rule.kind == "none";
    }
    let view = BookView::new(presentation);
    let Some(sheet) = view.index_of_slide(slide) else { return true };
    let bound = |f: &Option<String>| -> Option<f64> {
        let f = f.as_deref()?.trim();
        f.parse::<f64>().ok().or_else(|| match ibis_calc::recalc::evaluate(&view, (sheet, row, column), f.trim_start_matches('=')).0 {
            CellValue::Number(n) => Some(n),
            _ => None,
        })
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
