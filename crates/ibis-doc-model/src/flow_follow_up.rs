// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! What an edit of a document does besides its own commands, in the same step of undo: the comments and notes whose
//! marks it removed go with them, as Word does, and the paragraph styles that update themselves (スタイルを自動的に
//! 更新する, `w:autoRedefine`) take what it changed by hand. The Word editor and an agent's edits both add these, so the
//! two leave a document alike.

use std::collections::HashSet;

use crate::flow::{Block, FlowDocument, FlowRunKind};
use crate::flow_command::{FlowCommand, ParaChange, apply_to, delete_comment, locate, paragraph, style_matching};

/// What [`follow_up`] needs of the document as it was before the edit.
#[derive(Clone, Debug)]
pub struct Before {
    notes: HashSet<(bool, u32)>,
    comments: HashSet<u32>,
    redefine: Vec<u32>,
}

/// What the document is before `commands` are applied to it.
pub fn before(doc: &FlowDocument, commands: &[FlowCommand]) -> Before {
    let (notes, comments) = marked(doc);
    Before { notes, comments, redefine: redefine_targets(doc, commands) }
}

/// The commands that follow the edit in its step, on `doc` as the edit left it: comments, then notes, whose marks it
/// removed (only those marked before it: what a file has unmarked stays), then the styles of the paragraphs it formatted
/// by hand.
pub fn follow_up(before: &Before, doc: &FlowDocument) -> Vec<FlowCommand> {
    let mut doc = doc.clone();
    let mut out = Vec::new();
    // A group is given as its commands, all of them or none: they join the edit's step anyway.
    let mut push = |doc: &mut FlowDocument, command: FlowCommand| {
        let parts = match command {
            FlowCommand::Many(parts) => parts,
            command => vec![command],
        };
        let mut next = doc.clone();
        if parts.iter().any(|c| apply_to(&mut next, c.clone()).is_err()) {
            return false;
        }
        *doc = next;
        out.extend(parts);
        true
    };
    // A comment's replies go with it, which can unmark no other: a few rounds settle it.
    for _ in 0..8 {
        let (_, now) = marked(&doc);
        let mut lost: Vec<u32> = before.comments.difference(&now).copied().filter(|i| doc.comments.iter().any(|c| c.id == *i)).collect();
        lost.sort_unstable();
        let Some(command) = lost.first().and_then(|&id| delete_comment(&doc, id)) else { break };
        if !push(&mut doc, command) {
            break;
        }
    }
    let (now, _) = marked(&doc);
    let lost: HashSet<(bool, u32)> = before.notes.difference(&now).copied().collect();
    if !lost.is_empty() {
        let footnotes = doc.footnotes.iter().filter(|n| !lost.contains(&(false, n.id))).cloned().collect();
        let endnotes = doc.endnotes.iter().filter(|n| !lost.contains(&(true, n.id))).cloned().collect();
        push(&mut doc, FlowCommand::SetNotes { footnotes, endnotes });
    }
    for &id in &before.redefine {
        let Some((style_id, style, paragraph)) = style_matching(&doc, id, 0) else { continue };
        let Some((container, index)) = locate(&doc, id) else { continue };
        let redefine = FlowCommand::Many(vec![FlowCommand::SetStyle { id: style_id, style: Some(style) }, FlowCommand::ReplaceBlocks { container, index, remove: 1, blocks: vec![Block::Paragraph(paragraph)] }]);
        if !push(&mut doc, redefine) {
            break;
        }
    }
    out
}

/// The notes the body marks, and the comments it marks whole (start, end and reference).
fn marked(doc: &FlowDocument) -> (HashSet<(bool, u32)>, HashSet<u32>) {
    let mut notes = HashSet::new();
    if doc.footnotes.is_empty() && doc.endnotes.is_empty() && doc.comments.is_empty() {
        return (notes, HashSet::new());
    }
    fn walk(blocks: &[Block], notes: &mut HashSet<(bool, u32)>, marks: &mut std::collections::HashMap<u32, [bool; 3]>) {
        for b in blocks {
            match b {
                Block::Paragraph(p) => {
                    for r in &p.runs {
                        match r.kind {
                            FlowRunKind::NoteRef { endnote, id } => {
                                notes.insert((endnote, id));
                            }
                            FlowRunKind::CommentStart(i) => marks.entry(i).or_default()[0] = true,
                            FlowRunKind::CommentEnd(i) => marks.entry(i).or_default()[1] = true,
                            FlowRunKind::CommentRef(i) => marks.entry(i).or_default()[2] = true,
                            _ => {}
                        }
                    }
                }
                Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).for_each(|c| walk(&c.blocks, notes, marks)),
            }
        }
    }
    let mut marks = std::collections::HashMap::new();
    walk(&doc.body, &mut notes, &mut marks);
    (notes, marks.into_iter().filter(|(_, m)| *m == [true; 3]).map(|(i, _)| i).collect())
}

/// The paragraphs whose formatting `commands` change by hand (a paragraph's formatting, or a character formatting over a
/// paragraph's whole text) and whose style updates itself.
fn redefine_targets(doc: &FlowDocument, commands: &[FlowCommand]) -> Vec<u32> {
    if !doc.styles.styles.values().any(|s| s.auto_update) {
        return Vec::new();
    }
    let order = body_order(doc);
    let mut out = Vec::new();
    for command in commands {
        match command {
            FlowCommand::SetRunFormat { from, to, .. } if from.para == to.para && from.offset == 0 && paragraph(doc, from.para).is_some_and(|p| p.len() == to.offset && to.offset > 0) => out.push(from.para),
            FlowCommand::SetParagraphFormat { first, last, change } if !matches!(change, ParaChange::Style(_) | ParaChange::List(_) | ParaChange::Level(_) | ParaChange::AddBookmark(_) | ParaChange::RemoveBookmark(_) | ParaChange::SectionBreak(_)) => {
                if let (Some(a), Some(b)) = (order.iter().position(|p| p == first), order.iter().position(|p| p == last)) {
                    out.extend(order[a.min(b)..=a.max(b)].iter().copied());
                }
            }
            _ => {}
        }
    }
    out.sort_unstable();
    out.dedup();
    out.retain(|&id| paragraph(doc, id).and_then(|p| p.style.clone().or_else(|| doc.styles.default_paragraph.clone())).and_then(|s| doc.styles.styles.get(&s)).is_some_and(|s| s.auto_update));
    out
}

/// The body's paragraphs, those in tables' cells too, in order.
pub fn body_order(doc: &FlowDocument) -> Vec<u32> {
    fn walk(blocks: &[Block], out: &mut Vec<u32>) {
        for b in blocks {
            match b {
                Block::Paragraph(p) => out.push(p.id),
                Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).for_each(|c| walk(&c.blocks, out)),
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.body, &mut out);
    out
}
