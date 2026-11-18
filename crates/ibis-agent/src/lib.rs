// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Edits for an AI agent, on a presentation.
//!
//! The agent reads the document as [`inspect`] summarises it (slides, shapes and their text, each with its id), and
//! changes it with [`Op`]s, a small set of operations in JSON. [`plan`] turns a list of them into the editor's
//! [`Command`](ibis_doc_model::Command)s, tried on a copy of the document and saved there: an operation that fails, or a
//! result the file cannot be saved with, is refused and nothing changes. The editor executes the commands as one step,
//! so one edit of the agent's is one undo.
//!
//! The browser version runs this on the document open in the page (`ibis-office-shell`'s `agent`); [`Session`] runs it
//! on a file, with no window, for the command-line tool (`ibis-agent`).

mod inspect;
mod ops;
#[cfg(target_family = "wasm")]
mod wasm;

use std::sync::Arc;

use ibis_doc_model::{Document, Presentation};
use ibis_pptx::Source;

pub use inspect::{Inspection, inspect};
pub use ops::{Created, Op, OpError, Planned, plan};

/// Parse a JSON array of operations (or one operation).
pub fn parse_ops(json: &str) -> Result<Vec<Op>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("not JSON: {e}"))?;
    let list = match value {
        serde_json::Value::Array(items) => items,
        one => vec![one],
    };
    list.into_iter()
        .enumerate()
        .map(|(i, item)| serde_json::from_value(item).map_err(|e| format!("operation {i}: {e}")))
        .collect()
}

/// A file open for an agent's edits, with no editor: what the command-line tool works on.
pub struct Session {
    pub document: Document,
    pub source: Arc<Source>,
    pub name: String,
}

impl Session {
    /// Open a presentation's bytes (`name` is the file's name, for [`inspect`]).
    pub fn open(bytes: Vec<u8>, name: impl Into<String>) -> Result<Self, String> {
        let opened = ibis_pptx::open(bytes).map_err(|e| e.to_string())?;
        if opened.source.kind() != ibis_pptx::Kind::Presentation {
            return Err("only presentations (.pptx) can be edited yet".into());
        }
        Ok(Self { document: Document::new(opened.presentation), source: Arc::new(opened.source), name: name.into() })
    }

    pub fn presentation(&self) -> &Presentation {
        self.document.presentation()
    }

    pub fn inspect(&self) -> Inspection {
        inspect(self.presentation(), &self.name, self.document.is_dirty())
    }

    /// Apply `ops` as one step: all of them or, when one fails or the result cannot be saved, none.
    pub fn apply(&mut self, ops: &[Op]) -> Result<Vec<Created>, OpError> {
        let current = self.presentation().clone();
        let ids = self.document.ids();
        let planned = plan(ops, &current, &self.source, &mut || ids.new_shape_id().0)?;
        self.document.execute(planned.commands).map_err(|e| OpError { index: None, message: e.to_string() })?;
        Ok(planned.created)
    }

    pub fn save(&self) -> Result<Vec<u8>, String> {
        self.source.save(self.presentation(), ibis_pptx::SaveOptions::default()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests;
