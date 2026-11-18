// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! What the command-line tool does, for a JavaScript runtime with no window (a server editing a file nobody has open):
//! `scripts/build-agent-wasm.sh` builds it for wasm-bindgen's `nodejs` target.

use wasm_bindgen::prelude::*;

use crate::Session;

/// An empty presentation (one title slide).
#[wasm_bindgen]
pub fn new_presentation() -> Result<Vec<u8>, JsError> {
    ibis_pptx::blank_file(ibis_pptx::Kind::Presentation).map_err(|e| JsError::new(&e.to_string()))
}

/// The file `bytes`, named `name`, as the agent reads it (`crate::Inspection`), in JSON.
#[wasm_bindgen]
pub fn inspect(bytes: Vec<u8>, name: &str) -> Result<String, JsError> {
    let session = Session::open(bytes, name).map_err(|e| JsError::new(&e))?;
    Ok(serde_json::to_string(&session.inspect()).expect("serialisable"))
}

/// `ops` (JSON) applied to the file `bytes` as one step. Answers the file written, or throws why the operations were
/// refused (`crate::OpError` in JSON: nothing was written).
#[wasm_bindgen]
pub fn apply(bytes: Vec<u8>, name: &str, ops: &str) -> Result<Applied, JsError> {
    let ops = crate::parse_ops(ops).map_err(|e| JsError::new(&serde_json::json!({ "index": null, "message": e }).to_string()))?;
    let mut session = Session::open(bytes, name).map_err(|e| JsError::new(&serde_json::json!({ "index": null, "message": e }).to_string()))?;
    let created = session.apply(&ops).map_err(|e| JsError::new(&serde_json::to_string(&e).expect("serialisable")))?;
    let file = session.save().map_err(|e| JsError::new(&serde_json::json!({ "index": null, "message": e }).to_string()))?;
    let replaced: usize = created.iter().filter_map(|c| c.replaced).sum();
    Ok(Applied { file, replaced })
}

#[wasm_bindgen(getter_with_clone)]
pub struct Applied {
    /// The file with the operations applied.
    pub file: Vec<u8>,
    /// How many places `replace_text` replaced.
    pub replaced: usize,
}
