// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! A presentation (`.pptx`): its slides and their shapes, each with its id.

mod inspect;
mod ops;
mod text;

pub use inspect::{Inspection, inspect};
pub use ops::{Created, Op, plan};
pub use text::{FieldName, FieldPiece, ParagraphText, Piece};

#[cfg(test)]
mod tests;
