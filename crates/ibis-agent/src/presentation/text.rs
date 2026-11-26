// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! A presentation's text as the agent reads and writes it. A paragraph is its string; one holding a field — text the
//! presentation works out, a slide number or a date (`a:fld`) — is its pieces in order instead, each a string or the
//! field as `{"field": "slide_number"}` or `{"field": "date"}`: `["Page ", {"field": "slide_number"}]`. Given back,
//! a field stays the field it was, so the number goes on following the slide; a field is kept, never made.

use std::collections::HashMap;

use ibis_doc_model::text::{Paragraph, Run, RunStyle, TextBody, TextField};
use serde::{Deserialize, Serialize};

use crate::shared::{has_line_break, holdable};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldName {
    SlideNumber,
    Date,
}

impl FieldName {
    fn of(field: TextField) -> Self {
        match field {
            TextField::SlideNumber => FieldName::SlideNumber,
            TextField::DateTime { .. } => FieldName::Date,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldPiece {
    pub field: FieldName,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Piece {
    Text(String),
    Field(FieldPiece),
}

/// A paragraph as the summary gives it and the operations take it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParagraphText {
    Plain(String),
    Pieces(Vec<Piece>),
}

/// A paragraph's runs read as pieces: the runs of one field in a row (a date split where its script changes) are one.
pub fn read(paragraph: &Paragraph) -> ParagraphText {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut last_field = None;
    for run in &paragraph.runs {
        match run.style.field {
            Some(field) if last_field == Some(field) => {}
            Some(field) => pieces.push(Piece::Field(FieldPiece { field: FieldName::of(field) })),
            None => match pieces.last_mut() {
                Some(Piece::Text(text)) => text.push_str(&run.text),
                _ => pieces.push(Piece::Text(run.text.clone())),
            },
        }
        last_field = run.style.field;
    }
    pieces.retain(|p| !matches!(p, Piece::Text(t) if t.is_empty()));
    match pieces.as_slice() {
        [] => ParagraphText::Plain(String::new()),
        [Piece::Text(text)] => ParagraphText::Plain(text.clone()),
        _ => ParagraphText::Pieces(pieces),
    }
}

fn pieces_of(paragraph: &Paragraph) -> Vec<Piece> {
    match read(paragraph) {
        ParagraphText::Plain(text) if text.is_empty() => Vec::new(),
        ParagraphText::Plain(text) => vec![Piece::Text(text)],
        ParagraphText::Pieces(pieces) => pieces,
    }
}

pub fn read_body(body: &TextBody) -> Vec<ParagraphText> {
    body.paragraphs.iter().map(read).collect()
}

/// Text as its paragraphs, each its pieces (a plain paragraph is one piece, an empty one none).
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "Vec<ParagraphText>")]
pub struct SlideText(Vec<Vec<Piece>>);

impl TryFrom<Vec<ParagraphText>> for SlideText {
    type Error = String;

    fn try_from(paragraphs: Vec<ParagraphText>) -> Result<Self, String> {
        let mut out = Vec::with_capacity(paragraphs.len());
        for (i, paragraph) in paragraphs.into_iter().enumerate() {
            let pieces = match paragraph {
                ParagraphText::Plain(text) => vec![Piece::Text(text)],
                ParagraphText::Pieces(pieces) => pieces,
            };
            let mut merged: Vec<Piece> = Vec::new();
            for piece in pieces {
                if let Piece::Text(text) = &piece {
                    if has_line_break(text) {
                        return Err(format!("paragraph {i} has a line break: give each paragraph as its own string"));
                    }
                    holdable(text).map_err(|e| format!("paragraph {i}: {e}"))?;
                    if text.is_empty() {
                        continue;
                    }
                    if let Some(Piece::Text(before)) = merged.last_mut() {
                        before.push_str(text);
                        continue;
                    }
                }
                merged.push(piece);
            }
            out.push(merged);
        }
        Ok(SlideText(out))
    }
}

impl SlideText {
    /// The paragraphs joined by `\n`, for text that holds no field (speaker notes, a shape being made).
    pub fn plain(&self, what: &str) -> Result<String, String> {
        let mut lines = Vec::with_capacity(self.0.len());
        for pieces in &self.0 {
            let mut line = String::new();
            for piece in pieces {
                match piece {
                    Piece::Text(text) => line.push_str(text),
                    Piece::Field(_) => return Err(format!("{what} has no field to keep: a field (a slide number, a date) stays in the text it was in, and is not made")),
                }
            }
            lines.push(line);
        }
        Ok(lines.join("\n"))
    }

