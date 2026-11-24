// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! An agent's edits on a file, with no window:
//!
//!   ibis-agent inspect <file> [--name <name>]
//!   ibis-agent apply <file> <ops.json | -> [-o <out>]
//!   ibis-agent new <out.pptx | out.docx | out.xlsx>
//!
//! A file is a presentation, a document or a workbook (`.pptx`, `.docx`, `.xlsx`), told by its content. `inspect`
//! prints the summary the browser version's `inspect` gives, naming the file `<name>` when given (a server keeps a file
//! under a name of its own). `apply` applies the operations (a JSON array; `-` reads it from standard input) as one step
//! and writes the file (over itself without `-o`; the file there is replaced only by one that opens again), then prints `{"replaced": n, "document": <inspect of the file
//! written>}`; when they are refused it writes nothing, prints `{"error": …}` and exits with 1. `new` writes an empty file
//! of the kind its name's extension names.

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
        [cmd, file, rest @ ..] if cmd == "inspect" => {
            let mut session = open(file).map_err(text)?;
            match rest {
                [] => {}
                [flag, name] if flag == "--name" => session.name = name.clone(),
                _ => return Err(text(usage())),
            }
            Ok(serde_json::to_value(session.inspect()).expect("serialisable"))
        }
        [cmd, out] if cmd == "new" => {
            let bytes = ibis_agent::blank(out).map_err(text)?;
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
            let mut session = open(file).map_err(text)?;
            let ops = ibis_agent::parse_ops(session.kind(), &json).map_err(text)?;
            let applied = session.apply(&ops).map_err(|e| serde_json::to_value(e).expect("serialisable"))?;
            let bytes = session.save().map_err(text)?;
            // Written beside `out` and put in its place only once it reads again, so a file that would not open never
            // takes the place of the one there (often the file the operations were applied to).
            let written = format!("{out}.ibis-agent-tmp");
            std::fs::write(&written, &bytes).map_err(|e| text(format!("{written}: {e}")))?;
            // Ids are numbered as a file is read: those of what was made are told by the file as it is read again, not by
            // `created`.
            let reread = match std::fs::read(&written).map_err(|e| e.to_string()).and_then(|bytes| ibis_agent::Session::open(bytes, file_name(&out))) {
                Ok(reread) => reread,
                Err(e) => {
                    let _ = std::fs::remove_file(&written);
                    return Err(text(format!("the file written would not open again, so {out} is left as it was: {e}")));
                }
            };
            std::fs::rename(&written, &out).map_err(|e| text(format!("{out}: {e}")))?;
            Ok(serde_json::json!({ "replaced": applied.replaced, "document": reread.inspect() }))
        }
        _ => Err(text(usage())),
    }
}

fn open(path: &str) -> Result<ibis_agent::Session, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    ibis_agent::Session::open(bytes, file_name(path))
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn usage() -> String {
    "usage: ibis-agent inspect <file> [--name <name>] | apply <file> <ops.json|-> [-o <out>] | new <out.pptx|out.docx|out.xlsx>".into()
}
