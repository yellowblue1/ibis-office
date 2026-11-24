// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::sync::Arc;

use ibis_doc_model::geometry::EMU_PER_POINT;
use ibis_doc_model::model::SlideId;
use ibis_doc_model::sheet::{CellAlign, Sheet};
use ibis_doc_model::sheet_calc::sheet_of_slide;
use serde_json::json;

use super::{CELLS_PER_SHEET, CellInfo, Inspection};
use crate::{Applied, OpError, Ops, Session};

fn blank() -> Session {
    Session::open(crate::blank("new.xlsx").unwrap(), "new.xlsx").unwrap()
}

fn parse_ops(json: &str) -> Result<Ops, String> {
    crate::parse_ops(ibis_pptx::Kind::Workbook, json)
}

fn apply(session: &mut Session, json: &str) -> Result<Applied, OpError> {
    session.apply(&parse_ops(json).unwrap())
}

fn doc(session: &Session) -> Inspection {
    session.inspect().into_workbook()
}

fn find<'a>(doc: &'a Inspection, sheet: &str, at: &str) -> Option<&'a CellInfo> {
    doc.sheets.iter().find(|s| s.name == sheet)?.cells.iter().find(|c| c.at == at)
}

/// `sheet`'s cell `at`: its value, formula and shown text, as the summary gives them.
fn cell(session: &Session, sheet: &str, at: &str) -> (serde_json::Value, Option<String>, Option<String>) {
    let doc = doc(session);
    let c = find(&doc, sheet, at).unwrap_or_else(|| panic!("no {sheet}!{at}"));
    (c.value.clone(), c.formula.clone(), c.text.clone())
}

fn content(session: &Session, number: usize) -> Arc<Sheet> {
    sheet_of_slide(&session.model().slides[number]).unwrap().clone()
}

/// The new workbook's one sheet, empty: its id is its page's, and the summary says it is a workbook.
#[test]
fn inspect_names_the_sheets() {
    let session = blank();
    let doc = doc(&session);
    assert_eq!(doc.sheets.len(), 1);
    let sheet = &doc.sheets[0];
    assert_eq!((sheet.id, sheet.name.as_str(), sheet.number, sheet.cells.len()), (Some(session.model().slides[0].id.0), "Sheet1", 1, 0));
    let json = serde_json::to_value(session.inspect()).unwrap();
    assert_eq!(json["kind"], "workbook");
    assert_eq!(json["sheets"][0], json!({"id": sheet.id, "name": "Sheet1", "number": 1, "cells": []}));
}

