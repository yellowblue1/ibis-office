// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use crate::document::{BlockInfo, Inspection};
use crate::{Applied, OpError, Ops, Session};

fn blank() -> Session {
    Session::open(crate::blank("new.docx").unwrap(), "new.docx").unwrap()
}

fn parse_ops(json: &str) -> Result<Ops, String> {
    crate::parse_ops(ibis_pptx::Kind::Document, json)
}

fn apply(session: &mut Session, json: &str) -> Result<Applied, OpError> {
    session.apply(&parse_ops(json).unwrap())
}

fn read(session: &Session) -> Inspection {
    session.inspect().into_document()
}

fn reopened(session: &Session) -> Session {
    Session::open(session.save().unwrap(), "saved.docx").unwrap()
}

/// The body's paragraphs' text, a table as `[table]`.
fn texts(session: &Session) -> Vec<String> {
    read(session).blocks.into_iter().map(|b| b.text.unwrap_or_else(|| "[table]".into())).collect()
}

fn first_id(session: &Session) -> u32 {
    read(session).blocks[0].id
}

fn flow(session: &Session) -> &ibis_doc_model::FlowDocument {
    session.model().flow.as_deref().unwrap()
}

fn paragraph(session: &Session, id: u32) -> &ibis_doc_model::FlowParagraph {
    ibis_doc_model::flow_command::paragraph(flow(session), id).unwrap()
}

fn find<'a>(doc: &'a Inspection, text: &str) -> &'a BlockInfo {
    doc.blocks.iter().find(|b| b.text.as_deref() == Some(text)).unwrap_or_else(|| panic!("no paragraph {text:?}"))
}

