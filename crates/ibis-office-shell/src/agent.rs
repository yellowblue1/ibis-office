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
        self.agent_source()?;
        let name = self.file_name.as_deref().unwrap_or_default();
        Ok(serde_json::to_string(&ibis_agent::inspect(self.presentation(), name, self.document.is_dirty())).expect("serialisable"))
    }

    /// Apply the operations of `ops` (JSON) as one edit, and show the slide they worked on. Answers what each made and
    /// the document after, in JSON; nothing changes when they are refused.
    pub fn agent_apply(&mut self, ops: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<String, String> {
        let source = self.agent_source()?;
        let ops = ibis_agent::parse_ops(ops)?;
        if self.is_read_only() {
            return Err("the document is open read-only".into());
        }
        self.commit_pending_edit(window, cx);
        let current = self.presentation().clone();
        let ids = self.document.ids();
        let planned = ibis_agent::plan(&ops, &current, &source, &mut || ids.new_shape_id().0).map_err(|e| e.to_string())?;
        let steps = self.document.undo_len();
        self.execute(planned.commands, cx);
        if self.document.undo_len() == steps {
            return Err("the editor refused the change (the document is marked as final, or read-only)".into());
        }
        if let Some(slide) = planned.shown
            && slide != self.current_slide
        {
            self.selection = None;
            self.set_current_slide(slide);
            with_format!(self, clear_page_selection(self));
        }
        cx.notify();
        let document = ibis_agent::inspect(self.presentation(), self.file_name.as_deref().unwrap_or_default(), self.document.is_dirty());
        Ok(serde_json::json!({ "created": planned.created, "document": document }).to_string())
    }

    /// The file as it is now (what is being typed entered first), its name, and the document marked saved.
    pub fn agent_save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(String, Vec<u8>), String> {
        let source = self.agent_source()?;
        self.commit_pending_edit(window, cx);
        if let Some(why) = with_format!(self, save_refused(self)) {
            return Err(why);
        }
        let bytes = source.save(&self.presentation_for_save(), ibis_pptx::SaveOptions::default()).map_err(|e| e.to_string())?;
        self.document.mark_clean();
        cx.notify();
        Ok((self.file_name.as_deref().unwrap_or_default().to_string(), bytes))
    }

    /// The file the document was opened from: only an opened presentation is edited by the agent.
    fn agent_source(&self) -> Result<Arc<ibis_pptx::Source>, String> {
        match &self.source {
            Some(source) if source.kind() == ibis_pptx::Kind::Presentation && self.has_document => Ok(source.clone()),
            Some(_) => Err("only presentations (.pptx) can be edited by the agent yet".into()),
            None => Err("no document is open".into()),
        }
    }
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
