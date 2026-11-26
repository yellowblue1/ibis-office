// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! A presentation's text as the agent reads and writes it: its paragraphs, each a string, a field in one — text the
//! presentation works out, a slide number or a date (`a:fld`) — as its mark, `{{slide_number}}` or `{{date}}`, where
//! the number or date stands: `["Page {{slide_number}}"]`. Given back, a mark stays the field it was, so the number
//! goes on following the slide; a field is kept, never made. A mark in a string, not a piece of its own, because a
//! paragraph stays one string every model writes alike, and a translation keeps such a mark as it is.

use ibis_doc_model::text::{Paragraph, Run, RunStyle, TextBody, TextField};
use serde::Deserialize;

use crate::shared::check_paragraph;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FieldName {
    SlideNumber,
    Date,
}

impl FieldName {
    const MARKS: [(FieldName, &'static str); 2] = [(FieldName::SlideNumber, "{{slide_number}}"), (FieldName::Date, "{{date}}")];

    fn of(field: TextField) -> Self {
        match field {
            TextField::SlideNumber => FieldName::SlideNumber,
            TextField::DateTime { .. } => FieldName::Date,
        }
    }

    fn mark(self) -> &'static str {
        Self::MARKS.iter().find(|(f, _)| *f == self).expect("every field has a mark").1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    Words(String),
    Field(FieldName),
}

/// A paragraph's runs, in order: words, or a field with its runs (one field in a row is one, a date split where its
/// script changes among them). An empty run of words is left out.
fn groups(paragraph: &Paragraph) -> Vec<(Piece, Vec<Run>)> {
    let mut out: Vec<(Piece, Vec<Run>)> = Vec::new();
    let mut last = None;
    for run in &paragraph.runs {
        match (run.style.field, out.last_mut()) {
            (Some(field), Some((Piece::Field(_), runs))) if last == Some(field) => runs.push(run.clone()),
            (Some(field), _) => out.push((Piece::Field(FieldName::of(field)), vec![run.clone()])),
            (None, _) if run.text.is_empty() => {}
            (None, Some((Piece::Words(words), _))) => words.push_str(&run.text),
            (None, _) => out.push((Piece::Words(run.text.clone()), Vec::new())),
        }
        last = run.style.field;
    }
    out
}

fn pieces_of(paragraph: &Paragraph) -> Vec<Piece> {
    groups(paragraph).into_iter().map(|(piece, _)| piece).collect()
}

/// The body's paragraphs as the summary gives them.
pub fn read_paragraphs(body: &TextBody) -> Vec<String> {
    body.paragraphs
        .iter()
        .map(|p| {
            pieces_of(p)
                .into_iter()
                .map(|piece| match piece {
                    Piece::Words(words) => words,
                    Piece::Field(field) => field.mark().to_string(),
                })
                .collect()
        })
        .collect()
}

/// A paragraph's string as pieces: its marks fields, the rest words.
fn parse(text: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while let Some((at, field)) = FieldName::MARKS.iter().filter_map(|(f, mark)| rest.find(mark).map(|at| (at, *f))).min_by_key(|(at, _)| *at) {
        if at > 0 {
            pieces.push(Piece::Words(rest[..at].to_string()));
        }
        pieces.push(Piece::Field(field));
        rest = &rest[at + field.mark().len()..];
    }
    if !rest.is_empty() {
        pieces.push(Piece::Words(rest.to_string()));
    }
    pieces
}

/// Text as its paragraphs, each its pieces; no paragraph at all is one empty paragraph, as a text always has one.
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "Vec<String>")]
pub struct SlideText(Vec<Vec<Piece>>);

impl TryFrom<Vec<String>> for SlideText {
    type Error = String;

    fn try_from(paragraphs: Vec<String>) -> Result<Self, String> {
        for (i, p) in paragraphs.iter().enumerate() {
            check_paragraph(i, p)?;
        }
        let mut out: Vec<Vec<Piece>> = paragraphs.iter().map(|p| parse(p)).collect();
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
    /// The paragraphs joined by `\n`, for text that keeps no field (speaker notes, a shape being made).
    pub fn plain(&self, what: &str) -> Result<String, String> {
        let mut lines = Vec::with_capacity(self.0.len());
        for pieces in &self.0 {
            let mut line = String::new();
            for piece in pieces {
                match piece {
                    Piece::Words(words) => line.push_str(words),
                    Piece::Field(field) => {
                        return Err(format!("{what} has no field to keep ({}): a field (a slide number, a date) stays in the text it was in, and is not made", field.mark()));
                    }
                }
            }
            lines.push(line);
        }
        Ok(lines.join("\n"))
    }

    /// `body` with its text replaced. A paragraph given as it reads stays as it is. Another, paragraph `i`, keeps the
    /// format of the old paragraph `i` (the last one's, past its end), its words that of the old paragraph's first
    /// words, and each mark one of the old text's fields of its kind not given yet, as it was: old paragraph `i`'s
    /// first, then the text's first. A mark more than the old text had fields is refused.
    pub fn replace(&self, body: &TextBody) -> Result<TextBody, String> {
        let old = &body.paragraphs;
        let mut kept: Vec<Kept> = old
            .iter()
            .enumerate()
            .flat_map(|(paragraph, p)| {
                groups(p).into_iter().filter_map(move |(piece, runs)| match piece {
                    Piece::Field(field) => Some(Kept { paragraph, field, runs, used: false }),
                    Piece::Words(_) => None,
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
                    Piece::Words(words) => runs.push(Run { text: words.clone(), style: style.clone() }),
                    Piece::Field(field) => {
                        let free = |k: &Kept| !k.used && k.field == *field;
                        let at = kept.iter().position(|k| free(k) && k.paragraph == i).or_else(|| kept.iter().position(free)).ok_or_else(|| {
                            format!(
                                "paragraph {i}: the text has no more {} fields to keep (it had {}): a field stays in the text it was in, and is not made",
                                field.mark(),
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
