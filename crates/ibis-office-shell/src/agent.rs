// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! An AI agent's edits on the document open in the editor (`ibis_agent`): what the browser version's page answers to
//! the page that embeds it (`www/main.ts`, `ibisoffice-agent` messages). An edit of the agent's goes through the
//! editor's undo as the person's do, as one step, so ⌘Z takes it back whole; what the person was typing is entered
//! first, as a save does.

use std::sync::Arc;

use gpui_kit::*;
use gpui_kit::component::Root;

use crate::editor::SlideEditor;
use crate::format::with_format;

impl SlideEditor {
    /// The document as the agent reads it (`ibis_agent::inspect`), in JSON.
    pub fn agent_inspect(&self) -> Result<String, String> {
        let source = self.agent_source()?;
        Ok(serde_json::to_string(&self.agent_summary(&source)).expect("serialisable"))
    }

    /// Whether there are changes not saved, and a count that changes with every edit, undo and redo, in JSON: cheap, for
    /// the embedding page to save the person's edits once they stop.
    pub fn agent_state(&self) -> Result<String, String> {
        self.agent_source()?;
        Ok(serde_json::json!({ "dirty": self.document.is_dirty(), "revision": self.document.revision() }).to_string())
    }

    /// Apply the operations of `ops` (JSON) as one edit, and show the slide they worked on. Answers what each made and
    /// the document after, in JSON; nothing changes when they are refused.
    pub fn agent_apply(&mut self, ops: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<String, String> {
        let source = self.agent_source()?;
        let ops = ibis_agent::parse_ops(source.kind(), ops)?;
        if self.is_read_only() {
            return Err("the document is open read-only".into());
        }
        self.commit_pending_edit(window, cx);
        let current = self.presentation().clone();
        let ids = self.document.ids();
        let planned = ibis_agent::plan(&ops, &current, &source, &mut || ids.new_shape_id().0).map_err(|e| e.to_string())?;
        // A list that makes no command (an empty one) is done, changing nothing; only a change the editor drops is refused.
        if !planned.commands.is_empty() {
            let steps = self.document.undo_len();
            self.execute_unrepeated(planned.commands, cx);
            if self.document.undo_len() == steps {
                return Err("the editor refused the change (the document is marked as final, or read-only)".into());
            }
            with_format!(self, edited_from_outside(self));
        }
        if let Some(slide) = planned.shown
            && slide != self.current_slide
        {
            // A sheet is shown as a click on its tab shows it (the workbook's `show_page`): scrolled and zoomed as it was left.
            if source.kind() == ibis_pptx::Kind::Workbook {
                self.select_slide(slide, window, cx);
            } else {
                self.selection = None;
                self.set_current_slide(slide);
                with_format!(self, clear_page_selection(self));
            }
        }
        cx.notify();
        let document = self.agent_summary(&source);
        Ok(serde_json::json!({ "created": planned.created, "document": document }).to_string())
    }

    /// The file as it is now, its name, and the document marked saved. What the person is typing is in the model as it is
    /// typed, so the save does not end their typing (the embedding page saves while they work); only text still being
    /// composed with an input method is left out.
    pub fn agent_save(&mut self, cx: &mut Context<Self>) -> Result<(String, Vec<u8>), String> {
        let source = self.agent_source()?;
        if let Some(why) = with_format!(self, save_refused(self)) {
            return Err(why);
        }
        let bytes = source.save(&self.presentation_for_save(), ibis_pptx::SaveOptions::default()).map_err(|e| e.to_string())?;
        self.document.mark_clean();
        cx.notify();
        Ok((self.file_name.as_deref().unwrap_or_default().to_string(), bytes))
    }

    /// Where the person is (`Place`), in JSON, when it is not the place last told or shown: the embedding page shows it in
    /// the other editor of two side by side. A place this editor was asked to show is not told back, so two editors that
    /// follow each other do not answer each other's moves.
    pub fn agent_place_changed(&mut self) -> Option<String> {
        let place = serde_json::to_string(&self.agent_place()?).expect("serialisable");
        if self.agent_told_place.as_deref() == Some(place.as_str()) {
            return None;
        }
        self.agent_told_place = Some(place.clone());
        Some(place)
    }

    /// Show `params` (`Show`, JSON): its `top` at the view's top edge, its `at` selected, also in a read-only editor.
    pub fn agent_show(&mut self, params: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        if !self.has_document {
            return Err("no file is open".into());
        }
        let show: Show = serde_json::from_str(params).map_err(|e| format!("the place to show: {e}"))?;
        if show.top.is_none() && show.at.is_none() {
            return Ok(());
        }
        with_format!(self, agent_show(self, show.top, show.at, window, cx))?;
        cx.notify();
        self.agent_told_place = self.agent_place().map(|p| serde_json::to_string(&p).expect("serialisable"));
        Ok(())
    }

    /// Where the person is: what is at the view's top edge, and their caret or selection (`Place`).
    pub fn agent_place(&self) -> Option<Place> {
        if !self.has_document {
            return None;
        }
        with_format!(self, agent_place(self))
    }

    fn agent_summary(&self, source: &ibis_pptx::Source) -> ibis_agent::Summary {
        ibis_agent::inspect(source.kind(), self.presentation(), self.file_name.as_deref().unwrap_or_default(), self.document.is_dirty())
    }

    /// The file the document was opened from: only an opened file is edited by the agent.
    fn agent_source(&self) -> Result<Arc<ibis_pptx::Source>, String> {
        match &self.source {
            Some(source) if self.has_document => Ok(source.clone()),
            None if self.has_document => Err("the file open is a new one, not opened from a file: the agent edits a file that was opened".into()),
            _ => Err("no file is open".into()),
        }
    }
}

/// A place in the open file by index, the same in two files of the same structure (a translation made from a copy of the
/// original): the Nth block of a document's body, the Nth slide and its Nth shape as `inspect` lists them, the Nth sheet
/// of a workbook (hidden ones counted) and a cell. docs/web-agent.md, "Two editors side by side".
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Place {
    pub top: Top,
    pub at: Option<At>,
}

/// What is at the view's top edge.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum Top {
    /// `offset`: how far down into the block (by its laid-out height) the edge is, 0 to below 1.
    Block {
        block: usize,
        #[serde(default)]
        offset: f64,
    },
    Slide { slide: usize },
    /// The top-left cell of the part that scrolls, 1-based.
    Cell { sheet: usize, row: u32, column: u32 },
}