/// A formula computes as typed in the editor, and what depends on a cell is calculated again when it changes; the list
/// is one undo.
#[test]
fn formulas_compute_and_follow_their_cells() {
    let mut session = blank();
    apply(&mut session, r#"{"op": "set_cells", "sheet": "Sheet1", "at": "A1", "values": [[2], [3], ["=a1+a2"], ["=sum(A1:A3)"]]}"#).unwrap();
    assert_eq!(cell(&session, "Sheet1", "A3"), (json!(5), Some("=A1+A2".into()), None));
    assert_eq!(cell(&session, "Sheet1", "A4"), (json!(10), Some("=SUM(A1:A3)".into()), None));
    assert_eq!(doc(&session).sheets[0].used.as_deref(), Some("A1:A4"));
    assert!(doc(&session).dirty);

    apply(&mut session, r#"{"op": "set_cells", "sheet": "Sheet1", "at": "A1", "values": [[10]]}"#).unwrap();
    assert_eq!(cell(&session, "Sheet1", "A3").0, json!(13));
    assert!(session.document.undo());
    assert_eq!(cell(&session, "Sheet1", "A3").0, json!(5));
    assert!(session.document.undo());
    assert!(doc(&session).sheets[0].cells.is_empty());
}

/// A string is read as typing it would be: numbers, percentages, dates and logical values, with the number format Excel
/// gives them; `'` keeps text, a sign-led word is text (not a formula), and `null` or `""` empties the cell.
#[test]
fn values_are_entered_as_typed() {
    let mut session = blank();
    apply(
        &mut session,
        r#"[{"op": "set_cells", "sheet": "Sheet1", "at": "B2", "values": [["50%", "1,234", "2026/11/24", "TRUE", "'007", "-x", 7.5, false, "x"]]},
            {"op": "set_cells", "sheet": "Sheet1", "at": "J2", "values": [[null]]}]"#,
    )
    .unwrap();
    let doc = doc(&session);
    let row: Vec<_> = doc.sheets[0].cells.iter().map(|c| (c.at.as_str(), c.value.clone(), c.text.clone())).collect();
    assert_eq!(
        row,
        [
            ("B2", json!(0.5), Some("50%".to_string())),
            ("C2", json!(1234), Some("1,234".to_string())),
            ("D2", json!(46350), Some("2026/11/24".to_string())),
            ("E2", json!(true), None),
            ("F2", json!("007"), None),
            ("G2", json!("-x"), None),
            ("H2", json!(7.5), None),
            ("I2", json!(false), None),
        ]
    );
    apply(&mut session, r#"{"op": "set_cells", "sheet": "Sheet1", "at": "B2", "values": [[""]]}"#).unwrap();
    assert!(find(&super::inspect(session.model(), "", false), "Sheet1", "B2").is_none());
}

/// A formula that does not read is refused with its cell, before anything is entered.
#[test]
fn a_formula_that_does_not_read_is_refused() {
    let mut session = blank();
    let err = apply(&mut session, r#"[{"op": "set_cells", "sheet": "Sheet1", "at": "A1", "values": [[1]]}, {"op": "set_cells", "sheet": "Sheet1", "at": "C1", "values": [[1, "=SUM(A1:"]]}]"#).unwrap_err();
    assert_eq!(err.index, Some(1));
    assert!(err.message.starts_with("D1: the formula =SUM(A1: does not read"), "{err}");
    assert!(doc(&session).sheets[0].cells.is_empty());
    assert!(!session.document.can_undo());
}

/// A number format changes what the cell shows (`text`), not its value; a formula referring to a formatted cell takes
/// its format, as typing it does.
#[test]
fn a_number_format_is_shown_as_text() {
    let mut session = blank();
    apply(
        &mut session,
        r##"[{"op": "set_cells", "sheet": "Sheet1", "at": "A1", "values": [[1234.5, 0.256, 46350]]},
            {"op": "format_cells", "sheet": "Sheet1", "range": "A1", "format": {"number_format": "#,##0.0"}},
            {"op": "format_cells", "sheet": "Sheet1", "range": "B1", "format": {"number_format": "0.0%"}},
            {"op": "format_cells", "sheet": "Sheet1", "range": "C1", "format": {"number_format": "yyyy/mm/dd"}},
            {"op": "set_cells", "sheet": "Sheet1", "at": "D1", "values": [["=C1+1"]]}]"##,
    )
    .unwrap();
    assert_eq!(cell(&session, "Sheet1", "A1"), (json!(1234.5), None, Some("1,234.5".into())));
    assert_eq!(cell(&session, "Sheet1", "B1"), (json!(0.256), None, Some("25.6%".into())));
    assert_eq!(cell(&session, "Sheet1", "C1"), (json!(46350), None, Some("2026/11/24".into())));
    assert_eq!(cell(&session, "Sheet1", "D1"), (json!(46351), Some("=C1+1".into()), Some("2026/11/25".into())));
}

/// Sheets are added (named by `as` in the same list), renamed, moved and deleted, and each operation says where it
/// worked: the editor shows the last sheet named.
#[test]
fn sheets_are_added_renamed_moved_and_deleted() {
    let mut session = blank();
    let first = doc(&session).sheets[0].id.unwrap();
    let ops = parse_ops(
        r#"[{"op": "add_sheet", "as": "new"},
            {"op": "set_cells", "sheet": "new", "at": "A1", "values": [["売上"]]},
            {"op": "add_sheet", "name": "集計", "index": 0},
            {"op": "set_cells", "sheet": "集計", "at": "A1", "values": [["=Sheet2!A1"]]},
            {"op": "rename_sheet", "sheet": "new", "name": "Data"},
            {"op": "move_sheet", "sheet": "Data", "index": 0}]"#,
    )
    .unwrap();
    let current = session.model().clone();
    let ids = session.document.ids();
    let planned = crate::plan(&ops, &current, &session.source, &mut || ids.new_shape_id().0).unwrap();
    let new = planned.created[0]["sheet"].as_u64().unwrap();
    assert_eq!(planned.shown, Some(SlideId(new)));
    assert_eq!(planned.created[1], json!({}));
    session.document.execute(planned.commands).unwrap();
    let doc = doc(&session);
    let order: Vec<_> = doc.sheets.iter().map(|s| (s.name.as_str(), s.number)).collect();
    assert_eq!(order, [("Data", 1), ("集計", 2), ("Sheet1", 3)]);
    assert_eq!(doc.sheets[0].id, Some(new));
    // The formula follows the sheet's new name.
    assert_eq!(cell(&session, "集計", "A1"), (json!("売上"), Some("=Data!A1".into()), None));

    let created = apply(&mut session, &format!(r#"[{{"op": "delete_sheet", "sheet": {first}}}, {{"op": "delete_sheet", "sheet": "data"}}]"#)).unwrap();
    assert_eq!(created.created.len(), 2);
    let doc = super::inspect(session.model(), "", true);
    assert_eq!(doc.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["集計"]);
    assert_eq!(cell(&session, "集計", "A1").1.as_deref(), Some("=#REF!"));
    let err = apply(&mut session, r#"{"op": "delete_sheet", "sheet": "集計"}"#).unwrap_err();
    assert!(err.message.contains("the last shown sheet"), "{err}");
}

/// A sheet named wrongly is refused with the sheets there are; a name Excel does not take, with why.
#[test]
fn sheet_names_are_checked() {
    let mut session = blank();
    let id = doc(&session).sheets[0].id.unwrap();
    for (json, wanted) in [
        (r#"{"op": "set_cells", "sheet": "Sales", "at": "A1", "values": [[1]]}"#.to_string(), format!(r#"no sheet named "Sales" (the sheets: "Sheet1" (id {id}))"#)),
        (r#"{"op": "clear", "sheet": 999, "range": "A1"}"#.to_string(), "no sheet 999".to_string()),
        (r#"{"op": "add_sheet", "name": "sheet1"}"#.to_string(), r#"a sheet named "sheet1" is there already"#.to_string()),
        (r#"{"op": "rename_sheet", "sheet": "Sheet1", "name": "a/b"}"#.to_string(), "is not a sheet name".to_string()),
        (r#"{"op": "add_sheet", "index": 5}"#.to_string(), "index 5 is past the end (1 sheets)".to_string()),
        (r#"{"op": "move_sheet", "sheet": "Sheet1", "index": 1}"#.to_string(), "index 1 is past the end".to_string()),
    ] {
        let err = apply(&mut session, &json).unwrap_err();
        assert_eq!(err.index, Some(0));
        assert!(err.message.contains(&wanted), "{json}: {err}");
    }
}

/// A hidden sheet is listed, with its cells and no id, and is not edited.
#[test]
fn a_hidden_sheet_is_read_not_edited() {
    let mut session = blank();
    apply(&mut session, r#"[{"op": "add_sheet", "name": "Secret"}, {"op": "set_cells", "sheet": "Secret", "at": "A1", "values": [["x"]]}]"#).unwrap();
    let slide = session.model().slides[1].id;
    let shape = ibis_doc_model::model::ShapeId(session.document.ids().new_shape_id().0);
    let hide = ibis_doc_model::SheetCommand::HideSheet { slide, sheet: 1, hidden: true, shape };
    session.document.execute(vec![ibis_doc_model::command::Command::Sheet(hide)]).unwrap();
    let doc = doc(&session);
    assert_eq!((doc.sheets[1].id, doc.sheets[1].hidden, doc.sheets[1].number), (None, true, 2));
    assert_eq!(find(&doc, "Secret", "A1").unwrap().value, json!("x"));
    let err = apply(&mut session, r#"{"op": "clear", "sheet": "Secret", "range": "A1"}"#).unwrap_err();
    assert!(err.message.contains("\"Secret\" is hidden"), "{err}");
}

/// A protected sheet is read and said to be protected; its cells are not changed (a replacement over every sheet passes
/// it by), and the sheet itself is still renamed as Excel's protection lets it be.
#[test]
fn a_protected_sheet_is_not_changed() {
    let mut session = blank();
    apply(&mut session, r#"[{"op": "set_cells", "sheet": "Sheet1", "at": "A1", "values": [["固定"]]}, {"op": "add_sheet", "as": "two"}, {"op": "set_cells", "sheet": "two", "at": "A1", "values": [["固定"]]}]"#).unwrap();
    let slide = session.model().slides[0].id;
    let protect = ibis_doc_model::SheetCommand::SetProtection { slide, protection: Some(Default::default()) };
    session.document.execute(vec![ibis_doc_model::command::Command::Sheet(protect)]).unwrap();
    assert!(doc(&session).sheets[0].protected);
    let err = apply(&mut session, r#"{"op": "set_cells", "sheet": "Sheet1", "at": "B1", "values": [[1]]}"#).unwrap_err();
    assert!(err.message.contains("\"Sheet1\" is protected"), "{err}");
    assert_eq!(apply(&mut session, r#"{"op": "replace_text", "find": "固定", "replace": "可変"}"#).unwrap().replaced, 1);
    assert_eq!(cell(&session, "Sheet1", "A1").0, json!("固定"));
    apply(&mut session, r#"{"op": "rename_sheet", "sheet": "Sheet1", "name": "表紙"}"#).unwrap();
}

/// Rows and columns inserted and deleted move the cells, and the formulas follow them.
#[test]
fn rows_and_columns_are_inserted_and_deleted() {
    let mut session = blank();
    apply(&mut session, r#"{"op": "set_cells", "sheet": "Sheet1", "at": "A1", "values": [[1, 2], [3, 4], ["=A1+A2", "=B1*B2"]]}"#).unwrap();
    apply(
        &mut session,
        r#"[{"op": "insert_rows", "sheet": "Sheet1", "at": 2, "count": 2},
            {"op": "insert_columns", "sheet": "Sheet1", "at": "A"},
            {"op": "delete_columns", "sheet": "Sheet1", "at": "C"}]"#,
    )
    .unwrap();
    assert_eq!(cell(&session, "Sheet1", "B1").0, json!(1));
    assert_eq!(cell(&session, "Sheet1", "B4").0, json!(3));
    assert_eq!(cell(&session, "Sheet1", "B5"), (json!(4), Some("=B1+B4".into()), None));
    assert!(doc(&session).sheets[0].cells.iter().all(|c| c.at.starts_with('B')), "{:?}", doc(&session).sheets[0].cells);
    apply(&mut session, r#"{"op": "delete_rows", "sheet": "Sheet1", "at": 2, "count": 2}"#).unwrap();
    assert_eq!(cell(&session, "Sheet1", "B3"), (json!(4), Some("=B1+B2".into()), None));
    let err = apply(&mut session, r#"{"op": "delete_rows", "sheet": "Sheet1", "at": 0}"#).unwrap_err();
    assert!(err.message.contains("numbered from 1"), "{err}");
}

/// Formats, merges and sizes are set as the editor sets them, and the file saved and read again keeps them with the
/// values and formulas.
#[test]
fn formats_merges_and_sizes_save() {
    let mut session = blank();
    apply(
        &mut session,
        r##"[{"op": "set_cells", "sheet": "Sheet1", "at": "A1", "values": [["見出し", null, null], [1200, 3400, "=A2+B2"]]},
             {"op": "merge", "sheet": "Sheet1", "range": "A1:C1"},
             {"op": "format_cells", "sheet": "Sheet1", "range": "A1:C1", "format": {"bold": true, "size": 14, "color": "#FFFFFF", "fill": "#305496", "align": "center", "font": "Arial"}},
             {"op": "format_cells", "sheet": "Sheet1", "range": "A2:C2", "format": {"number_format": "#,##0", "italic": true, "underline": true, "wrap": true}},
             {"op": "set_column_width", "sheet": "Sheet1", "columns": "A:C", "width": 20},
             {"op": "set_row_height", "sheet": "Sheet1", "rows": "1", "height": 30}]"##,
    )
    .unwrap();
    let check = |session: &Session| {
        let doc = doc(session);
        assert_eq!(doc.sheets[0].merged, ["A1:C1"]);
        assert_eq!(cell(session, "Sheet1", "C2"), (json!(4600), Some("=A2+B2".into()), Some("4,600".into())));
        let sheet = content(session, 0);
        let style = |at: (u32, u32)| sheet.styles[sheet.cell(at.0, at.1).unwrap().style as usize].clone();
        let head = style((0, 0));
        assert_eq!((head.bold, head.font_size, head.color.0, head.fill.map(|f| f.0), head.align, head.font.as_deref()), (true, 1400, 0xFFFFFF, Some(0x305496), CellAlign::Center, Some("Arial")));
        let body = style((1, 2));
        assert_eq!((body.number_format.as_str(), body.italic, body.underline.enabled(), body.wrap), ("#,##0", true, true, true));
        let points = |emu: i64| emu as f64 / EMU_PER_POINT as f64;
        assert!((points(sheet.columns[1]) - 128.0).abs() < 1.0, "{}", points(sheet.columns[1]));
        assert_eq!(points(sheet.rows[0]), 30.0);
    };
    check(&session);
    let again = Session::open(session.save().unwrap(), "saved.xlsx").unwrap();
    check(&again);

    apply(&mut session, r#"[{"op": "unmerge", "sheet": "Sheet1", "range": "A1:D4"}, {"op": "format_cells", "sheet": "Sheet1", "range": "A1", "format": {"fill": null}}]"#).unwrap();
    assert!(doc(&session).sheets[0].merged.is_empty());
    assert_eq!(content(&session, 0).styles[content(&session, 0).cell(0, 0).unwrap().style as usize].fill, None);
    for (json, wanted) in [
        (r#"{"op": "unmerge", "sheet": "Sheet1", "range": "A1:C3"}"#, "no merged range within A1:C3 (the sheet's: none)"),
        (r#"{"op": "merge", "sheet": "Sheet1", "range": "B2"}"#, "is one cell"),
        (r##"{"op": "format_cells", "sheet": "Sheet1", "range": "A1", "format": {"fill": "blue"}}"##, "not a colour"),
        (r#"{"op": "format_cells", "sheet": "Sheet1", "range": "A1", "format": {}}"#, "sets nothing"),
        (r#"{"op": "format_cells", "sheet": "Sheet1", "range": "A:A", "format": {"bold": true}}"#, "is not a range of cells"),
        (r#"{"op": "set_column_width", "sheet": "Sheet1", "columns": "A", "width": 300}"#, "not 0 to 255"),
        (r#"{"op": "set_row_height", "sheet": "Sheet1", "rows": "5000", "height": 20}"#, "past them"),
    ] {
        let err = apply(&mut session, json).unwrap_err();
        assert!(err.message.contains(wanted), "{json}: {err}");
    }
}

/// Clear empties the values and formulas of a range, and leaves its formats.
#[test]
fn clear_takes_the_contents_only() {
    let mut session = blank();
    apply(&mut session, r#"[{"op": "set_cells", "sheet": "Sheet1", "at": "A1", "values": [[1, "=A1*2", "x"]]}, {"op": "format_cells", "sheet": "Sheet1", "range": "A1:C1", "format": {"bold": true}}]"#).unwrap();
    apply(&mut session, r#"{"op": "clear", "sheet": "Sheet1", "range": "B1:A1"}"#).unwrap();
    let doc = doc(&session);
    assert_eq!(doc.sheets[0].cells.iter().map(|c| c.at.as_str()).collect::<Vec<_>>(), ["C1"]);
    let sheet = content(&session, 0);
    assert!(sheet.styles[sheet.cell(0, 0).unwrap().style as usize].bold);
}

/// Replace text goes through the text cells of every shown sheet (not formulas, nor numbers), and says how many it
/// replaced; none found is an error the agent can read.
#[test]
fn replace_text_in_text_cells() {
    let mut session = blank();
    apply(
        &mut session,
        r#"[{"op": "set_cells", "sheet": "Sheet1", "at": "A1", "values": [["A社 A社", "=\"A社\"", 1]]},
            {"op": "add_sheet", "as": "two"},
            {"op": "set_cells", "sheet": "two", "at": "B2", "values": [["A社御中"]]}]"#,
    )
    .unwrap();
    let created = apply(&mut session, r#"{"op": "replace_text", "find": "A社", "replace": "イビス"}"#).unwrap();
    assert_eq!(created.replaced, 3);
    assert_eq!(cell(&session, "Sheet1", "A1").0, json!("イビス イビス"));
    assert_eq!(cell(&session, "Sheet1", "B1"), (json!("A社"), Some("=\"A社\"".into()), None));
    assert_eq!(cell(&session, "Sheet2", "B2").0, json!("イビス御中"));
    let err = apply(&mut session, r#"{"op": "replace_text", "find": "A社", "replace": "x", "sheet": "Sheet2"}"#).unwrap_err();
    assert!(err.message.contains("not in the text of any cell"), "{err}");
}

/// A big sheet lists its first cells, the first rows first, and says how many more there are.
#[test]
fn a_big_sheet_lists_its_first_cells() {
    let mut session = blank();
    let rows: Vec<Vec<u32>> = (0..210).map(|r| (0..10).map(|c| r * 10 + c).collect()).collect();
    apply(&mut session, &json!({"op": "set_cells", "sheet": "Sheet1", "at": "A1", "values": rows}).to_string()).unwrap();
    let doc = doc(&session);
    let sheet = &doc.sheets[0];
    assert_eq!((sheet.cells.len(), sheet.more, sheet.used.as_deref()), (CELLS_PER_SHEET, Some(2100 - CELLS_PER_SHEET), Some("A1:J210")));
    assert_eq!(sheet.cells.last().unwrap().at, "J200");
    let small = super::inspect(blank().model(), "", false);
    assert_eq!(serde_json::to_value(&small).unwrap()["sheets"][0].get("more"), None);
}

/// The ids are those a file is read with: the same bytes read again give the same ids, so a summary of a file names
/// what the command-line tool's next run of it edits.
#[test]
fn reading_the_same_file_gives_the_same_ids() {
    let mut session = blank();
    apply(&mut session, r#"[{"op": "add_sheet", "name": "B"}, {"op": "add_sheet", "name": "C", "index": 0}, {"op": "set_cells", "sheet": "B", "at": "A1", "values": [[1]]}]"#).unwrap();
    let bytes = session.save().unwrap();
    let a = Session::open(bytes.clone(), "a.xlsx").unwrap().inspect().into_workbook();
    let b = Session::open(bytes, "a.xlsx").unwrap().inspect().into_workbook();
    assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());
    assert_eq!(a.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["C", "Sheet1", "B"]);
    let ids: std::collections::HashSet<_> = a.sheets.iter().map(|s| s.id.unwrap()).collect();
    assert_eq!(ids.len(), 3);
}

#[test]
fn unknown_operations_and_fields_are_refused_by_name() {
    assert!(parse_ops(r#"[{"op": "explode"}]"#).unwrap_err().contains("operation 0"));
    let err = parse_ops(r#"{"op": "clear", "sheet": 1, "range": "A1", "colour": "red"}"#).unwrap_err();
    assert!(err.contains("colour"), "{err}");
    let err = parse_ops(r#"{"op": "format_cells", "sheet": 1, "range": "A1", "format": {"strike": true}}"#).unwrap_err();
    assert!(err.contains("strike"), "{err}");
}

/// Operations on another kind of file are refused as such.
#[test]
fn workbook_operations_are_for_a_workbook() {
    let mut deck = Session::open(crate::blank("deck.pptx").unwrap(), "deck.pptx").unwrap();
    let err = deck.apply(&parse_ops(r#"{"op": "add_sheet"}"#).unwrap()).unwrap_err();
    assert!(err.message.contains("operations on a workbook, and the file is a presentation"), "{err}");
}
