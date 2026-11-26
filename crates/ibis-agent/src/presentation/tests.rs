// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_doc_model::command::Command;
use ibis_doc_model::drawing::Rgb;
use ibis_doc_model::geometry::EmuRect;
use ibis_doc_model::model::{Shape, ShapeId, ShapeKind};
use ibis_doc_model::text::{Run, RunStyle, TextBody, TextField, TextStyle};

use super::{FieldName, FieldPiece, ParagraphText, Piece};
use crate::{Applied, OpError, Ops, Session};

fn blank() -> Session {
    Session::open(crate::blank("new.pptx").unwrap(), "new.pptx").unwrap()
}

/// A presentation of two slides whose second has a text box `ページ <slide number>` (its number bold red) and one
/// `作成日 <date>`, as read back from its file.
fn with_fields() -> (Session, u64, u64, u64) {
    let mut session = blank();
    let first = session.inspect().into_presentation().slides[0].id;
    apply(&mut session, &format!(r#"{{"op": "duplicate_slide", "slide": {first}}}"#)).unwrap();
    let slide = session.model().slides[1].id;
    let number = RunStyle { field: Some(TextField::SlideNumber), bold: Some(true), color: Some(Rgb(0xC00000)), ..Default::default() };
    let date = RunStyle { field: Some(TextField::DateTime { format: 4, east_asian: true }), ..Default::default() };
    let mut page = TextBody::plain("ページ ", TextStyle::default());
    page.paragraphs[0].runs.push(Run { text: "2".into(), style: number });
    let mut made = TextBody::plain("作成日 ", TextStyle::default());
    made.paragraphs[0].runs.push(Run { text: "2026年10月1日".into(), style: date });
    let ids = session.document.ids();
    let (a, b) = (ids.new_shape_id(), ids.new_shape_id());
    let frame = EmuRect::new(1_000_000, 1_000_000, 2_000_000, 400_000);
    session
        .document
        .execute(vec![
            Command::InsertShape { slide, index: 0, shape: Shape::new(a, frame, ShapeKind::TextBox(page)) },
            Command::InsertShape { slide, index: 1, shape: Shape::new(b, frame, ShapeKind::TextBox(made)) },
        ])
        .unwrap();
    let session = Session::open(session.save().unwrap(), "fields.pptx").unwrap();
    let doc = session.inspect().into_presentation();
    let slide = &doc.slides[1];
    let id = |start: &str| slide.shapes.iter().find(|s| serde_json::to_string(&s.text).unwrap().contains(start)).unwrap().id;
    let (page, made) = (id("ページ"), id("作成日"));
    (session, slide.id, page, made)
}

fn field(field: FieldName) -> Piece {
    Piece::Field(FieldPiece { field })
}

fn text_of(session: &Session, slide: u64, shape: u64) -> Vec<ParagraphText> {
    let doc = session.inspect().into_presentation();
    doc.slides.iter().find(|s| s.id == slide).unwrap().shapes.iter().find(|s| s.id == shape).unwrap().text.clone().unwrap()
}

fn runs(session: &Session, shape: u64) -> Vec<Run> {
    session.model().slides[1].shape(ShapeId(shape)).unwrap().text_body().unwrap().paragraphs[0].runs.clone()
}

/// The runs of the paragraph's field, and the text around it.
fn split(runs: &[Run]) -> (Vec<Run>, String) {
    let (field, words): (Vec<Run>, Vec<Run>) = runs.iter().cloned().partition(|r| r.style.field.is_some());
    (field, words.iter().map(|r| r.text.as_str()).collect())
}

/// A field reads as itself among the paragraph's words, and given back with other words it stays the field it was: the
/// number follows the slide, and keeps its own format.
#[test]
fn a_field_is_read_as_itself_and_kept_when_the_words_change() {
    let (mut session, slide, page, made) = with_fields();
    assert_eq!(text_of(&session, slide, page), vec![ParagraphText::Pieces(vec![Piece::Text("ページ ".into()), field(FieldName::SlideNumber)])]);
    let json = serde_json::to_string(&text_of(&session, slide, made)).unwrap();
    assert_eq!(json, r#"[["作成日 ",{"field":"date"}]]"#);

    let (number, _) = split(&runs(&session, page));
    let (date, _) = split(&runs(&session, made));
    let before = session.model().slides.clone();
    let given_back = serde_json::to_string(&text_of(&session, slide, page)).unwrap();
    apply(&mut session, &format!(r#"{{"op": "set_text", "slide": {slide}, "shape": {page}, "text": {given_back}}}"#)).unwrap();
    assert_eq!(session.model().slides, before);

    apply(
        &mut session,
        &format!(
            r#"[{{"op": "set_text", "slide": {slide}, "shape": {page}, "text": [["Page ", {{"field": "slide_number"}}, " of the deck"]]}},
                {{"op": "set_text", "slide": {slide}, "shape": {made}, "text": [["Made ", {{"field": "date"}}]]}}]"#
        ),
    )
    .unwrap();
    let page_runs = runs(&session, page);
    assert_eq!(page_runs.iter().map(|r| r.text.as_str()).collect::<String>(), "Page 2 of the deck");
    assert_eq!(page_runs[1..1 + number.len()], number[..]);
    assert_eq!(number[0].style.bold, Some(true));
    assert_eq!(split(&runs(&session, made)), (date.clone(), "Made ".to_string()));
    assert_eq!(date[0].style.field, Some(TextField::DateTime { format: 4, east_asian: true }));

    apply(&mut session, &format!(r#"{{"op": "move_slide", "slide": {slide}, "index": 0}}"#)).unwrap();
    let saved = Session::open(session.save().unwrap(), "saved.pptx").unwrap();
    let body = saved.model().slides[0].shapes.iter().filter_map(|s| s.text_body()).find(|b| b.text().starts_with("Page")).unwrap();
    assert_eq!(body.text(), "Page 1 of the deck");
    assert_eq!(split(&body.paragraphs[0].runs).0.iter().map(|r| r.text.as_str()).collect::<String>(), "1");
}

/// Plain words over a paragraph that had a field leave the field out, as asked; a field the text never had is refused,
/// as is one in text that keeps none (notes, a shape being made).
#[test]
fn a_field_is_kept_and_never_made() {
    let (mut session, slide, page, _) = with_fields();
    apply(&mut session, &format!(r#"{{"op": "set_text", "slide": {slide}, "shape": {page}, "text": ["Page two"]}}"#)).unwrap();
    assert!(runs(&session, page).iter().all(|r| r.style.field.is_none()));

    let err = apply(&mut session, &format!(r#"{{"op": "set_text", "slide": {slide}, "shape": {page}, "text": [["Page ", {{"field": "slide_number"}}]]}}"#)).unwrap_err();
    assert!(err.message.contains("no slide_number field to keep"), "{err}");
    let err = apply(&mut session, &format!(r#"{{"op": "set_notes", "slide": {slide}, "text": [["p. ", {{"field": "slide_number"}}]]}}"#)).unwrap_err();
    assert!(err.message.contains("has no field to keep"), "{err}");
    let err = parse_ops(&format!(r#"{{"op": "set_text", "slide": {slide}, "shape": {page}, "text": [[{{"field": "page"}}]]}}"#)).unwrap_err();
    assert!(err.contains("operation 0"), "{err}");
}

/// Replace text changes the words around a field and never the number or date it shows.
#[test]
fn replace_text_leaves_a_field_alone() {
    let (mut session, slide, page, made) = with_fields();
    apply(&mut session, r#"{"op": "replace_text", "find": "ページ", "replace": "Page"}"#).unwrap();
    assert_eq!(text_of(&session, slide, page), vec![ParagraphText::Pieces(vec![Piece::Text("Page ".into()), field(FieldName::SlideNumber)])]);
    let err = apply(&mut session, r#"{"op": "replace_text", "find": "2026年", "replace": "x"}"#).unwrap_err();
    assert!(err.message.contains("not in the text"), "{err}");
    assert_eq!(split(&runs(&session, made)).0.iter().map(|r| r.text.as_str()).collect::<String>(), "2026年10月1日");
}

fn parse_ops(json: &str) -> Result<Ops, String> {
    crate::parse_ops(ibis_pptx::Kind::Presentation, json)
}

fn apply(session: &mut Session, json: &str) -> Result<Applied, OpError> {
    session.apply(&parse_ops(json).unwrap())
}

fn texts(session: &Session) -> Vec<Vec<Vec<ParagraphText>>> {
    session.inspect().into_presentation().slides.iter().map(|s| s.shapes.iter().filter_map(|sh| sh.text.clone()).collect()).collect()
}

fn paragraphs(list: &[&str]) -> Vec<ParagraphText> {
    list.iter().map(|p| ParagraphText::Plain(p.to_string())).collect()
}

/// The new presentation's title slide, its layouts and its empty placeholders, as the agent reads them.
#[test]
fn inspect_names_slides_shapes_and_layouts() {
    let session = blank();
    let doc = session.inspect().into_presentation();
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
    let before = session.model().clone();
    let created = apply(
        &mut session,
        r#"[
            {"op": "add_slide", "layout": "タイトルとコンテンツ", "as": "agenda"},
            {"op": "set_text", "slide": "agenda", "shape": {"placeholder": "title"}, "text": ["本日の議題"]},
            {"op": "set_text", "slide": "agenda", "shape": {"placeholder": "body"}, "text": ["背景", "提案", "予定"]},
            {"op": "add_table", "slide": "agenda", "frame": {"x": 500, "y": 300, "w": 400, "h": 100}, "rows": [[["項目"], ["期日"]], [["試作"], ["12月"]]], "as": "t"},
            {"op": "set_cell", "slide": "agenda", "shape": "t", "row": 1, "column": 1, "text": ["11月"]}
        ]"#,
    )
    .unwrap();
    assert_eq!(created.created.len(), 5);
    let slide = created.created[0]["slide"].as_u64().unwrap();
    assert_eq!(created.created[3]["slide"].as_u64(), Some(slide));
    let doc = session.inspect().into_presentation();
    assert_eq!(doc.slides.len(), 2);
    assert_eq!(doc.slides[1].layout.as_deref(), Some("タイトルとコンテンツ"));
    assert_eq!(texts(&session)[1][..2], [paragraphs(&["本日の議題"]), paragraphs(&["背景", "提案", "予定"])]);
    let table = doc.slides[1].shapes.iter().find(|s| s.kind == "table").unwrap();
    assert_eq!(table.cells.as_ref().unwrap()[1], [paragraphs(&["試作"]), paragraphs(&["11月"])]);
    assert!(doc.dirty);

    let saved = session.save().unwrap();
    let again = Session::open(saved, "saved.pptx").unwrap();
    assert_eq!(texts(&again)[1][..2], texts(&session)[1][..2]);
    let table = again.inspect().into_presentation().slides[1].shapes.iter().find(|s| s.kind == "table").unwrap().cells.clone().unwrap();
    assert_eq!(table, vec![vec![paragraphs(&["項目"]), paragraphs(&["期日"])], vec![paragraphs(&["試作"]), paragraphs(&["11月"])]]);

    assert!(session.document.undo());
    assert_eq!(session.model().slides, before.slides);
    assert!(!session.document.can_undo());
}

/// Text goes in as the paragraphs it is read out as: what inspect gives, given back, changes nothing, and a cell's
/// paragraphs are a cell's too.
#[test]
fn text_is_read_and_written_as_the_same_paragraphs() {
    let mut session = blank();
    let slide = session.inspect().into_presentation().slides[0].id;
    apply(
        &mut session,
        &format!(
            r#"[{{"op": "set_text", "slide": {slide}, "shape": {{"placeholder": "subTitle"}}, "text": ["Empowering teams", "", "[Date / Presenter]"]}},
                {{"op": "add_table", "slide": {slide}, "rows": [[["one", "two"]]]}}]"#
        ),
    )
    .unwrap();
    let doc = session.inspect().into_presentation();
    let subtitle = doc.slides[0].shapes.iter().find(|s| s.placeholder.as_deref() == Some("subTitle")).unwrap();
    assert_eq!(subtitle.text.clone().unwrap(), paragraphs(&["Empowering teams", "", "[Date / Presenter]"]));
    let body = session.model().slides[0].shapes.iter().find(|s| s.id.0 == subtitle.id).unwrap().text_body().unwrap();
    assert_eq!(body.paragraphs.len(), 3);
    assert_eq!(doc.slides[0].shapes.iter().find(|s| s.kind == "table").unwrap().cells.as_ref().unwrap()[0][0], paragraphs(&["one", "two"]));

    let json = serde_json::to_string(&subtitle.text).unwrap();
    let before = session.model().slides.clone();
    apply(&mut session, &format!(r#"{{"op": "set_text", "slide": {slide}, "shape": {}, "text": {json}}}"#, subtitle.id)).unwrap();
    assert_eq!(session.model().slides, before);
}

/// A paragraph holds no line break: one is refused with where it is, so a paragraph is never split by a character
/// the agent wrote, nor a written `\n` taken for one.
#[test]
fn a_line_break_in_a_paragraph_is_refused() {
    let err = parse_ops(r#"{"op": "set_text", "slide": 1, "shape": 1, "text": ["a", "b\nc"]}"#).unwrap_err();
    assert!(err.contains("paragraph 1 has a line break"), "{err}");
    let err = parse_ops(r#"{"op": "set_text", "slide": 1, "shape": 1, "text": "a"}"#).unwrap_err();
    assert!(err.contains("operation 0"), "{err}");
}

/// The document's revision changes with an edit, its undo and an edit after it, where the undo stack's length does not.
#[test]
fn the_revision_tells_every_change() {
    let mut session = blank();
    let mut seen = vec![session.document.revision()];
    apply(&mut session, r#"{"op": "add_slide"}"#).unwrap();
    seen.push(session.document.revision());
    session.document.undo();
    seen.push(session.document.revision());
    apply(&mut session, r#"{"op": "add_slide"}"#).unwrap();
    seen.push(session.document.revision());
    let distinct: std::collections::HashSet<_> = seen.iter().collect();
    assert_eq!(distinct.len(), seen.len(), "{seen:?}");
}

/// An operation that fails refuses the whole list: what the ones before it did is not applied either.
#[test]
fn a_failing_operation_changes_nothing() {
    let mut session = blank();
    let before = session.model().clone();
    let err = apply(&mut session, r#"[{"op": "add_slide"}, {"op": "set_text", "slide": 999, "shape": 1, "text": ["x"]}]"#).unwrap_err();
    assert_eq!(err.index, Some(1));
    assert!(err.message.contains("no slide 999"), "{err}");
    assert_eq!(session.model().slides, before.slides);
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
    let slide = session.inspect().into_presentation().slides[0].id;
    apply(&mut session, &format!(r##"[{{"op": "set_text", "slide": {slide}, "shape": {{"placeholder": "title"}}, "text": ["旧"]}}, {{"op": "format_text", "slide": {slide}, "shape": {{"placeholder": "title"}}, "format": {{"bold": true, "size": 40, "color": "#C00000"}}}}]"##)).unwrap();
    apply(&mut session, &format!(r#"{{"op": "set_text", "slide": {slide}, "shape": {{"placeholder": "title"}}, "text": ["新しい見出し"]}}"#)).unwrap();
    let title = session.model().slides[0].shapes.iter().find(|s| s.text_body().is_some_and(|b| b.text() == "新しい見出し")).unwrap();
    let run = &title.text_body().unwrap().paragraphs[0].runs[0];
    assert_eq!((run.style.bold, run.style.font_size, run.style.color), (Some(true), Some(4000), Some(ibis_doc_model::drawing::Rgb(0xC00000))));
    assert_eq!(session.inspect().into_presentation().slides[0].shapes.iter().find(|s| s.text == Some(paragraphs(&["新しい見出し"]))).unwrap().font_size, Some(40.0));
}

/// Replace text goes through shapes and table cells of every slide, and says how many it replaced; none found is an
/// error the agent can read.
#[test]
fn replace_text_in_shapes_and_cells() {
    let mut session = blank();
    let slide = session.inspect().into_presentation().slides[0].id;
    apply(
        &mut session,
        &format!(
            r#"[{{"op": "set_text", "slide": {slide}, "shape": {{"placeholder": "title"}}, "text": ["A社の提案 A社"]}},
                {{"op": "add_table", "slide": {slide}, "rows": [[["A社"], ["B社"]]]}}]"#
        ),
    )
    .unwrap();
    let created = apply(&mut session, r#"{"op": "replace_text", "find": "A社", "replace": "イビス"}"#).unwrap();
    assert_eq!(created.replaced, 3);
    let doc = session.inspect().into_presentation();
    assert!(doc.slides[0].shapes.iter().any(|s| s.text == Some(paragraphs(&["イビスの提案 イビス"]))));
    assert_eq!(doc.slides[0].shapes.iter().find(|s| s.kind == "table").unwrap().cells.as_ref().unwrap()[0], [paragraphs(&["イビス"]), paragraphs(&["B社"])]);
    let err = apply(&mut session, r#"{"op": "replace_text", "find": "A社", "replace": "x"}"#).unwrap_err();
    assert!(err.message.contains("not in the text"), "{err}");
}

/// Slides are added, moved, copied and deleted; shapes added, moved, filled, outlined and deleted; all of it saves.
#[test]
fn slide_and_shape_operations_save() {
    let mut session = blank();
    let first = session.inspect().into_presentation().slides[0].id;
    let created = apply(
        &mut session,
        &format!(
            r##"[{{"op": "add_slide", "layout": "白紙", "as": "b"}},
                {{"op": "add_shape", "slide": "b", "geometry": "roundRect", "frame": {{"x": 100, "y": 100, "w": 200, "h": 80}}, "fill": "#FFC000", "text": ["要点"], "as": "box"}},
                {{"op": "add_text_box", "slide": "b", "frame": {{"x": 100, "y": 300, "w": 400, "h": 40}}, "text": ["注記", "補足"], "format": {{"italic": true}}}},
                {{"op": "set_frame", "slide": "b", "shape": "box", "x": 120, "w": 240}},
                {{"op": "set_line", "slide": "b", "shape": "box", "color": "#000000", "width": 2}},
                {{"op": "duplicate_slide", "slide": {first}, "as": "copy"}},
                {{"op": "set_background", "slide": {first}, "color": "#F2F2F2"}},
                {{"op": "move_slide", "slide": "b", "index": 0}},
                {{"op": "format_paragraphs", "slide": "b", "shape": "box", "format": {{"align": "right"}}}}]"##
        ),
    )
    .unwrap();
    let doc = session.inspect().into_presentation();
    assert_eq!(doc.slides.len(), 3);
    assert_eq!(doc.slides[0].id, created.created[0]["slide"].as_u64().unwrap());
    let shape = doc.slides[0].shapes.iter().find(|s| s.id == created.created[1]["shape"].as_u64().unwrap()).unwrap();
    assert_eq!((shape.x, shape.y, shape.w, shape.h), (120.0, 100.0, 240.0, 80.0));
    assert_eq!((shape.geometry.as_deref(), shape.fill.as_deref()), (Some("roundRect"), Some("#FFC000")));
    assert_eq!(doc.slides[1].background.as_deref(), Some("#F2F2F2"));

    let again = Session::open(session.save().unwrap(), "saved.pptx").unwrap();
    let reread = again.inspect().into_presentation();
    assert_eq!(reread.slides.len(), 3);
    let texts: Vec<_> = reread.slides[0].shapes.iter().filter_map(|s| s.text.clone()).collect();
    assert_eq!(texts, [paragraphs(&["要点"]), paragraphs(&["注記", "補足"])]);

    let delete = format!(r#"[{{"op": "delete_shape", "slide": {}, "shape": {}}}, {{"op": "delete_slide", "slide": {first}}}]"#, created.created[0]["slide"].as_u64().unwrap(), created.created[1]["shape"].as_u64().unwrap());
    apply(&mut session, &delete).unwrap();
    assert_eq!(session.inspect().into_presentation().slides.len(), 2);
    assert_eq!(session.inspect().into_presentation().slides[0].shapes.len(), 1);
}

#[test]
fn table_rows_are_inserted_and_deleted() {
    let mut session = blank();
    let slide = session.inspect().into_presentation().slides[0].id;
    let created = apply(&mut session, &format!(r#"{{"op": "add_table", "slide": {slide}, "rows": [[["a"], ["b"]], [["c"], ["d"]]]}}"#)).unwrap();
    let table = created.created[0]["shape"].as_u64().unwrap();
    apply(&mut session, &format!(r#"[{{"op": "insert_table_row", "slide": {slide}, "shape": {table}, "at": 1}}, {{"op": "set_cell", "slide": {slide}, "shape": {table}, "row": 1, "column": 0, "text": ["new"]}}]"#)).unwrap();
    let cells = |s: &Session| -> Vec<Vec<String>> {
        let cells = s.inspect().into_presentation().slides[0].shapes.iter().find(|s| s.kind == "table").unwrap().cells.clone().unwrap();
        let plain = |p: ParagraphText| match p {
            ParagraphText::Plain(text) => text,
            pieces => panic!("a field in {pieces:?}"),
        };
        cells.into_iter().map(|row| row.into_iter().map(|cell| cell.into_iter().map(plain).collect::<Vec<_>>().join("|")).collect()).collect()
    };
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
    let a = Session::open(bytes.clone(), "a.pptx").unwrap().inspect().into_presentation();
    let b = Session::open(bytes, "a.pptx").unwrap().inspect().into_presentation();
    assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());
}

/// What the file cannot be saved with yet is refused before it is applied (the editor would only find it at the save):
/// here a background on a slide that is new, and notes in a file with no notes master.
#[test]
fn a_change_the_file_cannot_save_is_refused() {
    let mut session = blank();
    let before = session.model().slides.clone();
    let first = before[0].id.0;
    for json in [
        r##"[{"op": "add_slide", "as": "n"}, {"op": "set_background", "slide": "n", "color": "#000000"}]"##.to_string(),
        format!(r#"{{"op": "set_notes", "slide": {first}, "text": ["x"]}}"#),
    ] {
        let err = apply(&mut session, &json).unwrap_err();
        assert_eq!(err.index, None);
        assert!(err.message.starts_with("the file cannot be saved with this change"), "{err}");
        assert_eq!(session.model().slides, before);
    }
}

#[test]
fn bad_values_are_refused_with_what_is_wrong() {
    let mut session = blank();
    let slide = session.inspect().into_presentation().slides[0].id;
    for (json, wanted) in [
        (format!(r#"{{"op": "set_background", "slide": {slide}, "color": "red"}}"#), "not a colour"),
        (format!(r#"{{"op": "add_shape", "slide": {slide}, "geometry": "blob", "frame": {{"x": 0, "y": 0, "w": 1, "h": 1}}}}"#), "no preset shape"),
        (format!(r#"{{"op": "add_text_box", "slide": {slide}, "frame": {{"x": 0, "y": 0, "w": 0, "h": 1}}, "text": ["x"]}}"#), "width and a height"),
        (r#"{"op": "add_slide", "layout": "無い"}"#.to_string(), "no layout named"),
        (format!(r#"{{"op": "set_text", "slide": {slide}, "shape": {{"placeholder": "pic"}}, "text": ["x"]}}"#), "no pic placeholder"),
    ] {
        let err = apply(&mut session, &json).unwrap_err();
        assert!(err.message.contains(wanted), "{json}: {err}");
    }
}
