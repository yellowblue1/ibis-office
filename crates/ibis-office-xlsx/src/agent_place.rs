// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! A workbook's place for the embedding page's two editors side by side (`shell::agent::Place`): the sheet shown by its
//! index in the workbook's order (hidden sheets counted, as `inspect`'s `number`), the top-left cell of the part of it
//! that scrolls (the frozen or split rows and columns left out) and the active cell, 1-based.

use gpui_kit::*;
use ibis_doc_model::{Emu, SlideId};

use crate::ext::*;
use crate::shell::agent::{At, Place, Top, wrong_kind};
use crate::shell::editor::SlideEditor;
use crate::sheet_ui::{COLUMN_HEADER_HEIGHT, MAX_SHEET_COLUMNS, MAX_SHEET_ROWS, row_header_width};

/// The top edge of row (or column) `index`, in EMU from the sheet's: `offsets` past their end go on at `default` each.
fn edge(offsets: &[Emu], default: Emu, index: u32) -> Emu {
    let last = offsets.len().saturating_sub(1);
    match offsets.get(index as usize) {
        Some(&at) => at,
        None => offsets.last().copied().unwrap_or(0) + (index as usize - last) as Emu * default,
    }
}

/// The row (or column) whose band holds `at` (EMU from the sheet's edge): of hidden ones, which take no room, the one after.
fn index_at(offsets: &[Emu], default: Emu, at: f32) -> u32 {
    let last = offsets.len().saturating_sub(1);
    let end = offsets.last().copied().unwrap_or(0);
    if at >= end as f32 {
        return (last as f32 + ((at - end as f32) / default.max(1) as f32).floor()) as u32;
    }
    offsets.partition_point(|&o| o as f32 <= at).saturating_sub(1) as u32
}

/// The view of the sheet shown: for each axis (x, y), where the part that scrolls starts in the window's content
/// (pixels: the headers, the frozen or split part), the sheet's edge there, the size of the view, of the content, and
/// pixels per EMU.
struct Geometry {
    head: Point<f32>,
    origin: Point<f32>,
    view: Size<f32>,
    content: Size<f32>,
    scale: f32,
}

fn geometry(editor: &SlideEditor) -> Option<Geometry> {
    let (content, ws, (shape, sheet)) = (editor.workspace_content()?, editor.workspace_bounds?, editor.current_sheet()?);
    let (split_x, split_y) = editor.frozen_split(content).unwrap_or((None, None));
    let head_w = if sheet.headings { row_header_width(sheet) } else { 0.0 };
    let head_h = if sheet.headings { COLUMN_HEADER_HEIGHT } else { 0.0 };
    Some(Geometry {
        head: point(split_x.unwrap_or(head_w), split_y.unwrap_or(head_h)),
        origin: point(
            f32::from(content.slide_origin.x) + shape.frame.x as f32 * content.scale,
            f32::from(content.slide_origin.y) + shape.frame.y as f32 * content.scale,
        ),
        view: size(f32::from(ws.size.width), f32::from(ws.size.height)),
        content: size(f32::from(content.size.width), f32::from(content.size.height)),
        scale: content.scale,
    })
}

/// The workbook's index of the sheet on `slide`.
fn sheet_index(editor: &SlideEditor, slide: SlideId) -> Option<usize> {
    editor.presentation().workbook.as_ref()?.sheets.iter().position(|s| s.slide == Some(slide))
}

/// The slide of the workbook's sheet `index` (the last for one past the end): only a worksheet that is shown has one.
fn sheet_slide(editor: &SlideEditor, index: usize) -> Result<SlideId, String> {
    let sheets = &editor.presentation().workbook.as_ref().ok_or("no workbook is open")?.sheets;
    let index = index.min(sheets.len().checked_sub(1).ok_or("the workbook has no sheets")?);
    let entry = &sheets[index];
    entry.slide.filter(|_| !entry.other).ok_or_else(|| format!("sheet {index} is hidden or not a worksheet, and is not shown"))
}

pub(crate) fn place(editor: &SlideEditor) -> Option<Place> {
    if editor.page_layout_shown() {
        return None;
    }
    let sheet = sheet_index(editor, editor.current_slide)?;
    let g = geometry(editor)?;
    let (_, cells) = editor.current_sheet()?;
    let offset = editor.workspace_scroll.offset();
    // Half a pixel's slack: a view put at a row's top edge reads back as that row.
    let at = |scrolled: f32, head: f32, origin: f32| (scrolled + head - origin + 0.5) / g.scale;
    let row = index_at(&cells.row_offsets(), cells.default_row, at(-f32::from(offset.y), g.head.y, g.origin.y));
    let column = index_at(&cells.column_offsets(), cells.default_column, at(-f32::from(offset.x), g.head.x, g.origin.x));
    let top = Top::Cell { sheet, row: row.min(MAX_SHEET_ROWS - 1) + 1, column: column.min(MAX_SHEET_COLUMNS - 1) + 1 };
    let active = editor.current_cell().map(|(column, row)| At::Cell { sheet, row: row + 1, column: column + 1 });
    Some(Place { top, at: active })
}

/// A 1-based row and column as the sheet's 0-based ones, kept on the sheet.
fn cell(row: u32, column: u32) -> (u32, u32) {
    (row.clamp(1, MAX_SHEET_ROWS) - 1, column.clamp(1, MAX_SHEET_COLUMNS) - 1)
}

pub(crate) fn show(editor: &mut SlideEditor, top: Option<Top>, at: Option<At>, window: &mut Window, cx: &mut Context<SlideEditor>) -> Result<(), String> {
    let top = match top {
        None => None,
        Some(Top::Cell { sheet, row, column }) => Some((sheet_slide(editor, sheet)?, cell(row, column))),
        Some(_) => return Err(wrong_kind("workbook")),
    };
    let at = match at {
        None => None,
        Some(At::Cell { sheet, row, column }) => Some((sheet_slide(editor, sheet)?, cell(row, column))),
        Some(_) => return Err(wrong_kind("workbook")),
    };
    let Some(shown) = top.or(at).map(|(slide, _)| slide) else { return Ok(()) };
    if editor.cell_edit().is_some() {
        editor.commit_cell_edit(cx);
    }
    if editor.current_slide != shown {
        editor.select_slide(shown, window, cx);
    }
    if let Some((slide, (row, column))) = at {
        if slide == shown {
            editor.set_cell(column, row);
        } else {
            // The active cell of a sheet not shown is the one it shows when its tab is clicked.
            let state = editor.wb_mut();
            state.cells.insert(slide, (column, row));
            state.cell_ends.remove(&slide);
        }
    }
    if let Some((_, (row, column))) = top {
        let (Some(g), Some((_, cells))) = (geometry(editor), editor.current_sheet()) else { return Ok(()) };
        let start = |offsets: &[Emu], default: Emu, index: u32, origin: f32, head: f32| origin + edge(offsets, default, index) as f32 * g.scale - head;
        let x = start(&cells.column_offsets(), cells.default_column, column, g.origin.x, g.head.x);
        let y = start(&cells.row_offsets(), cells.default_row, row, g.origin.y, g.head.y);
        let clamp = |v: f32, view: f32, content: f32| px(-v.clamp(0.0, (content - view).max(0.0)));
        editor.workspace_scroll.set_offset(point(clamp(x, g.view.width, g.content.width), clamp(y, g.view.height, g.content.height)));
    } else if at.is_some_and(|(slide, _)| slide == shown) {
        editor.scroll_cell_into_view();
    }
    Ok(())
}
