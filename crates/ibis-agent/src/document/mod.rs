// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! A document (`.docx`). Not edited by the agent yet.

use ibis_doc_model::Presentation;
use ibis_pptx::Source;
use serde::{Deserialize, Serialize};

use crate::{OpError, Planned};

#[derive(Debug, Serialize)]
pub struct Inspection {
    pub name: String,
    pub dirty: bool,
}

pub fn inspect(_model: &Presentation, name: &str, dirty: bool) -> Inspection {
    Inspection { name: name.to_string(), dirty }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {}

pub fn plan(ops: &[Op], _current: &Presentation, _source: &Source, _new_id: &mut dyn FnMut() -> u64) -> Result<Planned, OpError> {
    match ops.first() {
        Some(op) => match *op {},
        None => Err(OpError { index: None, message: "no operations".into() }),
    }
}
