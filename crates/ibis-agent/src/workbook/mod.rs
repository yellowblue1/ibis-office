// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! A workbook (`.xlsx`): its sheets in order, each with its id, and the cells with something in them, in A1 notation.

mod inspect;
mod ops;

pub use inspect::{CELLS_PER_SHEET, CellInfo, Inspection, SheetInfo, inspect};
pub use ops::{Created, Op, plan};

#[cfg(test)]
mod tests;
