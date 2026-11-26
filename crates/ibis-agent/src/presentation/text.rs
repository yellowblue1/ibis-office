// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! A presentation's text as the agent reads and writes it. A paragraph is its string; one holding a field — text the
//! presentation works out, a slide number or a date (`a:fld`) — is its pieces in order instead, each a string or the
//! field as `{"field": "slide_number"}` or `{"field": "date"}`: the text `[["Page ", {"field": "slide_number"}]]`.
//! Given back, a field stays the field it was, so the number goes on following the slide; a field is kept, never made.

use ibis_doc_model::text::{Paragraph, Run, RunStyle, TextBody, TextField};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::shared::check_paragraph;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldName {
    SlideNumber,
    Date,
}

impl FieldName {
    const NAMES: [(FieldName, &'static str); 2] = [(FieldName::SlideNumber, "slide_number"), (FieldName::Date, "date")];

    fn of(field: TextField) -> Self {
        match field {
            TextField::SlideNumber => FieldName::SlideNumber,
            TextField::DateTime { .. } => FieldName::Date,
        }
    }

    fn name(self) -> &'static str {
        Self::NAMES.iter().find(|(f, _)| *f == self).expect("every field is named").1
    }

    fn named(name: &str) -> Option<Self> {
        Self::NAMES.iter().find(|(_, n)| *n == name).map(|(f, _)| *f)
    }
}

impl Serialize for FieldName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FieldPiece {
    pub field: FieldName,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Piece {
    Text(String),
    Field(FieldPiece),
}

/// A paragraph as the summary gives it and the operations take it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ParagraphText {
    Plain(String),
    Pieces(Vec<Piece>),
}

/// What a paragraph's runs are, in order: words, or a field with its runs (one field in a row is one, a date split
/// where its script changes among them). An empty run of words is left out.
enum Group {
    Words(String),
    Field(FieldName, Vec<Run>),
}

fn groups(paragraph: &Paragraph) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    let mut last = None;
    for run in &paragraph.runs {
        match (run.style.field, out.last_mut()) {
            (Some(field), Some(Group::Field(_, runs))) if last == Some(field) => runs.push(run.clone()),
            (Some(field), _) => out.push(Group::Field(FieldName::of(field), vec![run.clone()])),
            (None, _) if run.text.is_empty() => {}
            (None, Some(Group::Words(words))) => words.push_str(&run.text),
            (None, _) => out.push(Group::Words(run.text.clone())),
        }
        last = run.style.field;
    }
    out
}

fn pieces_of(paragraph: &Paragraph) -> Vec<Piece> {
    groups(paragraph)
        .into_iter()
        .map(|g| match g {
            Group::Words(words) => Piece::Text(words),
            Group::Field(field, _) => Piece::Field(FieldPiece { field }),
        })
        .collect()
}

pub fn read_paragraphs(body: &TextBody) -> Vec<ParagraphText> {
    body.paragraphs
        .iter()
        .map(|p| {
            let mut pieces = pieces_of(p);
            match pieces.as_slice() {
                [] => ParagraphText::Plain(String::new()),
                [Piece::Text(_)] => match pieces.remove(0) {
                    Piece::Text(text) => ParagraphText::Plain(text),
                    Piece::Field(_) => unreachable!(),
                },
                _ => ParagraphText::Pieces(pieces),
            }
        })
        .collect()
}

/// Text as its paragraphs, each its pieces (a plain paragraph is one piece, an empty one none); no paragraph at all is
/// one empty paragraph, as a text always has one.
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "Vec<Value>")]
pub struct SlideText(Vec<Vec<Piece>>);

impl TryFrom<Vec<Value>> for SlideText {
    type Error = String;

