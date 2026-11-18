// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::*;

fn blank() -> Session {
    Session::open(ibis_pptx::blank_file(ibis_pptx::Kind::Presentation).unwrap(), "new.pptx").unwrap()
}

fn apply(session: &mut Session, json: &str) -> Result<Vec<Created>, OpError> {
    session.apply(&parse_ops(json).unwrap())
}

fn texts(session: &Session) -> Vec<Vec<String>> {
    session.inspect().slides.iter().map(|s| s.shapes.iter().filter_map(|sh| sh.text.clone()).collect()).collect()
}

/// The new presentation's title slide, its layouts and its empty placeholders, as the agent reads them.
#[test]
fn inspect_names_slides_shapes_and_layouts() {
    let session = blank();
    let doc = session.inspect();
    assert_eq!(doc.slides.len(), 1);
    assert!(doc.layouts.contains(&"タイトルとコンテンツ".to_string()), "{:?}", doc.layouts);
    let kinds: Vec<_> = doc.slides[0].shapes.iter().filter_map(|s| s.placeholder.clone()).collect();
    assert!(kinds.contains(&"ctrTitle".to_string()), "{kinds:?}");
    assert_eq!(doc.slide_size.w, 960.0);
    let json = serde_json::to_string(&doc).unwrap();
    assert!(json.contains("\"placeholder\":\"ctrTitle\""), "{json}");
}

/// A slide made, its placeholders filled and a table put on it, in one list: one undo takes it all away, and the file
/// saved and read again has it.
#[test]
fn a_slide_made_and_filled_is_one_undo_and_saves() {
    let mut session = blank();
    let before = session.presentation().clone();
    let created = apply(
        &mut session,
        r#"[
            {"op": "add_slide", "layout": "タイトルとコンテンツ", "as": "agenda"},
            {"op": "set_text", "slide": "agenda", "shape": {"placeholder": "title"}, "text": "本日の議題"},
            {"op": "set_text", "slide": "agenda", "shape": {"placeholder": "body"}, "text": "背景\n提案\n予定"},
            {"op": "add_table", "slide": "agenda", "frame": {"x": 500, "y": 300, "w": 400, "h": 100}, "rows": [["項目", "期日"], ["試作", "12月"]], "as": "t"},
            {"op": "set_cell", "slide": "agenda", "shape": "t", "row": 1, "column": 1, "text": "11月"}
        ]"#,
    )
    .unwrap();
    assert_eq!(created.len(), 5);
    let slide = created[0].slide.unwrap();
    assert_eq!(created[3].slide, Some(slide));
    let doc = session.inspect();
    assert_eq!(doc.slides.len(), 2);
    assert_eq!(doc.slides[1].layout.as_deref(), Some("タイトルとコンテンツ"));
    assert_eq!(texts(&session)[1][..2], ["本日の議題".to_string(), "背景\n提案\n予定".to_string()]);
    let table = doc.slides[1].shapes.iter().find(|s| s.kind == "table").unwrap();
    assert_eq!(table.cells.as_ref().unwrap()[1], ["試作", "11月"]);
    assert!(doc.dirty);

    let saved = session.save().unwrap();
    let again = Session::open(saved, "saved.pptx").unwrap();
    assert_eq!(texts(&again)[1][..2], texts(&session)[1][..2]);
    let table = again.inspect().slides[1].shapes.iter().find(|s| s.kind == "table").unwrap().cells.clone().unwrap();
    assert_eq!(table, vec![vec!["項目", "期日"], vec!["試作", "11月"]]);

    assert!(session.document.undo());
    assert_eq!(session.presentation().slides, before.slides);
    assert!(!session.document.can_undo());
}

