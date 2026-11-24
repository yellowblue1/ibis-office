// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Edits for an AI agent, on a presentation, a document or a workbook.
//!
//! The agent reads the file as [`inspect`] summarises it (with ids for what it can change), and changes it with
//! operations in JSON, a small set for each kind of file ([`presentation`], [`document`], [`workbook`]). [`plan`] turns
//! a list of them into the editor's [`Command`]s, tried on a copy of the file's model and saved there: an operation that
//! fails, or a result the file cannot be saved with, is refused and nothing changes. The editor executes the commands as
//! one step, so one edit of the agent's is one undo.
//!
//! The browser version runs this on the file open in the page (`ibis-office-shell`'s `agent`); [`Session`] runs it on a
//! file, with no window, for the command-line tool (`ibis-agent`).

pub mod document;
pub mod presentation;
mod shared;
pub mod workbook;

use std::sync::Arc;

use ibis_doc_model::command::Command;
use ibis_doc_model::model::SlideId;
use ibis_doc_model::{Document, Presentation};
use ibis_pptx::{Kind, Source};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The file as the agent reads it, its kind first.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Summary {
    Presentation(presentation::Inspection),
    Document(document::Inspection),
    Workbook(workbook::Inspection),
}

/// A list of operations, of the kind of file they edit.
#[derive(Clone, Debug)]
pub enum Ops {
    Presentation(Vec<presentation::Op>),
    Document(Vec<document::Op>),
    Workbook(Vec<workbook::Op>),
}

/// Why a list of operations was refused (nothing of it was applied). `index` is the operation's, when one failed.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OpError {
    pub index: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.index {
            Some(i) => write!(f, "operation {i}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// The commands for a list of operations, for the editor to execute as one step, and what each operation made.
#[derive(Debug)]
pub struct Planned {
    pub commands: Vec<Command>,
    /// What each operation made (its ids), in the list's order, for the agent to name it next.
    pub created: Vec<serde_json::Value>,
    /// How many places the list's `replace_text`s replaced.
    pub replaced: usize,
    /// The slide (a workbook's sheet) the last operation worked on, when it is still there: what the editor shows after.
    pub shown: Option<SlideId>,
}

/// What a list of operations made, once applied.
#[derive(Debug)]
pub struct Applied {
    pub created: Vec<serde_json::Value>,
    pub replaced: usize,
}

/// `kind`'s name as the summary gives it.
pub fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Presentation => "presentation",
        Kind::Document => "document",
        Kind::Workbook => "workbook",
    }
}

/// The file as the agent reads it.
pub fn inspect(kind: Kind, model: &Presentation, name: &str, dirty: bool) -> Summary {
    match kind {
        Kind::Presentation => Summary::Presentation(presentation::inspect(model, name, dirty)),
        Kind::Document => Summary::Document(document::inspect(model, name, dirty)),
        Kind::Workbook => Summary::Workbook(workbook::inspect(model, name, dirty)),
    }
}

/// Parse a JSON array of operations (or one operation) on a file of `kind`.
pub fn parse_ops(kind: Kind, json: &str) -> Result<Ops, String> {
    Ok(match kind {
        Kind::Presentation => Ops::Presentation(parse_list(json)?),
        Kind::Document => Ops::Document(parse_list(json)?),
        Kind::Workbook => Ops::Workbook(parse_list(json)?),
    })
}

