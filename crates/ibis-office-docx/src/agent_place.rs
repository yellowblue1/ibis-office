// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! A document's place for the embedding page's two editors side by side (`shell::agent::Place`): the block of the body at
//! the view's top edge and the one the caret is in, by index, a table being one block (as `inspect` lists them). A block
//! is as tall as its laid-out lines, from its first line's top to its last line's bottom (on several pages: the page break
//! between counted).

use std::collections::HashMap;

use gpui_kit::*;
use ibis_doc_model::{Block, EMU_PER_POINT, FlowPos};

use crate::doc_ui::page_place;
use crate::ext::*;
use crate::flow_layout::{Item, pages};
use crate::shell::agent::{At, Place, Top, wrong_kind};
use crate::shell::editor::SlideEditor;

/// The paragraphs of `block` in reading order, a table's cells' (and their tables') too.
fn paragraphs(block: &Block, out: &mut Vec<u32>) {
    match block {
        Block::Paragraph(p) => out.push(p.id),
        Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).flat_map(|c| &c.blocks).for_each(|b| paragraphs(b, out)),
    }
}

/// Each body block's band in the content (pixels, top and bottom), `None` for one with no line laid out.
fn bands(editor: &SlideEditor) -> Option<Vec<Option<(f32, f32)>>> {
    if editor.is_page_movement_side_to_side() {
        return None;
    }
    let doc = editor.flow()?;
    let content = editor.workspace_content()?;
    let mut owner = HashMap::new();
    for (index, block) in doc.body.iter().enumerate() {
        let mut ids = Vec::new();
        paragraphs(block, &mut ids);
        owner.extend(ids.into_iter().map(|id| (id, index)));
    }
    let sizes = editor.page_sizes();
    let k = content.scale * EMU_PER_POINT as f32;
    let mut bands: Vec<Option<(f32, f32)>> = vec![None; doc.body.len()];
    for (index, page) in pages(doc, &editor.resources).pages.iter().enumerate() {
        let top = page_place(&content, &sizes, index, false).1;
        for item in &page.items {
            // Lines of the header, the footer, notes and text boxes are of paragraphs outside the body's blocks.
            let Item::Line { para, y, h, .. } = item else { continue };
            let Some(&block) = owner.get(para) else { continue };
            let (from, to) = (top + y * k, top + (y + h) * k);
            let band = bands[block].get_or_insert((from, to));
            *band = (band.0.min(from), band.1.max(to));
        }
    }
    Some(bands)
}

/// The block nearest `index` that is laid out, after it first.
fn laid_out(bands: &[Option<(f32, f32)>], index: usize) -> Option<(usize, (f32, f32))> {
    let index = index.min(bands.len().checked_sub(1)?);
    let after = (index..bands.len()).find_map(|i| bands[i].map(|b| (i, b)));
    after.or_else(|| (0..index).rev().find_map(|i| bands[i].map(|b| (i, b))))
}

/// The top of the view and its height, in the content (pixels).
fn view(editor: &SlideEditor) -> Option<(f32, f32)> {
    let ws = editor.workspace_bounds?;
    let height = f32::from(ws.size.height);
    let height = if editor.doc_split().is_some() { height / 2.0 } else { height };
    Some((-f32::from(editor.doc_pane_scroll().offset().y), height))
}

fn block_of(editor: &SlideEditor, para: u32) -> Option<usize> {
    editor.flow()?.body.iter().position(|block| {
        let mut ids = Vec::new();
        paragraphs(block, &mut ids);
        ids.contains(&para)
    })
}

pub(crate) fn place(editor: &SlideEditor) -> Option<Place> {
    let bands = bands(editor)?;
    let (view_top, _) = view(editor)?;
    // The first block not wholly above the view's top edge; an edge in a gap (between paragraphs, a page's margins, the
    // space between pages) is at the top of the block below it.
    let top = bands.iter().enumerate().find_map(|(i, b)| b.filter(|&(_, bottom)| bottom > view_top).map(|b| (i, b)));
    let top = match top {
        Some((block, (from, to))) => Top::block(block, if from < view_top { f64::from((view_top - from) / (to - from)) } else { 0.0 }),
        None => {
            let (block, _) = laid_out(&bands, bands.len().checked_sub(1)?)?;
            Top::block(block, 1.0)
        }
    };
    let at = editor.ordered_selection().and_then(|(start, _)| block_of(editor, start.para)).map(|block| At::Block { block });
    Some(Place { top, at })
}

pub(crate) fn show(editor: &mut SlideEditor, top: Option<Top>, at: Option<At>, _window: &mut Window, cx: &mut Context<SlideEditor>) -> Result<(), String> {
    let top = match top {
        None => None,
        Some(Top::Block { block, offset }) => Some((block, offset)),
        Some(_) => return Err(wrong_kind("document")),
    };
    let at = match at {
        None => None,
        Some(At::Block { block }) => Some(block),
        Some(_) => return Err(wrong_kind("document")),
    };
    let bands = bands(editor).ok_or("the document's pages are not laid out yet (or they move side to side)")?;
    let Some(doc) = editor.flow().cloned() else { return Err("no document is open".into()) };
    if let Some((block, offset)) = top
        && let Some((_, (from, to))) = laid_out(&bands, block)
    {
        editor.scroll_flow_to(from + offset.clamp(0.0, 1.0) as f32 * (to - from));
    }
    if let Some(block) = at
        && let Some(last) = doc.body.len().checked_sub(1)
    {
        let mut ids = Vec::new();
        paragraphs(&doc.body[block.min(last)], &mut ids);
        if let (Some(&first), Some(&end)) = (ids.first(), ids.last()) {
            let length = editor.paragraph_of(end).map_or(0, |p| p.len());
            editor.set_caret(FlowPos::new(first, 0), false, cx);
            editor.set_caret(FlowPos::new(end, length), true, cx);
        }
        // Brought into view only when the view is not put elsewhere: its start at the top when it was out of view.
        if top.is_none()
            && let (Some((_, (from, _))), Some((view_top, height))) = (laid_out(&bands, block), view(editor))
            && (from < view_top || from >= view_top + height)
        {
            editor.scroll_flow_to(from);
        }
    }
    Ok(())
}