/// Where the person's caret or selection is.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum At {
    Block { block: usize },
    /// `shape`: the selected shape, `None` when none is.
    Shape { slide: usize, shape: Option<usize> },
    /// The active cell, 1-based.
    Cell { sheet: usize, row: u32, column: u32 },
}

impl Top {
    /// Rounded as it is told, so that a place read back after it was shown is the one told.
    pub fn block(block: usize, offset: f64) -> Self {
        Top::Block { block, offset: ((offset * 1000.0).floor() / 1000.0).clamp(0.0, 0.999) }
    }
}

/// A `show` request's `params`.
#[derive(serde::Deserialize)]
struct Show {
    #[serde(default)]
    top: Option<Top>,
    #[serde(default)]
    at: Option<At>,
}

/// A place given to a file of another kind.
pub fn wrong_kind(what: &str) -> String {
    format!("the place is not one of a {what}")
}

/// The editor of the page and its window (the browser version has one editor in a page).
pub fn page_editor(cx: &mut App) -> Option<(AnyWindowHandle, Entity<SlideEditor>)> {
    cx.windows().into_iter().find_map(|window| {
        let root = window.downcast::<Root>()?;
        let editor = root.update(cx, |root, _, _| root.view().clone().downcast::<SlideEditor>().ok()).ok().flatten()?;
        Some((window, editor))
    })
}

/// Run `f` on the page's editor, in its window.
pub fn with_page_editor<R>(cx: &mut App, f: impl FnOnce(&mut SlideEditor, &mut Window, &mut Context<SlideEditor>) -> Result<R, String>) -> Result<R, String> {
    let (window, editor) = page_editor(cx).ok_or("the editor has not started")?;
    window.update(cx, |_, window, cx| editor.update(cx, |editor, cx| f(editor, window, cx))).map_err(|e| e.to_string())?
}

/// Open `opened`, a file named `name`, in a new editor window, saving to it as a file opened with ファイル > 開く does.
pub fn open_editor_with_source(opened: ibis_pptx::Opened, name: SharedString, cx: &mut App) {
    crate::app::open_window_with(cx, move |editor, window, cx| {
        editor.load(opened.presentation, name, window, cx);
        editor.set_loaded_source(Arc::new(opened.source));
        with_format!(editor, file_opened(editor));
    });
}
