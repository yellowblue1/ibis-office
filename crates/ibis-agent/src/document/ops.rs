// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The operations an agent edits a document with, and how they become the editor's commands (those the Word editor
//! makes for the same edits).
//!
//! Sizes are in points, colours `#RRGGBB`, and the body's paragraphs and tables are named by the ids [`crate::inspect`]
//! gives, or, for one made earlier in the same list, by the name its `as` gave it.

use std::collections::{HashMap, HashSet};

use ibis_doc_model::command::Command;
use ibis_doc_model::flow::{Block, FlowAlign, FlowDocument, FlowParagraph, FlowRunKind, FlowStyles, StyleKind, StyleSetPreset};
use ibis_doc_model::flow_command::{self, Container, apply_to, FlowCommand, FlowPos, InsertFormat, ParaChange, RunChange, TableEdit};
use ibis_doc_model::{Document, EMU_PER_POINT, Presentation};
use ibis_pptx::Source;
use serde::{Deserialize, Serialize};

use crate::shared::{Align, List, ParagraphFormat, Text, TextFormat, font_size, has_line_break, holdable, occurrences, rgb};
use crate::{OpError, Planned};

/// A paragraph or a table of the body: its id, or the `as` name of one made earlier in the same list.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Ref {
    Id(u32),
    Name(String),
}

/// The text of one paragraph: it holds no line break, as each of [`Text`]'s paragraphs.
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "String")]
pub struct Line(String);

impl TryFrom<String> for Line {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        if has_line_break(&text) {
            return Err("the text has a line break: a paragraph holds none (more paragraphs are made with insert_paragraphs)".into());
        }
        holdable(&text)?;
        Ok(Line(text))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// Paragraphs of `text` after or before a block (at the end of the body when neither is given), in style `style` (by
    /// name; the normal one when not given) with no formatting of their own. `as` names the first.
    InsertParagraphs {
        after: Option<Ref>,
        before: Option<Ref>,
        text: Text,
        style: Option<String>,
        #[serde(rename = "as")]
        name: Option<String>,
    },
    /// Replace a paragraph's text, which takes the format of its first character.
    SetText { paragraph: Ref, text: Line },
    /// Remove paragraphs and tables of the body.
    Delete { blocks: Vec<Ref> },
    /// Replace every `find` with `replace`, in the body's paragraphs and its tables' cells, or in `paragraph` only (a
    /// paragraph or a table), keeping the format of the text replaced.
    ReplaceText { find: String, replace: String, paragraph: Option<Ref> },
    /// Give a paragraph's text (or each `match` in it; the text of cell `cell` [row, column] of `table`) a character
    /// format.
    FormatText {
        paragraph: Option<Ref>,
        table: Option<Ref>,
        cell: Option<[usize; 2]>,
        #[serde(rename = "match")]
        matching: Option<String>,
        format: TextFormat,
    },
    /// Give paragraphs (a table: its cells' paragraphs) a paragraph format.
    FormatParagraphs { paragraphs: Vec<Ref>, format: ParagraphFormat },
    /// Give paragraphs (a table: its cells' paragraphs) a paragraph style, by one of the names the summary's `styles`
    /// lists.
    SetStyle { paragraphs: Vec<Ref>, style: String },
    /// A table of `rows` (each a list of the cells' text) across the text's width, after or before a block (at the end
    /// of the body when neither is given), as 挿入 > 表 makes it.
    AddTable {
        after: Option<Ref>,
        before: Option<Ref>,
        rows: Vec<Vec<Text>>,
        #[serde(rename = "as")]
        name: Option<String>,
    },
    /// Replace the text of the cell at `row`, `column` (0-based; `column` counts the row's cells) of a table.
    SetCell { table: Ref, row: usize, column: usize, text: Text },
    /// A row inserted into a table at `at` (0-based; the end when not given), its cells empty and formatted as those of
    /// the row next to it.
    InsertTableRow { table: Ref, at: Option<usize> },
    DeleteTableRow { table: Ref, at: usize },
}

