// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! A workbook's place for the embedding page's two editors side by side (`shell::agent::Place`): the sheet shown by its
//! index in the workbook's order (hidden sheets counted, as `inspect`'s `number`), the top-left cell of the part of it
//! that scrolls (the frozen rows and columns left out) and the active cell, 1-based.

use gpui_kit::*;
use ibis_doc_model::{Emu, SlideId};

use crate::ext::*;
use crate::shell::agent::{At, Place, Top, wrong_kind};
use crate::shell::editor::{SlideEditor, sheet_of};
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
/// (pixels: the headers, the frozen part), the sheet's edge there, the size of the view, and pixels per EMU.
struct Geometry {
    head: Point<f32>,
    origin: Point<f32>,
    view: Size<f32>,
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

/// The page layout view (ページ レイアウト) and split panes (表示 > 分割) scroll in ways of their own: no place is told or
/// shown on a sheet shown so.
fn own_scrolling(sheet: &ibis_doc_model::Sheet) -> bool {
    crate::page_layout::shows_pages(sheet) || sheet.split.is_some()
}

/// A row's height and a column's width where the sheet gives none, as the grid has them (`workspace_content`).
fn defaults(sheet: &ibis_doc_model::Sheet) -> (Emu, Emu) {
    let point = ibis_doc_model::EMU_PER_POINT;
    let row = if sheet.default_row > 0 { sheet.default_row } else { 20 * point };
    let column = if sheet.default_column > 0 { sheet.default_column } else { 54 * point };
    (row, column)
}

pub(crate) fn place(editor: &SlideEditor) -> Option<Place> {
    let (_, cells) = editor.current_sheet()?;
    if own_scrolling(cells) {
        return None;
    }
    let sheet = sheet_index(editor, editor.current_slide)?;
    let g = geometry(editor)?;
    let offset = editor.workspace_scroll.offset();
    // Half a pixel's slack: a view put at a row's top edge reads back as that row.
    let at = |scrolled: f32, head: f32, origin: f32| (scrolled + head - origin + 0.5) / g.scale;
    let (default_row, default_column) = defaults(cells);
    let row = index_at(&cells.row_offsets(), default_row, at(-f32::from(offset.y), g.head.y, g.origin.y));
    let column = index_at(&cells.column_offsets(), default_column, at(-f32::from(offset.x), g.head.x, g.origin.x));
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
    let shown_sheet = editor.presentation().slides.iter().find(|s| s.id == shown).and_then(sheet_of);
    if shown_sheet.is_some_and(|(_, sheet)| own_scrolling(sheet)) {
        return Err("the sheet is shown in the page layout view or in split panes, which show no place".into());
    }
    // What the person is typing is entered only to select a cell (`at`): a view that follows the other editor's scrolling
    // leaves it being typed.
    if editor.cell_edit().is_some() {
        if at.is_none() {
            if editor.current_slide != shown {
                return Err("a cell is being typed in on another sheet: only a cell to select (`at`) leaves it".into());
            }
        } else {
            editor.commit_cell_edit(cx);
        }
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
        let (default_row, default_column) = defaults(cells);
        let (columns, rows) = (cells.column_offsets(), cells.row_offsets());
        let at = |offsets: &[Emu], default: Emu, index: u32, origin: f32| origin + edge(offsets, default, index) as f32 * g.scale;
        // Against the sheet's own end: the grid drawn reaches only some way past the view and the selection.
        let clamp = |index: u32, offsets: &[Emu], default: Emu, last: u32, origin: f32, head: f32, view: f32| {
            let end = at(offsets, default, last, origin);
            px(-(at(offsets, default, index, origin) - head).clamp(0.0, (end - view).max(0.0)))
        };
        let x = clamp(column, &columns, default_column, MAX_SHEET_COLUMNS, g.origin.x, g.head.x, g.view.width);
        let y = clamp(row, &rows, default_row, MAX_SHEET_ROWS, g.origin.y, g.head.y, g.view.height);
        editor.workspace_scroll.set_offset(point(x, y));
    } else if at.is_some_and(|(slide, _)| slide == shown) {
        editor.scroll_cell_into_view();
    }
    Ok(())
}
