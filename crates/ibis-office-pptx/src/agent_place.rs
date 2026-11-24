// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! A presentation's place for the embedding page's two editors side by side (`shell::agent::Place`): the slide shown and
//! the shape selected on it, by index, the shapes counted as `inspect` lists them.

use gpui_kit::*;
use ibis_doc_model::{ShapeId, Slide};

use crate::shell::agent::{At, Place, Top, wrong_kind};
use crate::shell::editor::SlideEditor;

/// The shapes of `slide` as `ibis_agent::inspect` lists them (a shape attached to another, a connector's label, is not).
fn listed(slide: &Slide) -> impl Iterator<Item = ShapeId> + '_ {
    slide.shapes.iter().filter(|s| !slide.is_attached(s.id)).map(|s| s.id)
}

pub(crate) fn place(editor: &SlideEditor) -> Option<Place> {
    let slide = editor.presentation().slide_index(editor.current_slide)?;
    let selected = editor.selection.or_else(|| editor.text_editing().map(|e| e.shape));
    let shape = selected.and_then(|id| listed(editor.slide()).position(|s| s == id));
    Some(Place { top: Top::Slide { slide }, at: Some(At::Shape { slide, shape }) })
}

pub(crate) fn show(editor: &mut SlideEditor, top: Option<Top>, at: Option<At>, window: &mut Window, cx: &mut Context<SlideEditor>) -> Result<(), String> {
    let top = match top {
        None => None,
        Some(Top::Slide { slide }) => Some(slide),
        Some(_) => return Err(wrong_kind("presentation")),
    };
    let at = match at {
        None => None,
        Some(At::Shape { slide, shape }) => Some((slide, shape)),
        Some(_) => return Err(wrong_kind("presentation")),
    };
    let last = editor.presentation().slides.len().checked_sub(1).ok_or("the presentation has no slides")?;
    let Some(shown) = top.or(at.map(|(slide, _)| slide)).map(|n| n.min(last)) else { return Ok(()) };
    let id = editor.presentation().slides[shown].id;
    if editor.current_slide != id {
        editor.select_slide(id, window, cx);
    }
    // A shape is selected only on the slide shown: one of another slide could not be seen.
    if let Some((slide, shape)) = at
        && slide.min(last) == shown
    {
        if editor.text_editing().is_some() {
            editor.commit_edit(window, cx);
        }
        let selected = shape.and_then(|m| listed(editor.slide()).nth(m));
        editor.set_selection(selected.into_iter().collect());
    }
    Ok(())
}