/// What an operation made, for the agent to name it next.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Created {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paragraphs: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<u32>,
    /// How many places `replace_text` replaced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaced: Option<usize>,
}

/// Turn `ops` into commands on `current`, opened from `source`. Each operation sees what the ones before it did, and
/// what the Word editor adds to an edit of its own (`flow_follow_up`) follows each; the result is saved (and the bytes
/// thrown away) to make sure the file can be written with it.
///
/// The paragraphs and tables made take their ids from `new_id` (the editor's document's counter, `Document::ids`),
/// past any the document has had: an id the agent read is never another block's while the file is open, though the
/// block it named was deleted (the document's own numbering, the largest id and one, would give it again).
pub fn plan(ops: &[Op], current: &Presentation, source: &Source, new_id: &mut dyn FnMut() -> u64) -> Result<Planned, OpError> {
    let doc = current.flow.as_deref().ok_or_else(|| OpError { index: None, message: "the file has no document".into() })?;
    editable(doc).map_err(|message| OpError { index: None, message })?;
    let mut planner = Planner { scratch: Document::new(current.clone()), names: HashMap::new(), commands: Vec::new(), new_id, high: 0 };
    // Past what the document has now, so that an id deleted by this list or a later one is not given again.
    planner.fresh().map_err(|message| OpError { index: None, message })?;
    let mut created = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        if doc.formatting_protection && matches!(op, Op::FormatText { .. } | Op::FormatParagraphs { .. } | Op::SetStyle { .. }) {
            return Err(OpError { index: Some(index), message: "the document's formatting is protected (書式の制限)".into() });
        }
        let made = planner.op(op).map_err(|message| OpError { index: Some(index), message })?;
        created.push(made);
    }
    crate::check_saves(source, planner.scratch.presentation())?;
    let replaced = created.iter().filter_map(|c| c.replaced).sum();
    let created = created.iter().map(|c| serde_json::to_value(c).expect("serialisable")).collect();
    Ok(Planned { commands: planner.commands, created, replaced, shown: None })
}

/// The edits the Word editor itself refuses, and those it would record as tracked changes, which these operations do
/// not make.
fn editable(doc: &FlowDocument) -> Result<(), String> {
    if doc.mark_as_final {
        return Err("the document is marked as final (最終版): it is not edited".into());
    }
    if doc.protection != ibis_doc_model::flow::DocumentProtection::None {
        return Err("the document is protected against editing (編集の制限)".into());
    }
    if doc.track_changes {
        return Err("the document records changes (変更履歴の記録), and the agent's edits are not recorded as changes: turn recording off first".into());
    }
    // Text deleted and not yet accepted is still in the paragraphs: an edit would count and find it as the text there.
    if has_revisions(doc) {
        return Err("the document has tracked changes not yet accepted or rejected (変更履歴): accept or reject them first".into());
    }
    Ok(())
}

/// Whether the document's body has a tracked change in it.
pub(super) fn has_revisions(doc: &FlowDocument) -> bool {
    fn any(blocks: &[Block]) -> bool {
        blocks.iter().any(|b| match b {
            Block::Paragraph(p) => p.mark_revision.is_some() || p.runs.iter().any(|r| r.revision.is_some()),
            Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).any(|c| any(&c.blocks)),
        })
    }
    any(&doc.body)
}

/// A paragraph style the summary offers: the document's, and Word's usual ones it does not have yet, defined when given.
pub(super) struct Offered {
    pub name: String,
    /// `None` for one the document has not defined.
    pub id: Option<String>,
}

/// The styles Word's style gallery always offers, by the names `styles.xml` gives them.
const BUILT_IN: [&str; 6] = ["Normal", "Title", "Subtitle", "heading 1", "heading 2", "heading 3"];