fn parse_list<T: DeserializeOwned>(json: &str) -> Result<Vec<T>, String> {
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

/// Turn `ops` into commands on `current`, opened from `source`: all of them, or none when one fails or the result cannot
/// be saved. `new_id` gives a fresh id (the editor's document's: `Document::ids`), for what the operations make.
pub fn plan(ops: &Ops, current: &Presentation, source: &Source, new_id: &mut dyn FnMut() -> u64) -> Result<Planned, OpError> {
    let kind = source.kind();
    let mismatch = |ops: Kind| OpError { index: None, message: format!("these are operations on a {}, and the file is a {}", kind_name(ops), kind_name(kind)) };
    match (ops, kind) {
        (Ops::Presentation(ops), Kind::Presentation) => presentation::plan(ops, current, source, new_id),
        (Ops::Document(ops), Kind::Document) => document::plan(ops, current, source, new_id),
        (Ops::Workbook(ops), Kind::Workbook) => workbook::plan(ops, current, source, new_id),
        (Ops::Presentation(_), _) => Err(mismatch(Kind::Presentation)),
        (Ops::Document(_), _) => Err(mismatch(Kind::Document)),
        (Ops::Workbook(_), _) => Err(mismatch(Kind::Workbook)),
    }
}

/// The model saved and the bytes read again (then thrown away), to make sure the file can be written with it and opened
/// after: a writer that wrote something unreadable refuses the edit rather than leave a file that no longer opens.
pub(crate) fn check_saves(source: &Source, model: &Presentation) -> Result<(), OpError> {
    let refused = |e: String| OpError { index: None, message: format!("the file cannot be saved with this change: {e}") };
    let bytes = source.save(model, ibis_pptx::SaveOptions::default()).map_err(|e| refused(e.to_string()))?;
    ibis_pptx::open(bytes).map(drop).map_err(|e| refused(format!("the file written would not open again ({e})")))
}

/// An empty file of the kind `name`'s extension names (`.pptx`: one title slide; `.docx`: one empty paragraph;
/// `.xlsx`: one empty sheet).
pub fn blank(name: &str) -> Result<Vec<u8>, String> {
    let kind = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(Kind::from_extension)
        .ok_or_else(|| format!("{name}: name a .pptx, .docx or .xlsx file"))?;
    ibis_pptx::blank_file(kind).map_err(|e| e.to_string())
}

/// A file open for an agent's edits, with no editor: what the command-line tool works on.
pub struct Session {
    pub document: Document,
    pub source: Arc<Source>,
    pub name: String,
}

impl Session {
    /// Open a file's bytes (`name` is the file's name, for [`inspect`]).
    pub fn open(bytes: Vec<u8>, name: impl Into<String>) -> Result<Self, String> {
        let opened = ibis_pptx::open(bytes).map_err(|e| e.to_string())?;
        Ok(Self { document: Document::new(opened.presentation), source: Arc::new(opened.source), name: name.into() })
    }

    pub fn kind(&self) -> Kind {
        self.source.kind()
    }

    pub fn model(&self) -> &Presentation {
        self.document.presentation()
    }

    pub fn inspect(&self) -> Summary {
        inspect(self.kind(), self.model(), &self.name, self.document.is_dirty())
    }

    /// Apply `ops` as one step: all of them or, when one fails or the result cannot be saved, none.
    pub fn apply(&mut self, ops: &Ops) -> Result<Applied, OpError> {
        let current = self.model().clone();
        let ids = self.document.ids();
        let planned = plan(ops, &current, &self.source, &mut || ids.new_shape_id().0)?;
        self.document.execute_unrepeated(planned.commands).map_err(|e| OpError { index: None, message: e.to_string() })?;
        Ok(Applied { created: planned.created, replaced: planned.replaced })
    }

    pub fn save(&self) -> Result<Vec<u8>, String> {
        self.source.save(self.model(), ibis_pptx::SaveOptions::default()).map_err(|e| e.to_string())
    }
}

impl Summary {
    pub fn into_presentation(self) -> presentation::Inspection {
        match self {
            Summary::Presentation(inspection) => inspection,
            other => panic!("not a presentation: {other:?}"),
        }
    }

    pub fn into_document(self) -> document::Inspection {
        match self {
            Summary::Document(inspection) => inspection,
            other => panic!("not a document: {other:?}"),
        }
    }

    pub fn into_workbook(self) -> workbook::Inspection {
        match self {
            Summary::Workbook(inspection) => inspection,
            other => panic!("not a workbook: {other:?}"),
        }
    }
}
