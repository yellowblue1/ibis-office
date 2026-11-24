// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Two editors side by side (docs/web-agent.md, "Two editors side by side"): the place an editor tells the embedding page,
//! and the place it shows when asked, by index, in each kind of file, in a read-only editor (`?view=1`) as well; a place
//! shown is not told back.
use super::Scenario;
use crate::docx::ext::*;
use crate::shell::agent::{At, Place, Top};
use crate::xlsx::ext::*;
use gpui_kit::{MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, TouchDragEvent, TouchPhase, point, px};
use ibis_doc_model::Block;
use ibis_pptx::Kind;

pub(crate) fn all() -> Vec<(&'static str, fn())> {
    vec![
        ("shell::side_by_side_place_of_a_document", place_of_a_document),
        ("shell::side_by_side_place_of_a_presentation", place_of_a_presentation),
        ("shell::side_by_side_place_of_a_workbook", place_of_a_workbook),
        ("shell::side_by_side_place_of_large_files", place_of_large_files),
        ("shell::side_by_side_place_person_input_seen_first", person_input_seen_first),
    ]
}

/// What one reading of the place costs in a long document and a big workbook (printed), and that the page's reading
/// every frame costs nothing while the editor draws nothing.
fn place_of_large_files() {
    let paragraph = format!("\"{}\"", "A sentence of a long specification, as many of them as fill a page. ".repeat(6));
    let paragraphs = vec![paragraph; 2000].join(",");
    let ops = format!(r#"[{{"op":"insert_paragraphs","text":[{paragraphs}]}}]"#);
    let row = format!("[{}]", (0..20).map(|c| format!("\"cell {c}\"")).collect::<Vec<_>>().join(","));
    let rows = vec![row; 5000].join(",");
    let sheet_ops = format!(r#"[{{"op":"set_cells","sheet":"Sheet1","at":"A1","values":[{rows}]}}]"#);
    for (kind, name, ops) in [(Kind::Document, "large.docx", ops), (Kind::Workbook, "large.xlsx", sheet_ops)] {
        let mut s = open_made(kind, name, &ops, true);
        s.wheel((600.0, 400.0), -3000.0);
        let pages = s.with(|e, _| e.presentation().slides.len());
        const READS: u32 = 200;
        let started = std::time::Instant::now();
        for _ in 0..READS {
            std::hint::black_box(s.with(|e, _| e.agent_place()));
        }
        let each = started.elapsed() / READS;
        let _ = s.update(|e, _, _| e.agent_place_changed());
        let started = std::time::Instant::now();
        for _ in 0..READS {
            assert_eq!(s.with_app(|cx| crate::shell::agent::with_page_editor(cx, |e, _, _| Ok(e.agent_place_changed()))), Ok(None));
        }
        let idle = started.elapsed() / READS;
        eprintln!("{name}: {pages} pages; a place read {each:?}; a frame's read while nothing is drawn {idle:?}");
        assert!(idle < std::time::Duration::from_micros(50),"a frame's read while nothing is drawn takes {idle:?}");
        // A draw lets the next read look again.
        s.wheel((600.0, 400.0), 500.0);
        assert!(s.update(|e, _, _| e.agent_place_changed()).is_some(), "the scroll is told");
    }
}

/// A file of `kind` made by the agent's operations `ops`, open in a window as 開く opens it, read-only when `view`.
fn open_made(kind: Kind, name: &str, ops: &str, view: bool) -> Scenario {
    let dir = crate::shell::scenario::repo_root().join("target/side-by-side");
    std::fs::create_dir_all(&dir).expect("the folder");
    std::fs::write(dir.join(name), ibis_pptx::blank_file(kind).expect("a blank file")).expect("written");
    let mut s = Scenario::open(&format!("target/side-by-side/{name}"));
    s.update(|e, window, cx| e.agent_apply(ops, window, cx)).expect("the operations");
    s.update(|e, _, cx| {
        e.document.mark_clean();
        e.read_only = view;
        cx.notify();
    });
    s
}

fn place(s: &mut Scenario) -> Place {
    s.with(|e, _| e.agent_place()).expect("a place")
}

fn show(s: &mut Scenario, params: &str) -> Result<(), String> {
    s.update(|e, window, cx| e.agent_show(params, window, cx))
}

/// The place the document opened at, as `ready` gives it (`www/main.ts`): taken as told, so nothing is told until the
/// person moves.
fn opened(s: &mut Scenario) -> String {
    let opened = s.update(|e, _, _| e.agent_place_opened()).expect("the opening place");
    assert!(opened.starts_with(r#"{"top":"#), "{opened}");
    assert_eq!(changed(s), None, "the opening place is not told");
    opened
}

fn changed(s: &mut Scenario) -> Option<String> {
    s.update(|e, _, _| e.agent_place_changed())
}

fn not_told_back(s: &mut Scenario) {
    assert_eq!(changed(s), None, "the place shown is not told back");
}

/// The person's pointer moving with the button held where it moves nothing (above the window's content).
fn nudge(s: &mut Scenario) {
    let position = point(px(2.0), px(2.0));
    s.dispatch(vec![PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: Some(MouseButton::Left), ..Default::default() })]);
}

fn scroll(s: &mut Scenario) -> (f32, f32) {
    s.with(|e, _| {
        let offset = e.workspace_scroll.offset();
        (f32::from(offset.x), f32::from(offset.y))
    })
}

fn place_of_a_document() {
    for view in [false, true] {
        let paragraphs: Vec<String> = (1..=40).map(|n| format!("\"Paragraph {n}. {}\"", "The same words again, to fill a few lines of the page. ".repeat(4))).collect();
        let paragraphs = paragraphs.join(",");
        let ops = format!(
            r#"[{{"op":"insert_paragraphs","text":[{paragraphs}]}},{{"op":"add_table","rows":[[["a"],["b"]],[["c"],["d"]],[["e"],["f"]]]}},{{"op":"insert_paragraphs","text":[{paragraphs}]}}]"#
        );
        let mut s = open_made(Kind::Document, "side-by-side.docx", &ops, view);
        let blocks = s.with(|e, _| e.flow().unwrap().body.len());
        assert!(blocks > 80, "{blocks} blocks");

        // At the start: the first block at the top (the view's edge is above the first page), the caret in it.
        assert_eq!(place(&mut s), Place { top: Top::Block { block: 0, offset: 0.0 }, at: Some(At::Block { block: 0 }) });
        assert_eq!(opened(&mut s), r#"{"top":{"block":0,"offset":0.0},"at":{"block":0}}"#);
        // After the opening place is taken, the first one told is the person's move.
        s.wheel((600.0, 400.0), -200.0);
        let first = s.update(|e, _, _| e.agent_place_changed()).expect("the person's first move is told");
        assert!(!first.contains(r#""top":{"block":0,"offset":0.0}"#), "{first}");

        // An agent's edit that moves the blocks under the view is not the person's move: nothing is told, until they
        // scroll. (A read-only editor is not edited.)
        if !view {
            show(&mut s, r#"{"top":{"block":30,"offset":0}}"#).unwrap();
            not_told_back(&mut s);
            let first = s.with(|e, _| match &e.flow().unwrap().body[0] {
                Block::Paragraph(p) => p.id,
                Block::Table(t) => t.id,
            });
            let before = place(&mut s).top;
            s.update(|e, window, cx| e.agent_apply(&format!(r#"[{{"op":"insert_paragraphs","before":{first},"text":["New","Paragraphs"]}}]"#), window, cx)).expect("the edit");
            s.wait(100);
            assert_ne!(place(&mut s).top, before, "the edit moved the blocks under the view");
            assert_eq!(s.update(|e, _, _| e.agent_place_changed()), None, "an agent's edit is not told");
            s.wheel((600.0, 400.0), -100.0);
            assert!(s.update(|e, _, _| e.agent_place_changed()).is_some(), "the person's scroll is told");
        }

        // A block halfway down at the top edge; read back as it was given.
        show(&mut s, r#"{"top":{"block":30,"offset":0.5}}"#).unwrap();
        let Top::Block { block, offset } = place(&mut s).top else { panic!("a document's place") };
        assert_eq!(block, 30);
        assert!((offset - 0.5).abs() < 0.01, "offset {offset}");
        not_told_back(&mut s);

        // The view moved by the person is told.
        s.wheel((600.0, 400.0), -300.0);
        let moved = s.update(|e, _, _| e.agent_place_changed()).expect("a scroll is told");
        assert!(!moved.contains(r#""block":30,"offset":0.5"#), "{moved}");

        // `at` with `top`: block 31 selected whole, where `top` puts the view, not scrolled to it.
        show(&mut s, r#"{"top":{"block":10,"offset":0}}"#).unwrap();
        let before = scroll(&mut s);
        show(&mut s, r#"{"top":{"block":10,"offset":0},"at":{"block":60}}"#).unwrap();
        assert_eq!(scroll(&mut s), before, "the selection is not scrolled to");
        let (start, end) = s.with(|e, _| e.ordered_selection()).expect("a selection");
        let (first, last, length) = s.with(|e, _| match &e.flow().unwrap().body[60] {
            Block::Paragraph(p) => (p.id, p.id, p.len()),
            Block::Table(_) => panic!("a paragraph"),
        });
        assert_eq!((start.para, start.offset, end.para, end.offset), (first, 0, last, length), "the whole block is selected");
        assert_eq!(place(&mut s).at, Some(At::Block { block: 60 }));
        not_told_back(&mut s);

        // `at` alone is brought into view.
        show(&mut s, r#"{"at":{"block":75}}"#).unwrap();
        assert_ne!(scroll(&mut s), before, "the block is brought into view");
        let Top::Block { block, .. } = place(&mut s).top else { panic!() };
        assert!(block <= 75 && block > 60, "block {block} at the top");

        // A table is one block, the caret in one of its cells in it.
        let table = s.with(|e, _| e.flow().unwrap().body.iter().position(|b| matches!(b, Block::Table(_)))).expect("the table");
        show(&mut s, &format!(r#"{{"at":{{"block":{table}}}}}"#)).unwrap();
        assert_eq!(place(&mut s).at, Some(At::Block { block: table }));

        // A place past the end is the last; one of another kind is refused.
        show(&mut s, r#"{"top":{"block":100000,"offset":0}}"#).unwrap();
        let Top::Block { block, .. } = place(&mut s).top else { panic!() };
        assert!(block + 5 >= blocks, "block {block} of {blocks}");
        assert!(show(&mut s, r#"{"top":{"slide":0}}"#).is_err());
        // The place pulled back from past the end is the one shown: the person's next input that moves nothing tells
        // nothing, not the place the view was pulled back to.
        not_told_back(&mut s);
        nudge(&mut s);
        assert_eq!(changed(&mut s), None, "a place shown and pulled back is not told after the person's input");

        // A place drawn a frame or more after the person's input is still theirs; one long after it is not.
        if !view {
            let insert = |s: &mut Scenario| {
                let first = s.with(|e, _| match &e.flow().unwrap().body[0] {
                    Block::Paragraph(p) => p.id,
                    Block::Table(t) => t.id,
                });
                s.update(|e, window, cx| e.agent_apply(&format!(r#"[{{"op":"insert_paragraphs","before":{first},"text":["New"]}}]"#), window, cx)).expect("the edit");
            };
            nudge(&mut s);
            assert_eq!(changed(&mut s), None, "the input moved nothing yet");
            insert(&mut s);
            assert!(changed(&mut s).is_some(), "a place changed in the frames after the person's input is told");
            nudge(&mut s);
            for _ in 0..crate::shell::agent::PERSON_INPUT_FRAMES {
                let _ = changed(&mut s);
            }
            insert(&mut s);
            assert_eq!(changed(&mut s), None, "a place changed long after the person's input is not told");
        }

        if view {
            // Escape does not take a read-only window out of viewing (writing the mode left a change to save).
            let revision = s.with(|e, _| e.document.revision());
            s.key("escape");
            s.type_text("x");
            s.key("backspace");
            assert_eq!(s.with(|e, _| e.document.revision()), revision, "a read-only document is not edited");
            assert!(!s.with(|e, _| e.document.is_dirty()), "nothing to save");
            assert!(s.with(|e, _| e.ordered_selection()).is_some(), "it can still be selected");
        }
    }
}

fn place_of_a_presentation() {
    for view in [false, true] {
        let ops = r#"[
            {"op": "add_slide", "as": "s2"},
            {"op": "add_slide", "as": "s3"},
            {"op": "add_text_box", "slide": "s3", "frame": {"x": 50, "y": 50, "w": 300, "h": 60}, "text": ["One"]},
            {"op": "add_text_box", "slide": "s3", "frame": {"x": 50, "y": 200, "w": 300, "h": 60}, "text": ["Two"]}
        ]"#;
        let mut s = open_made(Kind::Presentation, "side-by-side.pptx", ops, view);
        let shown = |s: &mut Scenario| s.with(|e, _| e.presentation().slide_index(e.current_slide).unwrap());
        opened(&mut s);
        show(&mut s, r#"{"top":{"slide":0}}"#).unwrap();
        assert_eq!(place(&mut s), Place { top: Top::Slide { slide: 0 }, at: Some(At::Shape { slide: 0, shape: None }) });
        not_told_back(&mut s);

        // The slide and its second shape (as inspect lists them) selected.
        let listed = s.with(|e, _| {
            let slide = &e.presentation().slides[2];
            slide.shapes.iter().filter(|x| !slide.is_attached(x.id)).map(|x| x.id).collect::<Vec<_>>()
        });
        show(&mut s, &format!(r#"{{"top":{{"slide":2}},"at":{{"slide":2,"shape":{}}}}}"#, listed.len() - 1)).unwrap();
        assert_eq!(shown(&mut s), 2);
        assert_eq!(s.with(|e, _| e.selection), listed.last().copied());
        assert_eq!(place(&mut s).at, Some(At::Shape { slide: 2, shape: Some(listed.len() - 1) }));
        not_told_back(&mut s);

        // A shape of a slide not shown is not selected; `at` alone shows its slide.
        show(&mut s, r#"{"top":{"slide":1},"at":{"slide":2,"shape":0}}"#).unwrap();
        assert_eq!((shown(&mut s), s.with(|e, _| e.selection)), (1, None));
        show(&mut s, r#"{"at":{"slide":2,"shape":0}}"#).unwrap();
        assert_eq!((shown(&mut s), s.with(|e, _| e.selection)), (2, listed.first().copied()));
        show(&mut s, r#"{"at":{"slide":2,"shape":null}}"#).unwrap();
        assert_eq!(s.with(|e, _| e.selection), None);

        // Past the end: the last slide; another kind's place is refused.
        show(&mut s, r#"{"top":{"slide":99}}"#).unwrap();
        assert_eq!(shown(&mut s), 2);
        assert!(show(&mut s, r#"{"top":{"block":0}}"#).is_err());

        if !view {
            // A shape's text being typed in stays so through a `top` of its own slide; another slide is refused, changing
            // nothing; only `at` ends it.
            s.update(|e, _, cx| {
                e.set_selection(Vec::new());
                cx.notify();
            });
            let index = s.with(|e, _| e.slide().shapes.iter().position(|x| Some(x.id) == listed.first().copied()).unwrap());
            s.double_click_shape(index);
            s.type_text("typed");
            let typing = |s: &mut Scenario| s.with(|e, _| e.text_editing().is_some());
            assert!(typing(&mut s), "the shape's text is being typed in");
            let revision = s.with(|e, _| e.document.revision());
            show(&mut s, r#"{"top":{"slide":2}}"#).unwrap();
            assert!(typing(&mut s), "a top of the same slide leaves the typing");
            assert!(show(&mut s, r#"{"top":{"slide":0}}"#).is_err(), "another slide while typing is refused");
            assert!(typing(&mut s), "a refused show leaves the typing");
            assert_eq!((shown(&mut s), s.with(|e, _| e.document.revision())), (2, revision), "a refused show changes nothing");
            show(&mut s, r#"{"top":{"slide":0},"at":{"slide":0,"shape":null}}"#).unwrap();
            assert!(!typing(&mut s), "`at` ends the typing");
            assert_eq!(shown(&mut s), 0);
            let text = s.with(|e, _| e.presentation().slides[2].shapes[index].text_body().map(|b| b.text()));
            assert!(text.is_some_and(|t| t.contains("typed")), "what was typed is kept");
        }

        if view {
            show(&mut s, r#"{"at":{"slide":2,"shape":0}}"#).unwrap();
            let revision = s.with(|e, _| e.document.revision());
            s.key("delete");
            s.type_text("x");
            assert_eq!(s.with(|e, _| e.document.revision()), revision, "a read-only presentation is not edited");
            assert!(!s.with(|e, _| e.document.is_dirty()), "nothing to save");
        }
    }
}

fn place_of_a_workbook() {
    for view in [false, true] {
        let ops = r#"[
            {"op": "add_sheet", "name": "Second"},
            {"op": "set_cells", "sheet": "Second", "at": "A1", "values": [["a", "b"], ["c", "d"]]},
            {"op": "set_cells", "sheet": "Sheet1", "at": "A1", "values": [["x"]]}
        ]"#;
        let mut s = open_made(Kind::Workbook, "side-by-side.xlsx", ops, view);
        // The sheet the last operation named, at its top-left corner.
        assert_eq!(place(&mut s), Place { top: Top::Cell { sheet: 0, row: 1, column: 1 }, at: Some(At::Cell { sheet: 0, row: 1, column: 1 }) });
        opened(&mut s);

        // Another sheet, scrolled so C50 is at the top left; read back as it was given.
        show(&mut s, r#"{"top":{"sheet":1,"row":50,"column":3}}"#).unwrap();
        assert_eq!(s.with(|e, _| e.presentation().slide_index(e.current_slide)), Some(1));
        assert_eq!(place(&mut s).top, Top::Cell { sheet: 1, row: 50, column: 3 });
        not_told_back(&mut s);

        // Far past the grid drawn so far, down and right: there exactly, and so on the next drawing too.
        show(&mut s, r#"{"top":{"sheet":1,"row":5000,"column":200}}"#).unwrap();
        assert_eq!(place(&mut s).top, Top::Cell { sheet: 1, row: 5000, column: 200 });
        s.wait(50);
        assert_eq!(place(&mut s).top, Top::Cell { sheet: 1, row: 5000, column: 200 });
        not_told_back(&mut s);
        show(&mut s, r#"{"top":{"sheet":1,"row":50,"column":3}}"#).unwrap();

        // The active cell with `top`: not scrolled to.
        let before = scroll(&mut s);
        show(&mut s, r#"{"top":{"sheet":1,"row":50,"column":3},"at":{"sheet":1,"row":7,"column":2}}"#).unwrap();
        assert_eq!(scroll(&mut s), before, "the active cell is not scrolled to");
        assert_eq!(s.with(|e, _| e.current_cell()), Some((1, 6)));
        assert_eq!(place(&mut s).at, Some(At::Cell { sheet: 1, row: 7, column: 2 }));
        not_told_back(&mut s);

        // Alone, it is brought into view.
        show(&mut s, r#"{"at":{"sheet":1,"row":7,"column":2}}"#).unwrap();
        let Top::Cell { row, .. } = place(&mut s).top else { panic!() };
        assert!(row <= 7, "row 7 in view, {row} at the top");
        not_told_back(&mut s);

        // The person moving the active cell is told.
        s.key("down");
        let moved = s.update(|e, _, _| e.agent_place_changed()).expect("a move is told");
        assert!(moved.contains(r#""at":{"sheet":1,"row":8,"column":2}"#), "{moved}");

        // The active cell of a sheet not shown is the one it shows.
        show(&mut s, r#"{"top":{"sheet":0,"row":1,"column":1},"at":{"sheet":1,"row":3,"column":3}}"#).unwrap();
        show(&mut s, r#"{"top":{"sheet":1,"row":1,"column":1}}"#).unwrap();
        assert_eq!(s.with(|e, _| e.current_cell()), Some((2, 2)));

        assert!(show(&mut s, r#"{"top":{"slide":0}}"#).is_err());

        if view {
            let revision = s.with(|e, _| e.document.revision());
            s.type_text("5");
            s.key("enter");
            s.key("delete");
            assert_eq!(s.with(|e, _| e.document.revision()), revision, "a read-only workbook is not edited");
            assert!(!s.with(|e, _| e.document.is_dirty()), "nothing to save");
        }

        if !view {
            // A formula half typed stays being typed through a `top` of its own sheet, which only scrolls; one of another
            // sheet is refused, changing nothing; only `at` enters it.
            show(&mut s, r#"{"top":{"sheet":1,"row":1,"column":1},"at":{"sheet":1,"row":2,"column":2}}"#).unwrap();
            s.type_text("=VLOOKUP(");
            let typed = |s: &mut Scenario| s.with(|e, _| e.cell_edit().map(|c| (c.row, c.column, c.text.clone())));
            assert_eq!(typed(&mut s), Some((1, 1, "=VLOOKUP(".to_string())), "the formula is being typed");
            let revision = s.with(|e, _| e.document.revision());
            show(&mut s, r#"{"top":{"sheet":1,"row":40,"column":1}}"#).unwrap();
            assert_eq!(place(&mut s).top, Top::Cell { sheet: 1, row: 40, column: 1 }, "scrolled");
            assert_eq!(typed(&mut s), Some((1, 1, "=VLOOKUP(".to_string())), "a top of the same sheet leaves the formula being typed");
            let before = scroll(&mut s);
            assert!(show(&mut s, r#"{"top":{"sheet":0,"row":1,"column":1}}"#).is_err(), "another sheet while typing is refused");
            assert_eq!(typed(&mut s), Some((1, 1, "=VLOOKUP(".to_string())), "a refused show leaves the formula being typed");
            assert_eq!(s.with(|e, _| e.presentation().slide_index(e.current_slide)), Some(1), "a refused show changes nothing");
            assert_eq!((scroll(&mut s), s.with(|e, _| e.document.revision())), (before, revision), "a refused show changes nothing");
            s.key("escape");
            assert_eq!(typed(&mut s), None);
        }

        // Split panes (表示 > 分割) scroll in their own way: no place is told or shown, and a `show` there is refused
        // before anything changes.
        s.update(|e, _, _| {
            let slide = e.current_slide;
            e.document.execute(vec![ibis_doc_model::Command::Sheet(ibis_doc_model::SheetCommand::Split { slide, split: Some((2, 3)) })]).expect("split");
        });
        assert_eq!(s.with(|e, _| e.agent_place()), None);
        assert!(show(&mut s, r#"{"top":{"sheet":1,"row":10,"column":1}}"#).is_err());
        show(&mut s, r#"{"top":{"sheet":0,"row":1,"column":1}}"#).unwrap();
        if !view {
            s.update(|e, _, cx| {
                e.set_cell(0, 0);
                cx.notify();
            });
            s.type_text("=SUM(");
            let typing = s.with(|e, _| e.cell_edit().map(|c| c.text.clone()));
            assert_eq!(typing.as_deref(), Some("=SUM("));
            assert!(show(&mut s, r#"{"top":{"sheet":1,"row":10,"column":1},"at":{"sheet":1,"row":3,"column":3}}"#).is_err());
            assert_eq!(s.with(|e, _| e.cell_edit().map(|c| c.text.clone())), typing, "a refused show enters nothing");
            assert_eq!(s.with(|e, _| e.presentation().slide_index(e.current_slide)), Some(0), "a refused show shows no other sheet");
            s.key("escape");
        }
    }
}

/// The person's input is noted before any element that handles it can stop it: the formula bar's splitter takes the drag
/// that resizes it in the capture phase and stops it there. A touch drag and a pinch are input too.
fn person_input_seen_first() {
    let mut s = open_made(Kind::Workbook, "input.xlsx", r#"[{"op": "set_cells", "sheet": "Sheet1", "at": "A1", "values": [["x"]]}]"#, false);
    s.update(|e, _, cx| e.toggle_formula_bar_expanded(cx));
    s.settle();
    let bar = s.with(|e, _| e.wb().bar_bounds.get()).expect("the formula bar");
    // The splitter is the first row under the bar's text whose drag resizes the bar.
    let resized = (0..16).any(|dy| {
        let at = point(bar.center().x, bar.bottom() + px(dy as f32));
        s.dispatch(vec![
            PlatformInput::MouseMove(MouseMoveEvent { position: at, ..Default::default() }),
            PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: at, click_count: 1, ..Default::default() }),
        ]);
        assert!(s.update(|e, _, _| e.take_person_input()), "the press is noted");
        let to = point(at.x, at.y + px(40.0));
        s.dispatch(vec![PlatformInput::MouseMove(MouseMoveEvent { position: to, pressed_button: Some(MouseButton::Left), ..Default::default() })]);
        let noted = s.update(|e, _, _| e.take_person_input());
        let resized = s.with(|e, _| e.wb().bar_bounds.get()).expect("the formula bar").size.height > bar.size.height + px(20.0);
        if resized {
            assert!(noted, "the drag the splitter stopped is noted");
        }
        s.dispatch(vec![PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: to, click_count: 1, ..Default::default() })]);
        let _ = s.update(|e, _, _| e.take_person_input());
        resized
    });
    assert!(resized, "the splitter under the bar {bar:?} takes a drag");

    let start = point(px(400.0), px(400.0));
    s.dispatch(vec![PlatformInput::TouchDrag(TouchDragEvent { phase: TouchPhase::Moved, start_position: start, position: point(px(400.0), px(300.0)) })]);
    assert!(s.update(|e, _, _| e.take_person_input()), "a touch drag is noted");
    s.pinch((400.0, 400.0), 0.1);
    assert!(s.update(|e, _, _| e.take_person_input()), "a pinch is noted");
}