/// The paragraph styles offered, as the Word editor's style list orders them: Word's usual ones, then the document's
/// others (those it marks for the gallery, when it marks any), by their priority.
pub(super) fn offered_styles(styles: &FlowStyles) -> Vec<Offered> {
    let mut out: Vec<Offered> = BUILT_IN.iter().map(|name| Offered { name: shown(name), id: styles.paragraph_style_named(name).map(str::to_string) }).collect();
    let others = || styles.styles.iter().filter(|(_, s)| s.kind == StyleKind::Paragraph && !BUILT_IN.iter().any(|n| n.eq_ignore_ascii_case(&s.name)));
    let marked = others().any(|(_, s)| s.quick);
    let mut rest: Vec<(u32, String, &String)> = others().filter(|(_, s)| s.quick || !marked).map(|(id, s)| (if marked { s.ui_priority.unwrap_or(99) } else { 0 }, shown(&s.name), id)).collect();
    rest.sort();
    out.extend(rest.into_iter().map(|(_, name, id)| Offered { name, id: Some(id.clone()) }));
    out
}

/// A style's name as Word shows it in English (`styles.xml` writes its own in lower case: `heading 1`).
pub(super) fn shown(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

struct Planner<'a> {
    /// The document as the operations so far left it.
    scratch: Document,
    names: HashMap<String, u32>,
    commands: Vec<Command>,
    new_id: &'a mut dyn FnMut() -> u64,
    /// The last id `fresh` gave.
    high: u32,
}