    fn try_from(paragraphs: Vec<Value>) -> Result<Self, String> {
        let mut out = Vec::with_capacity(paragraphs.len().max(1));
        for (i, paragraph) in paragraphs.into_iter().enumerate() {
            let given = match paragraph {
                Value::String(text) => vec![Value::String(text)],
                Value::Array(pieces) => pieces,
                _ => return Err(format!("paragraph {i} is neither a string nor a list of pieces")),
            };
            let mut pieces: Vec<Piece> = Vec::new();
            for given in given {
                match given {
                    Value::String(text) => {
                        check_paragraph(i, &text)?;
                        match pieces.last_mut() {
                            _ if text.is_empty() => {}
                            Some(Piece::Text(before)) => before.push_str(&text),
                            _ => pieces.push(Piece::Text(text)),
                        }
                    }
                    Value::Object(object) => {
                        let field = match (object.len(), object.get("field")) {
                            (1, Some(Value::String(name))) => FieldName::named(name),
                            _ => None,
                        };
                        let field = field.ok_or_else(|| {
                            let names: Vec<&str> = FieldName::NAMES.iter().map(|(_, n)| *n).collect();
                            format!("paragraph {i}: a field is {{\"field\": one of {names:?}}}, not {}", Value::Object(object))
                        })?;
                        pieces.push(Piece::Field(FieldPiece { field }));
                    }
                    other => return Err(format!("paragraph {i}: a piece is a string or a field, not {other}")),
                }
            }
            out.push(pieces);
        }
        if out.is_empty() {
            out.push(Vec::new());
        }
        Ok(SlideText(out))
    }
}

/// A field of the old text: its paragraph, what it is and its runs, and whether the new text has it yet.
struct Kept {
    paragraph: usize,
    field: FieldName,
    runs: Vec<Run>,
    used: bool,
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

    /// `body` with its text replaced. A paragraph given as it reads stays as it is. Another, paragraph `i`, keeps the
    /// format of the old paragraph `i` (the last one's, past its end), its words that of the old paragraph's first
    /// words, and each field one of the old text's of its kind not given yet, as it was: old paragraph `i`'s first,
    /// then the text's first. A field more than the old text had is refused.
    pub fn replace(&self, body: &TextBody) -> Result<TextBody, String> {
        let old = &body.paragraphs;
        let mut kept: Vec<Kept> = old
            .iter()
            .enumerate()
            .flat_map(|(paragraph, p)| {
                groups(p).into_iter().filter_map(move |g| match g {
                    Group::Field(field, runs) => Some(Kept { paragraph, field, runs, used: false }),
                    Group::Words(_) => None,
                })
            })
            .collect();
        let unchanged: Vec<bool> = self.0.iter().enumerate().map(|(i, pieces)| old.get(i).is_some_and(|p| pieces_of(p) == *pieces)).collect();
        for k in kept.iter_mut() {
            k.used = unchanged.get(k.paragraph).copied().unwrap_or(false);
        }
        let mut paragraphs = Vec::with_capacity(self.0.len());
        for (i, pieces) in self.0.iter().enumerate() {
            if unchanged[i] {
                paragraphs.push(old[i].clone());
                continue;
            }
            let template = old.get(i).or(old.last()).cloned().unwrap_or_default();
            let style = template
                .runs
                .iter()
                .find(|r| r.style.field.is_none() && !r.text.is_empty())
                .or(template.runs.first())
                .map_or_else(|| template.end_style.clone(), |r| r.style.clone());
            let style = RunStyle { field: None, ..style };
            let mut runs = Vec::new();
            for piece in pieces {
                match piece {
                    Piece::Text(text) => runs.push(Run { text: text.clone(), style: style.clone() }),
                    Piece::Field(FieldPiece { field }) => {
                        let free = |k: &Kept| !k.used && k.field == *field;
                        let at = kept.iter().position(|k| free(k) && k.paragraph == i).or_else(|| kept.iter().position(free)).ok_or_else(|| {
                            format!(
                                "paragraph {i}: the text has no more {} fields to keep (it had {}): a field stays in the text it was in, and is not made",
                                field.name(),
                                kept.iter().filter(|k| k.field == *field).count()
                            )
                        })?;
                        kept[at].used = true;
                        runs.extend(kept[at].runs.iter().cloned());
                    }
                }
            }
            paragraphs.push(Paragraph { runs, end_style: style, ..template });
        }
        Ok(TextBody { paragraphs, ..body.clone() })
    }
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
