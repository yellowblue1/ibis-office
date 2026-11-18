// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! An agent's edits on a file, with no window:
//!
//!   ibis-agent inspect <file.pptx>
//!   ibis-agent apply <file.pptx> <ops.json | -> [-o <out.pptx>]
//!   ibis-agent new <out.pptx>
//!
//! `inspect` prints the summary the browser version's `inspect` gives. `apply` applies the operations (a JSON array; `-`
//! reads it from standard input) as one step and writes the file (over itself without `-o`), then prints
//! `{"replaced": n, "document": <inspect of the file written>}`; when they are refused it writes nothing, prints `{"error": …}` and exits
//! with 1. `new` writes an empty presentation (one title slide).

use std::io::Read;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(out) => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("{}", serde_json::json!({ "error": error }));
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<serde_json::Value, serde_json::Value> {
    let text = |e: String| serde_json::Value::String(e);
    match args {
        [cmd, file] if cmd == "inspect" => {
            let session = open(file).map_err(text)?;
            Ok(serde_json::to_value(session.inspect()).expect("serialisable"))
        }
        [cmd, out] if cmd == "new" => {
            let bytes = ibis_pptx::blank_file(ibis_pptx::Kind::Presentation).map_err(|e| text(e.to_string()))?;
            std::fs::write(out, bytes).map_err(|e| text(format!("{out}: {e}")))?;
            Ok(serde_json::json!({ "written": out }))
        }
        [cmd, file, ops, rest @ ..] if cmd == "apply" => {
            let out = match rest {
                [] => file.clone(),
                [flag, out] if flag == "-o" => out.clone(),
                _ => return Err(text(usage())),
            };
            let json = if ops == "-" {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s).map_err(|e| text(e.to_string()))?;
                s
            } else {
                std::fs::read_to_string(ops).map_err(|e| text(format!("{ops}: {e}")))?
            };
            let ops = ibis_agent::parse_ops(&json).map_err(text)?;
            let mut session = open(file).map_err(text)?;
            let created = session.apply(&ops).map_err(|e| serde_json::to_value(e).expect("serialisable"))?;
            let bytes = session.save().map_err(text)?;
            std::fs::write(&out, bytes).map_err(|e| text(format!("{out}: {e}")))?;
            // Ids are numbered as a file is read: those of what was made are told by the file as it is read again, not by
            // `created`.
            let replaced: usize = created.iter().filter_map(|c| c.replaced).sum();
            let reread = open(&out).map_err(text)?;
            Ok(serde_json::json!({ "replaced": replaced, "document": reread.inspect() }))
        }
        _ => Err(text(usage())),
    }
}

fn open(path: &str) -> Result<ibis_agent::Session, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    ibis_agent::Session::open(bytes, name)
}

fn usage() -> String {
    "usage: ibis-agent inspect <file.pptx> | apply <file.pptx> <ops.json|-> [-o <out.pptx>] | new <out.pptx>".into()
}