/// A report made on the new document in one list: a heading, a list and a table, as the agent would (a list's items
/// after the first are named by the ids `created` gives, so they are made a list by a list of their own).
fn report(session: &mut Session) -> Applied {
    let first = first_id(session);
    let applied = apply(
        session,
        &format!(
            r#"[
                {{"op": "set_text", "paragraph": {first}, "text": "週次報告"}},
                {{"op": "set_style", "paragraphs": [{first}], "style": "Heading 1"}},
                {{"op": "insert_paragraphs", "after": {first}, "text": ["背景", "提案", "予定"], "as": "items"}},
                {{"op": "format_paragraphs", "paragraphs": ["items"], "format": {{"list": "bullet"}}}},
                {{"op": "add_table", "rows": [[["項目"], ["期日"]], [["試作"], ["12月"]]], "as": "t"}},
                {{"op": "insert_paragraphs", "text": ["以上"]}}
            ]"#
        ),
    )
    .unwrap();
    let items = &applied.created[2]["paragraphs"];
    apply(session, &format!(r#"{{"op": "format_paragraphs", "paragraphs": [{}, {}], "format": {{"list": "bullet"}}}}"#, items[1], items[2])).unwrap();
    applied
}

/// The new document is one empty paragraph, with the styles a heading can take.
#[test]
fn inspect_reads_the_blank_document() {
    let doc = read(&blank());
    assert_eq!(doc.blocks.len(), 1);
    assert_eq!((doc.blocks[0].kind, doc.blocks[0].text.as_deref()), ("paragraph", Some("")));
    assert_eq!(doc.styles[..6], ["Normal", "Title", "Subtitle", "Heading 1", "Heading 2", "Heading 3"]);
    let json = serde_json::to_string(&blank().inspect()).unwrap();
    assert!(json.starts_with(r#"{"kind":"document","name":"new.docx","dirty":false,"styles":["#), "{json}");
    assert!(json.contains(r#""blocks":[{"id":"#) && json.contains(r#""type":"paragraph","text":"""#), "{json}");
}

/// A heading, a list and a table made by the operations are one undo for each list of them, and the file saved and
/// read again has them, formatted.
#[test]
fn a_report_is_an_undo_for_each_list_and_saves() {
    let mut session = blank();
    let before = session.model().clone();
    let applied = report(&mut session);
    assert_eq!(applied.created.len(), 6);
    assert_eq!(applied.created[2]["paragraphs"].as_array().unwrap().len(), 3);
    assert!(applied.created[4]["table"].is_u64());
    assert_eq!(texts(&session), ["週次報告", "背景", "提案", "予定", "[table]", "", "以上"]);

    for s in [&session, &reopened(&session)] {
        let doc = read(s);
        assert_eq!(texts(s), ["週次報告", "背景", "提案", "予定", "[table]", "", "以上"]);
        assert_eq!(doc.blocks[0].style.as_deref(), Some("Heading 1"));
        assert!(doc.styles.contains(&"Heading 1".to_string()));
        for item in &doc.blocks[1..4] {
            assert_eq!((item.list, item.level, item.style.as_deref()), (Some("bullet"), Some(0), None), "{item:?}");
        }
        assert_eq!(doc.blocks[6].list, None);
        let cells = doc.blocks[4].cells.clone().unwrap();
        assert_eq!(cells, vec![vec![vec!["項目".to_string()], vec!["期日".to_string()]], vec![vec!["試作".to_string()], vec!["12月".to_string()]]]);
        let heading = paragraph(s, doc.blocks[0].id);
        assert!(heading.runs[0].style.bold, "a heading of Word's default style set is bold");
        assert_eq!(heading.runs[0].style.size, 1600);
    }
    assert!(read(&session).dirty);

    assert!(session.document.undo());
    assert!(session.document.undo());
    assert_eq!(session.model().flow, before.flow);
    assert!(!session.document.can_undo());
}

/// Text goes in as the paragraphs it is read out as: given back, it changes nothing.
#[test]
fn text_is_read_and_written_as_the_same_paragraphs() {
    let mut session = blank();
    let first = first_id(&session);
    apply(&mut session, &format!(r#"{{"op": "insert_paragraphs", "after": {first}, "text": ["Empowering teams", "", "[Date]"]}}"#)).unwrap();
    assert_eq!(texts(&session), ["", "Empowering teams", "", "[Date]"]);
    let before = session.model().flow.clone();
    let doc = read(&session);
    let ops: Vec<String> = doc.blocks.iter().map(|b| format!(r#"{{"op": "set_text", "paragraph": {}, "text": {}}}"#, b.id, serde_json::to_string(b.text.as_ref().unwrap()).unwrap())).collect();
    apply(&mut session, &format!("[{}]", ops.join(","))).unwrap();
    assert_eq!(session.model().flow, before);
}

/// A paragraph holds no line break: one is refused with where it is, in a paragraph list and in one paragraph's text.
#[test]
fn a_line_break_in_a_paragraph_is_refused() {
    let err = parse_ops(r#"{"op": "insert_paragraphs", "text": ["a", "b\nc"]}"#).unwrap_err();
    assert!(err.contains("operation 0") && err.contains("paragraph 1 has a line break"), "{err}");
    let err = parse_ops(r#"[{"op": "delete", "blocks": [1]}, {"op": "set_text", "paragraph": 1, "text": "a\nb"}]"#).unwrap_err();
    assert!(err.contains("operation 1") && err.contains("line break"), "{err}");
    let err = parse_ops(r#"{"op": "insert_paragraphs", "text": "a"}"#).unwrap_err();
    assert!(err.contains("operation 0"), "{err}");
}

/// An operation that fails refuses the whole list, with its index: what the ones before it did is not applied either.
#[test]
fn a_failing_operation_changes_nothing() {
    let mut session = blank();
    let before = session.model().clone();
    let err = apply(&mut session, r#"[{"op": "insert_paragraphs", "text": ["x"]}, {"op": "set_text", "paragraph": 999, "text": "y"}]"#).unwrap_err();
    assert_eq!(err.index, Some(1));
    assert!(err.message.contains("no block 999"), "{err}");
    assert_eq!(session.model().flow, before.flow);
    assert!(!session.document.can_undo());
}

#[test]
fn unknown_operations_and_fields_are_refused_by_name() {
    assert!(parse_ops(r#"[{"op": "explode"}]"#).unwrap_err().contains("operation 0"));
    let err = parse_ops(r#"{"op": "delete", "blocks": [1], "colour": "red"}"#).unwrap_err();
    assert!(err.contains("colour"), "{err}");
    let err = crate::parse_ops(ibis_pptx::Kind::Presentation, r#"{"op": "insert_paragraphs", "text": ["x"]}"#).unwrap_err();
    assert!(err.contains("operation 0"), "{err}");
}

/// Set text keeps the paragraph's format: a line made bold and red stays so when its words change.
#[test]
fn set_text_keeps_the_format_of_what_it_replaces() {
    let mut session = blank();
    let first = first_id(&session);
    apply(&mut session, &format!(r##"[{{"op": "set_text", "paragraph": {first}, "text": "旧"}}, {{"op": "format_text", "paragraph": {first}, "format": {{"bold": true, "size": 14, "color": "#C00000", "font": "游ゴシック"}}}}]"##)).unwrap();
    apply(&mut session, &format!(r#"{{"op": "set_text", "paragraph": {first}, "text": "新しい見出し"}}"#)).unwrap();
    for s in [&session, &reopened(&session)] {
        let p = paragraph(s, read(s).blocks[0].id);
        assert_eq!(p.runs.len(), 1);
        let f = &p.runs[0].format;
        assert_eq!((f.bold, f.size, f.color), (Some(true), Some(1400), Some(ibis_doc_model::Rgb(0xC00000))));
        assert_eq!((f.font.as_deref(), f.east_asian_font.as_deref()), (Some("游ゴシック"), Some("游ゴシック")), "a Japanese typeface is set for both scripts");
        assert_eq!(texts(s), ["新しい見出し"]);
    }
}

/// Format text gives only the matches its format, and a Latin typeface only to Latin text.
#[test]
fn format_text_formats_the_matches() {
    let mut session = blank();
    let first = first_id(&session);
    apply(&mut session, &format!(r#"[{{"op": "set_text", "paragraph": {first}, "text": "締切は金曜、金曜です"}}, {{"op": "format_text", "paragraph": {first}, "match": "金曜", "format": {{"underline": true, "italic": true, "font": "Arial"}}}}]"#)).unwrap();
    let p = paragraph(&session, first);
    let marked: Vec<(String, Option<bool>)> = p.runs.iter().map(|r| (match &r.kind { ibis_doc_model::FlowRunKind::Text(t) => t.clone(), _ => String::new() }, r.format.underline)).collect();
    assert_eq!(marked, [("締切は".to_string(), None), ("金曜".into(), Some(true)), ("、".into(), None), ("金曜".into(), Some(true)), ("です".into(), None)]);
    assert_eq!((p.runs[1].format.font.as_deref(), p.runs[1].format.east_asian_font.as_deref()), (Some("Arial"), None));
    let err = apply(&mut session, &format!(r#"{{"op": "format_text", "paragraph": {first}, "match": "土曜", "format": {{"bold": true}}}}"#)).unwrap_err();
    assert!(err.message.contains("not in the text"), "{err}");
}

/// Replace text goes through the body's paragraphs and its tables' cells, keeps the format of what it replaces, and
/// says how many it replaced; none found is an error the agent can read.
#[test]
fn replace_text_in_paragraphs_and_cells() {
    let mut session = blank();
    let first = first_id(&session);
    apply(
        &mut session,
        &format!(
            r#"[{{"op": "set_text", "paragraph": {first}, "text": "A社の提案 A社"}},
                {{"op": "format_text", "paragraph": {first}, "match": "提案", "format": {{"bold": true}}}},
                {{"op": "add_table", "after": {first}, "rows": [[["A社"], ["B社"]]]}}]"#
        ),
    )
    .unwrap();
    let applied = apply(&mut session, r#"{"op": "replace_text", "find": "A社", "replace": "イビス"}"#).unwrap();
    assert_eq!(applied.replaced, 3);
    let doc = read(&session);
    assert_eq!(doc.blocks[0].text.as_deref(), Some("イビスの提案 イビス"));
    assert_eq!(doc.blocks[1].cells.as_ref().unwrap()[0], [vec!["イビス".to_string()], vec!["B社".to_string()]]);
    let bold: Vec<bool> = paragraph(&session, first).runs.iter().map(|r| r.format.bold == Some(true)).collect();
    assert_eq!(bold, [false, true, false]);

    let applied = apply(&mut session, &format!(r#"{{"op": "replace_text", "find": "イビス", "replace": "", "paragraph": {first}}}"#)).unwrap();
    assert_eq!(applied.replaced, 2);
    assert_eq!(read(&session).blocks[1].cells.as_ref().unwrap()[0][0], ["イビス"]);
    let err = apply(&mut session, r#"{"op": "replace_text", "find": "A社", "replace": "x"}"#).unwrap_err();
    assert!(err.message.contains("not in the text"), "{err}");
}

/// Paragraphs go before and after blocks, in a style; deleted, they and tables are gone; the result saves.
#[test]
fn paragraphs_are_inserted_where_named_and_deleted() {
    let mut session = blank();
    report(&mut session);
    let doc = read(&session);
    let table = doc.blocks[4].id;
    let created = apply(
        &mut session,
        &format!(
            r#"[{{"op": "insert_paragraphs", "before": {table}, "text": ["表 1"], "style": "Subtitle", "as": "caption"}},
                {{"op": "insert_paragraphs", "after": "caption", "text": ["注"]}},
                {{"op": "insert_paragraphs", "before": {}, "text": ["まえがき"]}}]"#,
            doc.blocks[0].id
        ),
    )
    .unwrap();
    assert_eq!(texts(&session), ["まえがき", "週次報告", "背景", "提案", "予定", "表 1", "注", "[table]", "", "以上"]);
    let doc = read(&session);
    assert_eq!(find(&doc, "表 1").style.as_deref(), Some("Subtitle"));
    assert_eq!(find(&doc, "注").style, None);
    assert_eq!(created.created[0]["paragraphs"][0].as_u64(), Some(u64::from(find(&doc, "表 1").id)));

    apply(&mut session, &format!(r#"{{"op": "delete", "blocks": [{}, {table}, {}]}}"#, find(&doc, "注").id, find(&doc, "提案").id)).unwrap();
    assert_eq!(texts(&session), ["まえがき", "週次報告", "背景", "予定", "表 1", "", "以上"]);
    assert_eq!(texts(&reopened(&session)), texts(&session));

    let all: Vec<String> = read(&session).blocks.iter().map(|b| b.id.to_string()).collect();
    let err = apply(&mut session, &format!(r#"{{"op": "delete", "blocks": [{}]}}"#, all.join(", "))).unwrap_err();
    assert!(err.message.contains("ends with a paragraph"), "{err}");
}

/// Paragraph formats: alignment, spacing, a numbered list and its levels, and taking the list away again.
#[test]
fn paragraph_formats_and_lists() {
    let mut session = blank();
    let first = first_id(&session);
    apply(&mut session, &format!(r#"{{"op": "insert_paragraphs", "after": {first}, "text": ["一", "二", "三"], "as": "l"}}"#)).unwrap();
    let ids: Vec<u32> = read(&session).blocks[1..].iter().map(|b| b.id).collect();
    apply(
        &mut session,
        &format!(
            r#"[{{"op": "format_paragraphs", "paragraphs": [{}, {}, {}], "format": {{"list": "numbered", "align": "center", "space_before": 6, "space_after": 12}}}},
                {{"op": "format_paragraphs", "paragraphs": [{}], "format": {{"level": 1}}}}]"#,
            ids[0], ids[1], ids[2], ids[1]
        ),
    )
    .unwrap();
    for s in [&session, &reopened(&session)] {
        let doc = read(s);
        let lists: Vec<_> = doc.blocks[1..].iter().map(|b| (b.list, b.level)).collect();
        assert_eq!(lists, [(Some("numbered"), Some(0)), (Some("numbered"), Some(1)), (Some("numbered"), Some(0))]);
        let p = paragraph(s, doc.blocks[1].id);
        assert_eq!((p.align, p.space_before, p.space_after), (ibis_doc_model::FlowAlign::Center, 6 * 12_700, 12 * 12_700));
    }
    apply(&mut session, &format!(r#"{{"op": "format_paragraphs", "paragraphs": [{}], "format": {{"list": "none"}}}}"#, ids[2])).unwrap();
    assert_eq!(read(&session).blocks[3].list, None);
    let err = apply(&mut session, &format!(r#"{{"op": "format_paragraphs", "paragraphs": [{first}], "format": {{"level": 2}}}}"#)).unwrap_err();
    assert!(err.message.contains("not a list item"), "{err}");
}

/// A style is named as the summary lists it; another name is refused with the names there are.
#[test]
fn an_unknown_style_is_refused_with_the_styles() {
    let mut session = blank();
    let first = first_id(&session);
    let err = apply(&mut session, &format!(r#"{{"op": "set_style", "paragraphs": [{first}], "style": "Heading 7"}}"#)).unwrap_err();
    assert_eq!(err.index, Some(0));
    assert!(err.message.contains(r#"no style named "Heading 7""#) && err.message.contains(r#""Heading 1""#), "{err}");
    apply(&mut session, &format!(r#"[{{"op": "set_style", "paragraphs": [{first}], "style": "heading 2"}}, {{"op": "set_style", "paragraphs": [{first}], "style": "Normal"}}]"#)).unwrap();
    assert_eq!(read(&session).blocks[0].style, None);
}

/// Table cells are set, rows inserted and deleted, and all of it saves.
#[test]
fn table_cells_and_rows() {
    let mut session = blank();
    let created = apply(&mut session, r#"{"op": "add_table", "rows": [[["a"], ["b"]], [["c"], ["d"]]]}"#).unwrap();
    let table = created.created[0]["table"].as_u64().unwrap();
    let cells = |s: &Session| -> Vec<Vec<String>> {
        let doc = read(s);
        let cells = doc.blocks.iter().find(|b| b.kind == "table").unwrap().cells.clone().unwrap();
        cells.into_iter().map(|row| row.into_iter().map(|cell| cell.join("|")).collect()).collect()
    };
    apply(
        &mut session,
        &format!(
            r#"[{{"op": "insert_table_row", "table": {table}, "at": 1}},
                {{"op": "set_cell", "table": {table}, "row": 1, "column": 0, "text": ["new", "line"]}},
                {{"op": "insert_table_row", "table": {table}}},
                {{"op": "set_cell", "table": {table}, "row": 0, "column": 1, "text": ["B"]}},
                {{"op": "format_text", "table": {table}, "cell": [0, 1], "format": {{"bold": true}}}},
                {{"op": "format_paragraphs", "paragraphs": [{table}], "format": {{"align": "right"}}}}]"#
        ),
    )
    .unwrap();
    assert_eq!(cells(&session), vec![vec!["a", "B"], vec!["new|line", ""], vec!["c", "d"], vec!["", ""]]);
    apply(&mut session, &format!(r#"{{"op": "delete_table_row", "table": {table}, "at": 0}}"#)).unwrap();
    assert_eq!(cells(&session), vec![vec!["new|line", ""], vec!["c", "d"], vec!["", ""]]);
    assert_eq!(cells(&reopened(&session)), cells(&session));

    for (json, wanted) in [
        (format!(r#"{{"op": "set_cell", "table": {table}, "row": 9, "column": 0, "text": ["x"]}}"#), "no cell [9, 0]"),
        (format!(r#"{{"op": "delete_table_row", "table": {table}, "at": 3}}"#), "no row 3"),
        (format!(r#"{{"op": "insert_table_row", "table": {table}, "at": 9}}"#), "past the end"),
        (format!(r#"{{"op": "format_text", "table": {table}, "format": {{"bold": true}}}}"#), "give cell"),
        (format!(r#"{{"op": "set_text", "paragraph": {table}, "text": "x"}}"#), "is a table"),
    ] {
        let err = apply(&mut session, &json).unwrap_err();
        assert!(err.message.contains(wanted), "{json}: {err}");
    }
}

/// The ids are those a file is read with: the same bytes read again give the same ids, so a summary of a file names
/// what the command-line tool's next run of it edits; while it is open, an edit leaves the others' ids as they were.
#[test]
fn reading_the_same_file_gives_the_same_ids() {
    let mut session = blank();
    report(&mut session);
    let bytes = session.save().unwrap();
    let a = Session::open(bytes.clone(), "a.docx").unwrap();
    let b = Session::open(bytes, "a.docx").unwrap();
    assert_eq!(serde_json::to_string(&a.inspect()).unwrap(), serde_json::to_string(&b.inspect()).unwrap());

    let mut a = a;
    let ids: Vec<u32> = read(&a).blocks.iter().map(|b| b.id).collect();
    apply(&mut a, &format!(r#"[{{"op": "insert_paragraphs", "after": {}, "text": ["x"]}}, {{"op": "delete", "blocks": [{}]}}]"#, ids[0], ids[2])).unwrap();
    let after: Vec<u32> = read(&a).blocks.iter().map(|b| b.id).collect();
    assert_eq!([after[0], after[2], after[3]], [ids[0], ids[1], ids[3]]);
}

/// The edits the Word editor would refuse, or record as changes, are refused before anything is applied.
#[test]
fn a_document_that_records_changes_is_refused() {
    let mut session = blank();
    session.document.execute(vec![ibis_doc_model::Command::Flow(ibis_doc_model::FlowCommand::SetTrackChanges(true))]).unwrap();
    let err = apply(&mut session, r#"{"op": "insert_paragraphs", "text": ["x"]}"#).unwrap_err();
    assert_eq!(err.index, None);
    assert!(err.message.contains("records changes"), "{err}");
}

/// A document whose first paragraph reads "keep gone" with "gone" a tracked deletion not yet accepted.
fn with_a_deletion() -> Session {
    let mut session = blank();
    let first = first_id(&session);
    apply(&mut session, &format!(r#"{{"op": "set_text", "paragraph": {first}, "text": "keep gone"}}"#)).unwrap();
    let mut model = session.model().clone();
    let mut doc = (*model.flow.take().unwrap()).clone();
    let Some(ibis_doc_model::flow::Block::Paragraph(p)) = doc.body.first_mut() else { unreachable!() };
    let mut gone = p.runs[0].clone();
    p.runs[0].kind = ibis_doc_model::flow::FlowRunKind::Text("keep ".into());
    gone.kind = ibis_doc_model::flow::FlowRunKind::Text("gone".into());
    gone.revision = Some(ibis_doc_model::flow::Revision { kind: ibis_doc_model::flow::RevisionKind::Delete, author: "someone".into(), date: String::new() });
    p.runs.push(gone);
    model.flow = Some(std::sync::Arc::new(doc));
    session.document = ibis_doc_model::Document::new(model);
    session
}

/// A tracked change is the person's to accept or reject: the summary reads the document as accepting them would leave
/// it, says it has them, and no edit is made until they are gone.
#[test]
fn a_document_with_tracked_changes_reads_as_accepted_and_is_not_edited() {
    let session = with_a_deletion();
    let doc = read(&session);
    assert_eq!(doc.blocks[0].text.as_deref(), Some("keep "));
    assert!(doc.revisions);
    let json = serde_json::to_string(&session.inspect()).unwrap();
    assert!(json.contains(r#""revisions":true"#), "{json}");
    assert!(!serde_json::to_string(&blank().inspect()).unwrap().contains("revisions"));

    let mut session = session;
    let err = apply(&mut session, r#"{"op": "insert_paragraphs", "text": ["x"]}"#).unwrap_err();
    assert!(err.message.contains("tracked changes"), "{err}");
    assert_eq!(session.document.undo_len(), 0);
}

/// A character a file cannot hold is refused in what the agent writes; one already in the model is left out of the file
/// written, which then opens again.
#[test]
fn characters_a_file_cannot_hold() {
    let mut session = blank();
    let first = first_id(&session);
    for json in [
        format!(r#"{{"op": "set_text", "paragraph": {first}, "text": "a\u0001b"}}"#),
        r#"{"op": "insert_paragraphs", "text": ["a\u0007"]}"#.to_string(),
        r#"{"op": "replace_text", "find": "a", "replace": "\u0000"}"#.to_string(),
    ] {
        let err = parse_ops(&json).map(|ops| session.apply(&ops)).map_or_else(|e| e, |r| r.unwrap_err().message);
        assert!(err.contains("which a file cannot hold"), "{json}: {err}");
    }
    apply(&mut session, &format!(r#"{{"op": "set_text", "paragraph": {first}, "text": "a\tb"}}"#)).unwrap();

    let at = ibis_doc_model::FlowPos::new(first, 1);
    session.document.execute(vec![ibis_doc_model::Command::Flow(ibis_doc_model::FlowCommand::InsertText { at, text: "\u{1}".into(), format: None })]).unwrap();
    assert_eq!(texts(&reopened(&session)), ["a\tb"]);
}

/// Deleting the text a comment marks deletes the comment, in the same undo, as the Word editor does.
#[test]
fn a_comment_goes_with_the_text_it_marks() {
    let mut session = blank();
    let first = first_id(&session);
    let applied = apply(&mut session, &format!(r#"[{{"op": "set_text", "paragraph": {first}, "text": "rest"}}, {{"op": "insert_paragraphs", "text": ["noted"]}}]"#)).unwrap();
    let noted = applied.created[1]["paragraphs"][0].as_u64().unwrap() as u32;
    let who = ibis_doc_model::flow_command::CommentAuthor { author: "someone".into(), initials: "S".into(), date: "2026-11-24T00:00:00Z".into() };
    let add = ibis_doc_model::flow_command::add_comment(flow(&session), ibis_doc_model::FlowPos::new(noted, 0), ibis_doc_model::FlowPos::new(noted, 5), &who, "check").unwrap();
    session.document.execute(add.into_iter().map(ibis_doc_model::Command::Flow).collect()).unwrap();
    assert_eq!(flow(&session).comments.len(), 1);
    let steps = session.document.undo_len();
    let marked = texts(&session);

    apply(&mut session, &format!(r#"{{"op": "delete", "blocks": [{noted}]}}"#)).unwrap();
    assert!(flow(&session).comments.is_empty());
    assert!(reopened(&session).model().flow.as_deref().unwrap().comments.is_empty());
    assert_eq!(session.document.undo_len(), steps + 1);
    session.document.undo();
    assert_eq!(flow(&session).comments.len(), 1);
    assert_eq!(texts(&session), marked);
}

/// An id the summary showed names that block only: one deleted is not given to a block made after, while the file is
/// open.
#[test]
fn ids_are_not_given_again() {
    let mut session = blank();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..3 {
        let applied = apply(&mut session, r#"[{"op": "insert_paragraphs", "text": ["a", "b"]}, {"op": "add_table", "rows": [[["x"], ["y"]]]}]"#).unwrap();
        let table = applied.created[1]["table"].as_u64().unwrap() as u32;
        apply(&mut session, &format!(r#"{{"op": "insert_table_row", "table": {table}}}"#)).unwrap();
        apply(&mut session, &format!(r#"{{"op": "set_cell", "table": {table}, "row": 1, "column": 0, "text": ["p", "q"]}}"#)).unwrap();
        let doc = flow(&session);
        let mut ids = Vec::new();
        collect(&doc.body, &mut ids);
        let made: Vec<u32> = ids.into_iter().filter(|&id| id != first_id(&blank())).collect();
        for id in &made {
            assert!(seen.insert(*id), "id {id} given again");
        }
        let blocks: Vec<String> = read(&session).blocks.iter().skip(1).map(|b| b.id.to_string()).collect();
        apply(&mut session, &format!(r#"{{"op": "delete", "blocks": [{}]}}"#, blocks.join(", "))).unwrap();
    }

    fn collect(blocks: &[ibis_doc_model::flow::Block], ids: &mut Vec<u32>) {
        for b in blocks {
            match b {
                ibis_doc_model::flow::Block::Paragraph(p) => ids.push(p.id),
                ibis_doc_model::flow::Block::Table(t) => {
                    ids.push(t.id);
                    for row in &t.rows {
                        ids.push(row.id);
                        for cell in &row.cells {
                            ids.push(cell.id);
                            collect(&cell.blocks, ids);
                        }
                    }
                }
            }
        }
    }
}

/// A list of no operations is done and changes nothing.
#[test]
fn an_empty_list_changes_nothing() {
    let mut session = blank();
    let applied = apply(&mut session, "[]").unwrap();
    assert!(applied.created.is_empty());
    assert_eq!(session.document.undo_len(), 0);
    assert!(!session.document.is_dirty());
}

#[test]
fn bad_values_are_refused_with_what_is_wrong() {
    let mut session = blank();
    let first = first_id(&session);
    apply(&mut session, &format!(r#"{{"op": "set_text", "paragraph": {first}, "text": "x"}}"#)).unwrap();
    for (json, wanted) in [
        (format!(r#"{{"op": "format_text", "paragraph": {first}, "format": {{"color": "red"}}}}"#), "not a colour"),
        (format!(r#"{{"op": "format_text", "paragraph": {first}, "format": {{"size": 0}}}}"#), "not a font size"),
        (format!(r#"{{"op": "format_paragraphs", "paragraphs": [{first}], "format": {{"space_after": -1}}}}"#), "not a space"),
        (format!(r#"{{"op": "insert_paragraphs", "after": {first}, "before": {first}, "text": ["x"]}}"#), "not both"),
        (r#"{"op": "add_table", "rows": []}"#.to_string(), "at least one row"),
        (r#"{"op": "set_style", "paragraphs": ["nope"], "style": "Normal"}"#.to_string(), "nothing named \"nope\""),
        (r#"{"op": "replace_text", "find": "", "replace": "x"}"#.to_string(), "find is empty"),
    ] {
        let err = apply(&mut session, &json).unwrap_err();
        assert!(err.message.contains(wanted), "{json}: {err}");
    }
}