/// An operation that fails refuses the whole list: what the ones before it did is not applied either.
#[test]
fn a_failing_operation_changes_nothing() {
    let mut session = blank();
    let before = session.presentation().clone();
    let err = apply(&mut session, r#"[{"op": "add_slide"}, {"op": "set_text", "slide": 999, "shape": 1, "text": "x"}]"#).unwrap_err();
    assert_eq!(err.index, Some(1));
    assert!(err.message.contains("no slide 999"), "{err}");
    assert_eq!(session.presentation().slides, before.slides);
    assert!(!session.document.can_undo());
}

#[test]
fn unknown_operations_and_fields_are_refused_by_name() {
    assert!(parse_ops(r#"[{"op": "explode"}]"#).unwrap_err().contains("operation 0"));
    let err = parse_ops(r#"{"op": "delete_slide", "slide": 1, "colour": "red"}"#).unwrap_err();
    assert!(err.contains("colour"), "{err}");
}

/// Set text keeps each paragraph's format: a title made bold stays bold when its words change.
#[test]
fn set_text_keeps_the_format_of_what_it_replaces() {
    let mut session = blank();
    let slide = session.inspect().slides[0].id;
    apply(&mut session, &format!(r##"[{{"op": "set_text", "slide": {slide}, "shape": {{"placeholder": "title"}}, "text": "旧"}}, {{"op": "format_text", "slide": {slide}, "shape": {{"placeholder": "title"}}, "format": {{"bold": true, "size": 40, "color": "#C00000"}}}}]"##)).unwrap();
    apply(&mut session, &format!(r#"{{"op": "set_text", "slide": {slide}, "shape": {{"placeholder": "title"}}, "text": "新しい見出し"}}"#)).unwrap();
    let title = session.presentation().slides[0].shapes.iter().find(|s| s.text_body().is_some_and(|b| b.text() == "新しい見出し")).unwrap();
    let run = &title.text_body().unwrap().paragraphs[0].runs[0];
    assert_eq!((run.style.bold, run.style.font_size, run.style.color), (Some(true), Some(4000), Some(ibis_doc_model::drawing::Rgb(0xC00000))));
    assert_eq!(session.inspect().slides[0].shapes.iter().find(|s| s.text.as_deref() == Some("新しい見出し")).unwrap().font_size, Some(40.0));
}

/// Replace text goes through shapes and table cells of every slide, and says how many it replaced; none found is an
/// error the agent can read.
#[test]
fn replace_text_in_shapes_and_cells() {
    let mut session = blank();
    let slide = session.inspect().slides[0].id;
    apply(
        &mut session,
        &format!(
            r#"[{{"op": "set_text", "slide": {slide}, "shape": {{"placeholder": "title"}}, "text": "A社の提案 A社"}},
                {{"op": "add_table", "slide": {slide}, "rows": [["A社", "B社"]]}}]"#
        ),
    )
    .unwrap();
    let created = apply(&mut session, r#"{"op": "replace_text", "find": "A社", "replace": "イビス"}"#).unwrap();
    assert_eq!(created[0].replaced, Some(3));
    let doc = session.inspect();
    assert!(doc.slides[0].shapes.iter().any(|s| s.text.as_deref() == Some("イビスの提案 イビス")));
    assert_eq!(doc.slides[0].shapes.iter().find(|s| s.kind == "table").unwrap().cells.as_ref().unwrap()[0], ["イビス", "B社"]);
    let err = apply(&mut session, r#"{"op": "replace_text", "find": "A社", "replace": "x"}"#).unwrap_err();
    assert!(err.message.contains("not in the text"), "{err}");
}

/// Slides are added, moved, copied and deleted; shapes added, moved, filled, outlined and deleted; all of it saves.
#[test]
fn slide_and_shape_operations_save() {
    let mut session = blank();
    let first = session.inspect().slides[0].id;
    let created = apply(
        &mut session,
        &format!(
            r##"[{{"op": "add_slide", "layout": "白紙", "as": "b"}},
                {{"op": "add_shape", "slide": "b", "geometry": "roundRect", "frame": {{"x": 100, "y": 100, "w": 200, "h": 80}}, "fill": "#FFC000", "text": "要点", "as": "box"}},
                {{"op": "add_text_box", "slide": "b", "frame": {{"x": 100, "y": 300, "w": 400, "h": 40}}, "text": "注記", "format": {{"italic": true}}}},
                {{"op": "set_frame", "slide": "b", "shape": "box", "x": 120, "w": 240}},
                {{"op": "set_line", "slide": "b", "shape": "box", "color": "#000000", "width": 2}},
                {{"op": "duplicate_slide", "slide": {first}, "as": "copy"}},
                {{"op": "set_background", "slide": {first}, "color": "#F2F2F2"}},
                {{"op": "move_slide", "slide": "b", "index": 0}},
                {{"op": "format_paragraphs", "slide": "b", "shape": "box", "format": {{"align": "right"}}}}]"##
        ),
    )
    .unwrap();
    let doc = session.inspect();
    assert_eq!(doc.slides.len(), 3);
    assert_eq!(doc.slides[0].id, created[0].slide.unwrap());
    let shape = doc.slides[0].shapes.iter().find(|s| s.id == created[1].shape.unwrap()).unwrap();
    assert_eq!((shape.x, shape.y, shape.w, shape.h), (120.0, 100.0, 240.0, 80.0));
    assert_eq!((shape.geometry.as_deref(), shape.fill.as_deref()), (Some("roundRect"), Some("#FFC000")));
    assert_eq!(doc.slides[1].background.as_deref(), Some("#F2F2F2"));

    let again = Session::open(session.save().unwrap(), "saved.pptx").unwrap();
    let reread = again.inspect();
    assert_eq!(reread.slides.len(), 3);
    let texts: Vec<_> = reread.slides[0].shapes.iter().filter_map(|s| s.text.clone()).collect();
    assert_eq!(texts, ["要点", "注記"]);

    let delete = format!(r#"[{{"op": "delete_shape", "slide": {}, "shape": {}}}, {{"op": "delete_slide", "slide": {first}}}]"#, created[0].slide.unwrap(), created[1].shape.unwrap());
    apply(&mut session, &delete).unwrap();
    assert_eq!(session.inspect().slides.len(), 2);
    assert_eq!(session.inspect().slides[0].shapes.len(), 1);
}

#[test]
fn table_rows_are_inserted_and_deleted() {
    let mut session = blank();
    let slide = session.inspect().slides[0].id;
    let created = apply(&mut session, &format!(r#"{{"op": "add_table", "slide": {slide}, "rows": [["a", "b"], ["c", "d"]]}}"#)).unwrap();
    let table = created[0].shape.unwrap();
    apply(&mut session, &format!(r#"[{{"op": "insert_table_row", "slide": {slide}, "shape": {table}, "at": 1}}, {{"op": "set_cell", "slide": {slide}, "shape": {table}, "row": 1, "column": 0, "text": "new"}}]"#)).unwrap();
    let cells = |s: &Session| s.inspect().slides[0].shapes.iter().find(|s| s.kind == "table").unwrap().cells.clone().unwrap();
    assert_eq!(cells(&session), vec![vec!["a", "b"], vec!["new", ""], vec!["c", "d"]]);
    apply(&mut session, &format!(r#"{{"op": "delete_table_row", "slide": {slide}, "shape": {table}, "at": 0}}"#)).unwrap();
    assert_eq!(cells(&session), vec![vec!["new", ""], vec!["c", "d"]]);
    session.save().unwrap();
}

/// The ids are those a file is read with: the same bytes read again give the same ids, so a summary of a file names
/// what the command-line tool's next run of it edits.
#[test]
fn reading_the_same_file_gives_the_same_ids() {
    let mut session = blank();
    apply(&mut session, r#"{"op": "add_slide", "layout": "タイトルとコンテンツ"}"#).unwrap();
    let bytes = session.save().unwrap();
    let a = Session::open(bytes.clone(), "a.pptx").unwrap().inspect();
    let b = Session::open(bytes, "a.pptx").unwrap().inspect();
    assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());
}

/// What the file cannot be saved with yet is refused before it is applied (the editor would only find it at the save):
/// here a background on a slide that is new, and notes in a file with no notes master.
#[test]
fn a_change_the_file_cannot_save_is_refused() {
    let mut session = blank();
    let before = session.presentation().slides.clone();
    let first = before[0].id.0;
    for json in [
        r##"[{"op": "add_slide", "as": "n"}, {"op": "set_background", "slide": "n", "color": "#000000"}]"##.to_string(),
        format!(r#"{{"op": "set_notes", "slide": {first}, "text": "x"}}"#),
    ] {
        let err = apply(&mut session, &json).unwrap_err();
        assert_eq!(err.index, None);
        assert!(err.message.starts_with("the file cannot be saved with this change"), "{err}");
        assert_eq!(session.presentation().slides, before);
    }
}

#[test]
fn bad_values_are_refused_with_what_is_wrong() {
    let mut session = blank();
    let slide = session.inspect().slides[0].id;
    for (json, wanted) in [
        (format!(r#"{{"op": "set_background", "slide": {slide}, "color": "red"}}"#), "not a colour"),
        (format!(r#"{{"op": "add_shape", "slide": {slide}, "geometry": "blob", "frame": {{"x": 0, "y": 0, "w": 1, "h": 1}}}}"#), "no preset shape"),
        (format!(r#"{{"op": "add_text_box", "slide": {slide}, "frame": {{"x": 0, "y": 0, "w": 0, "h": 1}}, "text": "x"}}"#), "width and a height"),
        (r#"{"op": "add_slide", "layout": "無い"}"#.to_string(), "no layout named"),
        (format!(r#"{{"op": "set_text", "slide": {slide}, "shape": {{"placeholder": "pic"}}, "text": "x"}}"#), "no pic placeholder"),
    ] {
        let err = apply(&mut session, &json).unwrap_err();
        assert!(err.message.contains(wanted), "{json}: {err}");
    }
}