    /// `body` with its text replaced: a paragraph given as it reads stays as it is; another, paragraph `i`, keeps the
    /// format of the old paragraph `i` (the last one's, past its end), its characters that of the old paragraph's
    /// first run that is no field, and the `n`th field of a kind is the old text's `n`th of that kind (its last, past
    /// the end), as it was.
    pub fn replace(&self, body: &TextBody) -> Result<TextBody, String> {
        let fields = fields(body);
        let mut taken: HashMap<FieldName, usize> = HashMap::new();
        let old = &body.paragraphs;
        let mut paragraphs = Vec::with_capacity(self.0.len());
        for (i, pieces) in self.0.iter().enumerate() {
            if let Some(same) = old.get(i).filter(|p| pieces_of(p) == *pieces) {
                for piece in pieces {
                    if let Piece::Field(FieldPiece { field }) = piece {
                        *taken.entry(*field).or_default() += 1;
                    }
                }
                paragraphs.push(same.clone());
                continue;
            }
            let template = old.get(i).or(old.last()).cloned().unwrap_or_default();
            let style = template
                .runs
                .iter()
                .find(|r| r.style.field.is_none())
                .or(template.runs.first())
                .map_or_else(|| template.end_style.clone(), |r| r.style.clone());
            let style = RunStyle { field: None, ..style };
            let mut runs = Vec::new();
            for piece in pieces {
                match piece {
                    Piece::Text(text) => runs.push(Run { text: text.clone(), style: style.clone() }),
                    Piece::Field(FieldPiece { field }) => {
                        let same: Vec<&Vec<Run>> = fields.iter().filter(|(name, _)| name == field).map(|(_, runs)| runs).collect();
                        let count = taken.entry(*field).or_default();
                        let kept = same.get(*count).or(same.last()).ok_or_else(|| {
                            format!("paragraph {i}: the text has no {} field to keep: a field stays in the text it was in, and is not made", name(*field))
                        })?;
                        *count += 1;
                        runs.extend(kept.iter().cloned());
                    }
                }
            }
            paragraphs.push(Paragraph { runs, end_style: style, ..template });
        }
        Ok(TextBody { paragraphs, ..body.clone() })
    }
}

fn name(field: FieldName) -> &'static str {
    match field {
        FieldName::SlideNumber => "slide_number",
        FieldName::Date => "date",
    }
}

/// The body's fields in order, each with its runs.
fn fields(body: &TextBody) -> Vec<(FieldName, Vec<Run>)> {
    let mut out: Vec<(FieldName, Vec<Run>)> = Vec::new();
    for paragraph in &body.paragraphs {
        let mut last = None;
        for run in &paragraph.runs {
            if let Some(field) = run.style.field {
                if last == Some(field) {
                    out.last_mut().expect("a field before").1.push(run.clone());
                } else {
                    out.push((FieldName::of(field), vec![run.clone()]));
                }
            }
            last = run.style.field;
        }
    }
    out
}

/// Where the body's fields are in its text (paragraphs joined by `\n`), as character ranges.
pub fn field_spans(body: &TextBody) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut at = 0;
    for paragraph in &body.paragraphs {
        for run in &paragraph.runs {
            let len = run.text.chars().count();
            if run.style.field.is_some() && len > 0 {
                spans.push((at, at + len));
            }
            at += len;
        }
        at += 1;
    }
    spans
}
