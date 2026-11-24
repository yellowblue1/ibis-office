// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The document as the agent reads it: the blocks of its body in order (paragraphs and tables), each with its id, and
//! the paragraph styles it can be given. The ids are the document's: they stay while it is open, and are what
//! [`Op`](crate::document::Op)s name; a file is numbered as it is read, so the same bytes give the same ids. An id
//! once given is not given again while the file is open, even after its block is deleted.

use ibis_doc_model::Presentation;
use ibis_doc_model::flow::{Block, FlowDocument, FlowParagraph, FlowStyles, StyleKind};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Inspection {
    pub name: String,
    /// Whether there are edits not saved yet.
    pub dirty: bool,
    /// The paragraph styles `set_style` and `insert_paragraphs` take, by name.
    pub styles: Vec<String>,
    pub blocks: Vec<BlockInfo>,
    /// Whether the document has tracked changes (変更履歴) not yet accepted or rejected: it is shown as it reads once
    /// they are accepted, and is not edited until the person accepts or rejects them.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub revisions: bool,
    /// The paragraphs of the header and the footer (of the last section's pages), read only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub header: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub footer: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BlockInfo {
    pub id: u32,
    /// `paragraph` or `table`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// A paragraph's text (a line break in it, Shift+Enter's, as `\n`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A paragraph's style, when it is not the normal one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// `bullet` or `numbered`, for a list item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list: Option<&'static str>,
    /// A list item's level, 0 to 8.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    /// A table's cells' paragraphs, by row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cells: Option<Vec<Vec<Vec<String>>>>,
}

pub fn inspect(model: &Presentation, name: &str, dirty: bool) -> Inspection {
    let Some(doc) = model.flow.as_deref() else {
        return Inspection { name: name.to_string(), dirty, styles: Vec::new(), blocks: Vec::new(), revisions: false, header: Vec::new(), footer: Vec::new() };
    };
    Inspection {
        name: name.to_string(),
        dirty,
        styles: super::ops::offered_styles(&doc.styles).into_iter().map(|s| s.name).collect(),
        blocks: doc.body.iter().map(|b| block_info(doc, b)).collect(),
        revisions: super::ops::has_revisions(doc),
        header: paragraphs(&doc.header),
        footer: paragraphs(&doc.footer),
    }
}

fn block_info(doc: &FlowDocument, block: &Block) -> BlockInfo {
    match block {
        Block::Paragraph(p) => {
            let list = p.list.map(|l| if doc.styles.numbering.level(l.num_id, 0).is_some_and(|level| level.format == "bullet") { "bullet" } else { "numbered" });
            BlockInfo { id: p.id, kind: "paragraph", text: Some(text(p)), style: style_name(&doc.styles, p), list, level: p.list.map(|l| l.level), cells: None }
        }
        Block::Table(t) => BlockInfo {
            id: t.id,
            kind: "table",
            text: None,
            style: None,
            list: None,
            level: None,
            cells: Some(t.rows.iter().map(|row| row.cells.iter().map(|cell| paragraphs(&cell.blocks)).collect()).collect()),
        },
    }
}

/// The text of the paragraphs of `blocks` (a table among them left out).
fn paragraphs(blocks: &[Block]) -> Vec<String> {
    blocks.iter().filter_map(|b| if let Block::Paragraph(p) = b { Some(text(p)) } else { None }).collect()
}

/// One character for each of the paragraph's positions, as the operations count them. Text a tracked change deleted is
/// left out (the document as it reads once the changes are accepted); such a document is not edited, so the positions
/// it would shift are never counted.
pub(super) fn text(p: &FlowParagraph) -> String {
    let text = if p.runs.iter().any(|r| r.revision.as_ref().is_some_and(|v| v.kind.is_delete())) {
        FlowParagraph { runs: p.runs.iter().filter(|r| !r.revision.as_ref().is_some_and(|v| v.kind.is_delete())).cloned().collect(), ..p.clone() }.text()
    } else {
        p.text()
    };
    text.replace('\u{b}', "\n")
}

fn style_name(styles: &FlowStyles, p: &FlowParagraph) -> Option<String> {
    let id = p.style.as_deref()?;
    if Some(id) == styles.default_paragraph.as_deref() {
        return None;
    }
    styles.styles.get(id).filter(|s| s.kind == StyleKind::Paragraph).map(|s| super::ops::shown(&s.name))
}