impl Planner<'_> {
    fn doc(&self) -> &FlowDocument {
        self.scratch.presentation().flow.as_deref().expect("a document")
    }

    /// Apply `commands` to the scratch document (for what follows to see) and keep them for the editor, with what the
    /// Word editor adds to its own edits.
    fn run(&mut self, commands: Vec<FlowCommand>) -> Result<(), String> {
        let before = ibis_doc_model::flow_follow_up::before(self.doc(), &commands);
        self.execute(commands)?;
        let follow = ibis_doc_model::flow_follow_up::follow_up(&before, self.doc());
        self.execute(follow)
    }

    fn execute(&mut self, commands: Vec<FlowCommand>) -> Result<(), String> {
        let commands: Vec<Command> = commands.into_iter().map(Command::Flow).collect();
        self.scratch.execute(commands.clone()).map_err(|e| e.to_string())?;
        self.commands.extend(commands);
        Ok(())
    }

    /// An id for a paragraph or a table made: from the editor's counter, past every id the document has (and had when
    /// it was read: `id_floor`) and those given before.
    fn fresh(&mut self) -> Result<u32, String> {
        let past = (self.doc().new_id() - 1).max(self.high);
        loop {
            let id = u32::try_from((self.new_id)()).map_err(|_| "the document has run out of ids".to_string())?;
            if id > past {
                self.high = id;
                return Ok(id);
            }
        }
    }

    fn op(&mut self, op: &Op) -> Result<Created, String> {
        match op {
            Op::InsertParagraphs { after, before, text, style, name } => {
                let index = self.place(after.as_ref(), before.as_ref())?;
                let style = match style {
                    Some(name) => self.style(name)?,
                    None => None,
                };
                let mut template = FlowParagraph { style, ..FlowParagraph::default() };
                template.resolve(&self.doc().styles);
                let (blocks, fill) = self.paragraphs_of(&template, text, None, None)?;
                let made: Vec<u32> = blocks.iter().map(block_id).collect();
                let mut commands = vec![FlowCommand::ReplaceBlocks { container: Container::Body, index, remove: 0, blocks }];
                commands.extend(fill);
                self.run(commands)?;
                if let Some(name) = name {
                    self.names.insert(name.clone(), made[0]);
                }
                Ok(Created { paragraphs: made, ..Default::default() })
            }
            Op::SetText { paragraph, text } => {
                let id = self.paragraph(paragraph)?;
                let p = flow_command::paragraph(self.doc(), id).expect("resolved");
                if !p.runs.iter().all(|r| matches!(r.kind, FlowRunKind::Text(_) | FlowRunKind::Tab | FlowRunKind::LineBreak)) {
                    return Err(format!("paragraph {id} holds more than text (a picture, a field, a comment's mark…): change its words with replace_text"));
                }
                let len = p.len();
                let (format, char_style, _) = p.format_at(0);
                let mut commands = Vec::new();
                if len > 0 {
                    commands.push(FlowCommand::Delete { from: FlowPos::new(id, 0), to: FlowPos::new(id, len) });
                }
                if !text.0.is_empty() {
                    commands.push(FlowCommand::InsertText { at: FlowPos::new(id, 0), text: text.0.clone(), format: Some(InsertFormat { format, char_style }) });
                }
                self.run(commands)?;
                Ok(Created::default())
            }
            Op::Delete { blocks } => {
                let mut indexes = blocks.iter().map(|b| self.block(b)).collect::<Result<Vec<_>, _>>()?;
                indexes.sort_unstable();
                indexes.dedup();
                let body = &self.doc().body;
                for &i in &indexes {
                    if let Block::Paragraph(p) = &body[i]
                        && (p.section.is_some() || p.section_break != ibis_doc_model::flow::SectionBreak::None)
                    {
                        return Err(format!("paragraph {} ends a section: its page setup would go with it", p.id));
                    }
                }
                let last_kept = (0..body.len()).rev().find(|i| indexes.binary_search(i).is_err());
                if !last_kept.is_some_and(|i| matches!(body[i], Block::Paragraph(_))) {
                    return Err("a document ends with a paragraph (Word keeps one after a table at its end): this would leave none".into());
                }
                let commands = indexes.into_iter().rev().map(|index| FlowCommand::ReplaceBlocks { container: Container::Body, index, remove: 1, blocks: Vec::new() }).collect();
                self.run(commands)?;
                Ok(Created::default())
            }
            Op::ReplaceText { find, replace, paragraph } => {
                if find.is_empty() {
                    return Err("find is empty".into());
                }
                if has_line_break(find) || has_line_break(replace) {
                    return Err("find and replace are text within a paragraph: they hold no line break".into());
                }
                holdable(replace)?;
                let ids = match paragraph {
                    Some(r) => {
                        let index = self.block(r)?;
                        all_paragraphs(std::slice::from_ref(&self.doc().body[index]))
                    }
                    None => all_paragraphs(&self.doc().body),
                };
                let doc = self.doc();
                let n = find.chars().count();
                let mut commands = Vec::new();
                for id in ids {
                    let p = flow_command::paragraph(doc, id).expect("listed");
                    for at in occurrences(&super::inspect::text(p), find).into_iter().rev() {
                        // The format of the first character replaced, as 置換 takes it.
                        let (format, char_style, _) = p.format_at(at + 1);
                        commands.push(FlowCommand::Delete { from: FlowPos::new(id, at), to: FlowPos::new(id, at + n) });
                        if !replace.is_empty() {
                            commands.push(FlowCommand::InsertText { at: FlowPos::new(id, at), text: replace.clone(), format: Some(InsertFormat { format, char_style }) });
                        }
                    }
                }
                let count = commands.iter().filter(|c| matches!(c, FlowCommand::Delete { .. })).count();
                if count == 0 {
                    return Err(format!("{find:?} is not in the text"));
                }
                self.run(commands)?;
                Ok(Created { replaced: Some(count), ..Default::default() })
            }
            Op::FormatText { paragraph, table, cell, matching, format } => {
                let ids = match (paragraph, table, cell) {
                    (Some(p), None, None) => vec![self.paragraph(p)?],
                    (None, Some(t), Some([row, column])) => {
                        let table = self.table(t)?;
                        self.cell(table, *row, *column)?.iter().filter_map(|b| if let Block::Paragraph(p) = b { Some(p.id) } else { None }).collect()
                    }
                    (None, Some(_), None) => return Err("a table's text is named by its cell: give cell [row, column]".into()),
                    _ => return Err("give paragraph, or table and cell".into()),
                };
                let doc = self.doc();
                let mut ranges = Vec::new();
                for id in ids {
                    let p = flow_command::paragraph(doc, id).expect("listed");
                    match matching {
                        Some(m) if m.is_empty() => return Err("match is empty".into()),
                        Some(m) => ranges.extend(occurrences(&super::inspect::text(p), m).into_iter().map(|at| (FlowPos::new(id, at), FlowPos::new(id, at + m.chars().count())))),
                        None => ranges.push((FlowPos::new(id, 0), FlowPos::new(id, p.len()))),
                    }
                }
                if ranges.is_empty() {
                    return Err(format!("{:?} is not in the text", matching.as_deref().unwrap_or_default()));
                }
                let changes = run_changes(format)?;
                let commands = ranges.into_iter().flat_map(|(from, to)| changes.iter().map(move |change| FlowCommand::SetRunFormat { from, to, change: change.clone() })).collect();
                self.run(commands)?;
                Ok(Created::default())
            }
            Op::FormatParagraphs { paragraphs, format } => {
                format.check()?;
                let ids = self.paragraphs(paragraphs)?;
                let mut changes = Vec::new();
                if let Some(align) = format.align {
                    changes.push(ParaChange::Align(match align {
                        Align::Left => FlowAlign::Left,
                        Align::Center => FlowAlign::Center,
                        Align::Right => FlowAlign::Right,
                        Align::Justify => FlowAlign::Justify,
                    }));
                }
                if let Some(list) = format.list {
                    let bullet = match list {
                        List::None => None,
                        List::Bullet => Some(true),
                        List::Numbered => Some(false),
                    };
                    let change = match bullet {
                        None => ParaChange::List(None),
                        // The document's list of that kind, or a new one, as the toolbar's 箇条書き and 段落番号 do.
                        Some(bullet) => {
                            let numbering = &self.doc().styles.numbering;
                            match numbering.find(bullet) {
                                Some(id) => ParaChange::List(Some(id)),
                                None => {
                                    let (with, id) = numbering.with_new_list(bullet);
                                    self.run(vec![FlowCommand::SetNumbering { numbering: with }])?;
                                    ParaChange::List(Some(id))
                                }
                            }
                        }
                    };
                    changes.push(change);
                }
                let emu = |points: f64| (points * EMU_PER_POINT as f64).round() as i64;
                changes.extend(format.space_before.map(|p| ParaChange::SpaceBefore(emu(p))));
                changes.extend(format.space_after.map(|p| ParaChange::SpaceAfter(emu(p))));
                self.run(ids.iter().flat_map(|&id| changes.iter().map(move |change| FlowCommand::SetParagraphFormat { first: id, last: id, change: change.clone() })).collect())?;
                if let Some(level) = format.level {
                    let doc = self.doc();
                    if let Some(&id) = ids.iter().find(|&&id| flow_command::paragraph(doc, id).is_some_and(|p| p.list.is_none())) {
                        return Err(format!("paragraph {id} is not a list item: a level is a list item's (give list too)"));
                    }
                    self.run(ids.iter().map(|&id| FlowCommand::SetParagraphFormat { first: id, last: id, change: ParaChange::Level(level.into()) }).collect())?;
                }
                Ok(Created::default())
            }
            Op::SetStyle { paragraphs, style } => {
                let ids = self.paragraphs(paragraphs)?;
                let style = self.style(style)?;
                self.run(ids.into_iter().map(|id| FlowCommand::SetParagraphFormat { first: id, last: id, change: ParaChange::Style(style.clone()) }).collect())?;
                Ok(Created::default())
            }
            Op::AddTable { after, before, rows, name } => {
                let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
                if rows.is_empty() || columns == 0 {
                    return Err("a table needs at least one row and one column".into());
                }
                let index = self.place(after.as_ref(), before.as_ref())?;
                let doc = self.doc();
                let width = doc.page.size.width - doc.page.left - doc.page.right;
                // The table 挿入 > 表 makes, made on a copy before a paragraph, then given ids of its own and put in place.
                let (first, at) = doc.body.iter().enumerate().find_map(|(i, b)| if let Block::Paragraph(p) = b { Some((i, FlowPos::new(p.id, 0))) } else { None }).ok_or("the document has no paragraph")?;
                let mut built = doc.clone();
                apply_to(&mut built, FlowCommand::InsertTable { at, rows: rows.len(), columns, width }).map_err(|e| e.to_string())?;
                let mut table = built.body[first].clone();
                self.renumber(&mut table, &HashSet::new())?;
                let Block::Table(t) = &mut table else { unreachable!("InsertTable puts a table before the paragraph") };
                let id = t.id;
                let mut fill = Vec::new();
                for (r, line) in rows.iter().enumerate() {
                    for (c, text) in line.iter().enumerate() {
                        let cell = &mut t.rows[r].cells[c];
                        let Some(Block::Paragraph(template)) = cell.blocks.first().cloned() else { unreachable!("a new cell has its paragraph") };
                        let (blocks, commands) = self.paragraphs_of(&template, text, Some(template.id), None)?;
                        t.rows[r].cells[c].blocks = blocks;
                        fill.extend(commands);
                    }
                }
                let mut blocks = vec![table];
                // Word keeps a paragraph after a table at the end of a document.
                if index == self.doc().body.len() {
                    let mut end = FlowParagraph { id: self.fresh()?, ..FlowParagraph::default() };
                    end.resolve(&self.doc().styles);
                    blocks.push(Block::Paragraph(end));
                }
                let mut commands = vec![FlowCommand::ReplaceBlocks { container: Container::Body, index, remove: 0, blocks }];
                commands.extend(fill);
                self.run(commands)?;
                if let Some(name) = name {
                    self.names.insert(name.clone(), id);
                }
                Ok(Created { table: Some(id), ..Default::default() })
            }
            Op::SetCell { table, row, column, text } => {
                let table = self.table(table)?;
                let blocks = self.cell(table, *row, *column)?;
                let template = blocks.iter().find_map(|b| if let Block::Paragraph(p) = b { Some(p.clone()) } else { None }).ok_or("the cell has no paragraph")?;
                let remove = blocks.len();
                let (format, char_style, _) = template.format_at(0);
                let (blocks, fill) = self.paragraphs_of(&template, text, Some(template.id), Some(InsertFormat { format, char_style }))?;
                let mut commands = vec![FlowCommand::ReplaceBlocks { container: Container::Cell { table, row: *row, cell: *column }, index: 0, remove, blocks }];
                commands.extend(fill);
                self.run(commands)?;
                Ok(Created::default())
            }
            Op::InsertTableRow { table, at } => {
                let table = self.table(table)?;
                let rows = self.table_rows(table);
                let at = at.unwrap_or(rows);
                let edit = match at {
                    0 => TableEdit::InsertRow { row: 0, below: false },
                    at if at <= rows => TableEdit::InsertRow { row: at - 1, below: true },
                    at => return Err(format!("row {at} is past the end ({rows} rows)")),
                };
                // The row 行の挿入 makes, made on a copy, then given ids of its own: the copy numbers it past the ids the
                // document has now, which can be ids it had before.
                let index = self.doc().body.iter().position(|b| block_id(b) == table).ok_or("the table is not in the document's body")?;
                let mut kept = HashSet::new();
                ids_in(&self.doc().body[index], &mut kept);
                let mut built = self.doc().clone();
                apply_to(&mut built, FlowCommand::EditTable { table, edit }).map_err(|e| e.to_string())?;
                let mut edited = built.body[index].clone();
                self.renumber(&mut edited, &kept)?;
                self.run(vec![FlowCommand::ReplaceBlocks { container: Container::Body, index, remove: 1, blocks: vec![edited] }])?;
                Ok(Created::default())
            }
            Op::DeleteTableRow { table, at } => {
                let table = self.table(table)?;
                let rows = self.table_rows(table);
                if *at >= rows {
                    return Err(format!("no row {at} ({rows} rows)"));
                }
                if rows == 1 {
                    return Err("that is the table's only row: delete the table with delete".into());
                }
                self.run(vec![FlowCommand::EditTable { table, edit: TableEdit::DeleteRows { first: *at, last: *at } }])?;
                Ok(Created::default())
            }
        }
    }

    /// Empty paragraphs formatted as `template`, one for each of `text`'s, and the commands that then put their text in
    /// them (formatted as `format`; as the paragraph when not given). The first is `first` when given (the paragraph a
    /// cell keeps); the others are new, with ids of their own.
    fn paragraphs_of(&mut self, template: &FlowParagraph, text: &Text, first: Option<u32>, format: Option<InsertFormat>) -> Result<(Vec<Block>, Vec<FlowCommand>), String> {
        let mut blocks = Vec::new();
        let mut fill = Vec::new();
        for (i, line) in text.as_str().split('\n').enumerate() {
            let p = match (i, first) {
                (0, Some(id)) => FlowParagraph { id, runs: Vec::new(), ..template.clone() },
                _ => FlowParagraph {
                    id: self.fresh()?,
                    runs: Vec::new(),
                    section_break: ibis_doc_model::flow::SectionBreak::None,
                    section: None,
                    bookmarks: Vec::new(),
                    bookmark_ends: Vec::new(),
                    field_start: None,
                    field_end: false,
                    field_dirty: false,
                    subdoc: None,
                    mark_revision: None,
                    ..template.clone()
                },
            };
            if !line.is_empty() {
                fill.push(FlowCommand::InsertText { at: FlowPos::new(p.id, 0), text: line.to_string(), format: format.clone() });
            }
            blocks.push(Block::Paragraph(p));
        }
        Ok((blocks, fill))
    }

    /// Give `block`, and the rows, cells and blocks in it, fresh ids, but for those of `keep`.
    fn renumber(&mut self, block: &mut Block, keep: &HashSet<u32>) -> Result<(), String> {
        match block {
            Block::Paragraph(p) => {
                if !keep.contains(&p.id) {
                    p.id = self.fresh()?;
                }
            }
            Block::Table(t) => {
                if !keep.contains(&t.id) {
                    t.id = self.fresh()?;
                }
                for row in &mut t.rows {
                    if !keep.contains(&row.id) {
                        row.id = self.fresh()?;
                    }
                    for cell in &mut row.cells {
                        if !keep.contains(&cell.id) {
                            cell.id = self.fresh()?;
                        }
                        for b in &mut cell.blocks {
                            self.renumber(b, keep)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Where in the body a block goes: after `after`, before `before`, or at the end.
    fn place(&self, after: Option<&Ref>, before: Option<&Ref>) -> Result<usize, String> {
        match (after, before) {
            (Some(_), Some(_)) => Err("give after or before, not both".into()),
            (Some(r), None) => Ok(self.block(r)? + 1),
            (None, Some(r)) => self.block(r),
            (None, None) => Ok(self.doc().body.len()),
        }
    }

    /// The index in the body of the block `r` names.
    fn block(&self, r: &Ref) -> Result<usize, String> {
        let id = match r {
            Ref::Id(id) => *id,
            Ref::Name(name) => *self.names.get(name).ok_or_else(|| format!("nothing named {name:?} was made before"))?,
        };
        let doc = self.doc();
        doc.body.iter().position(|b| block_id(b) == id).ok_or_else(|| match flow_command::locate(doc, id) {
            Some(_) => format!("{id} is not a block of the body (a cell's paragraph, the header's…)"),
            None => format!("no block {id}"),
        })
    }

    fn paragraph(&self, r: &Ref) -> Result<u32, String> {
        match &self.doc().body[self.block(r)?] {
            Block::Paragraph(p) => Ok(p.id),
            Block::Table(t) => Err(format!("{} is a table, not a paragraph", t.id)),
        }
    }

    fn table(&self, r: &Ref) -> Result<u32, String> {
        match &self.doc().body[self.block(r)?] {
            Block::Table(t) => Ok(t.id),
            Block::Paragraph(p) => Err(format!("{} is a paragraph, not a table", p.id)),
        }
    }

    /// The paragraphs `refs` name, a table's being all those of its cells.
    fn paragraphs(&self, refs: &[Ref]) -> Result<Vec<u32>, String> {
        let mut ids = Vec::new();
        for r in refs {
            let index = self.block(r)?;
            ids.extend(all_paragraphs(std::slice::from_ref(&self.doc().body[index])));
        }
        Ok(ids)
    }

    fn table_rows(&self, table: u32) -> usize {
        match self.doc().body.iter().find(|b| block_id(b) == table) {
            Some(Block::Table(t)) => t.rows.len(),
            _ => 0,
        }
    }

    fn cell(&self, table: u32, row: usize, column: usize) -> Result<&[Block], String> {
        flow_command::blocks(self.doc(), Container::Cell { table, row, cell: column }).ok_or_else(|| {
            let rows = self.table_rows(table);
            match self.doc().body.iter().find(|b| block_id(b) == table) {
                Some(Block::Table(t)) if row < rows => format!("no cell [{row}, {column}] (row {row} has {} cells)", t.rows[row].cells.len()),
                _ => format!("no cell [{row}, {column}] (the table has {rows} rows)"),
            }
        })
    }

    /// The id of the style named `name` (`None`: the normal one), defining one of Word's usual styles the document does
    /// not have yet as Word's default style set has it.
    fn style(&mut self, name: &str) -> Result<Option<String>, String> {
        let offered = offered_styles(&self.doc().styles);
        let Some(chosen) = offered.into_iter().find(|s| s.name.eq_ignore_ascii_case(name)) else {
            let names: Vec<String> = offered_styles(&self.doc().styles).into_iter().map(|s| format!("{:?}", s.name)).collect();
            return Err(format!("no style named {name:?} (the styles: {})", names.join(", ")));
        };
        if let Some(id) = chosen.id {
            return Ok(Some(id));
        }
        if chosen.name.eq_ignore_ascii_case("Normal") {
            return Ok(None);
        }
        let mut defined = (*self.doc().styles).clone();
        StyleSetPreset::Default.apply(&mut defined);
        let id = defined.paragraph_style_named(name).expect("the default style set defines Word's usual styles").to_string();
        let style = defined.styles[&id].clone();
        self.run(vec![FlowCommand::SetStyle { id: id.clone(), style: Some(style) }])?;
        Ok(Some(id))
    }
}

/// The ids of `block` and of the rows, cells and blocks in it.
fn ids_in(block: &Block, ids: &mut HashSet<u32>) {
    ids.insert(block_id(block));
    if let Block::Table(t) = block {
        for row in &t.rows {
            ids.insert(row.id);
            for cell in &row.cells {
                ids.insert(cell.id);
                cell.blocks.iter().for_each(|b| ids_in(b, ids));
            }
        }
    }
}

fn block_id(block: &Block) -> u32 {
    match block {
        Block::Paragraph(p) => p.id,
        Block::Table(t) => t.id,
    }
}

/// The paragraphs of `blocks`, those in tables' cells too, in order.
fn all_paragraphs(blocks: &[Block]) -> Vec<u32> {
    let mut out = Vec::new();
    for block in blocks {
        match block {
            Block::Paragraph(p) => out.push(p.id),
            Block::Table(t) => {
                for cell in t.rows.iter().flat_map(|r| &r.cells) {
                    out.extend(all_paragraphs(&cell.blocks));
                }
            }
        }
    }
    out
}

fn run_changes(format: &TextFormat) -> Result<Vec<RunChange>, String> {
    let mut changes = Vec::new();
    changes.extend(format.bold.map(RunChange::Bold));
    changes.extend(format.italic.map(RunChange::Italic));
    changes.extend(format.underline.map(RunChange::Underline));
    if let Some(size) = format.size {
        changes.push(RunChange::Size(font_size(size)?));
    }
    if let Some(color) = &format.color {
        changes.push(RunChange::Color(rgb(color)?));
    }
    if let Some(font) = &format.font {
        // A Japanese typeface is set for both scripts, a Latin one for Latin text only, as the toolbar's font list does.
        changes.push(if font.is_ascii() && !ibis_doc_model::text::is_east_asian_typeface(font) { RunChange::Font(font.clone()) } else { RunChange::AllFonts(font.clone()) });
    }
    Ok(changes)
}
