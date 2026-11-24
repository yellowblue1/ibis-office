// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! What the agent's edits of a workbook ask of the editor beyond the commands (`agent`).

use gpui_kit::*;

use crate::editor::SlideEditor;

impl SlideEditor {
    /// Show the sheet the agent's edit worked on as a click on its tab does: through the workbook's own `show_page`
    /// (`ibis-office-xlsx`), so the sheet comes back scrolled and zoomed as it was left, not as the sheet before it was.
    pub(crate) fn agent_show_sheet(&mut self, slide: ibis_doc_model::SlideId, window: &mut Window, cx: &mut Context<Self>) {
        self.select_slide(slide, window, cx);
    }
}
