// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! What the operations of more than one kind of file take alike: text as its paragraphs, a character and a paragraph
//! format, and colours.

use serde::Deserialize;

/// Text as its paragraphs, in order: the same shape [`crate::inspect`] gives it in. A paragraph holds no line break, so
/// where one paragraph ends is never a character the agent has to write.
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "Vec<String>")]
pub struct Text(String);

impl TryFrom<Vec<String>> for Text {
    type Error = String;

    fn try_from(paragraphs: Vec<String>) -> Result<Self, String> {
        for (i, p) in paragraphs.iter().enumerate() {
            check_paragraph(i, p)?;
        }
        Ok(Text(paragraphs.join("\n")))
    }
}

/// Refuse what paragraph `i`'s text cannot be: a line break in it, or a character a file cannot hold.
pub(crate) fn check_paragraph(i: usize, text: &str) -> Result<(), String> {
    if has_line_break(text) {
        return Err(format!("paragraph {i} has a line break: give each paragraph as its own string"));
    }
    holdable(text).map_err(|e| format!("paragraph {i}: {e}"))
}

/// Refuse a control character other than a tab (and the line breaks, refused on their own): a file holds none, so it
/// would be left out of what is saved, and the file would not have what the agent reads.
pub(crate) fn holdable(text: &str) -> Result<(), String> {
    match text.chars().find(|&c| c < ' ' && !matches!(c, '\t' | '\n' | '\r' | '\u{b}') || matches!(c, '\u{fffe}' | '\u{ffff}')) {
        Some(c) => Err(format!("the text has the character U+{:04X}, which a file cannot hold", c as u32)),
        None => Ok(()),
    }
}

impl Text {
    /// The paragraphs joined by `\n`.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

pub(crate) fn has_line_break(text: &str) -> bool {
    text.contains(['\n', '\r', '\u{b}'])
}

/// A character format: what is given is set, the rest is left as it is.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextFormat {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    /// In points.
    pub size: Option<f64>,
    pub color: Option<String>,
    pub font: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum List {
    None,
    Bullet,
    Numbered,
}

/// A paragraph format: what is given is set, the rest is left as it is.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParagraphFormat {
    pub align: Option<Align>,
    pub list: Option<List>,
    /// The outline level, 0 to 8.
    pub level: Option<u8>,
    /// In points.
    pub space_before: Option<f64>,
    pub space_after: Option<f64>,
}

impl ParagraphFormat {
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.level.is_some_and(|l| l > 8) {
            return Err("level is 0 to 8".into());
        }
        for v in [self.space_before, self.space_after].into_iter().flatten() {
            if !(v >= 0.0 && v.is_finite()) {
                return Err(format!("{v} is not a space (points, 0 or more)"));
            }
        }
        Ok(())
    }
}

/// A size in points, as hundredths of a point.
pub(crate) fn font_size(points: f64) -> Result<u32, String> {
    if points > 0.0 && points <= 4000.0 { Ok((points * 100.0).round() as u32) } else { Err(format!("size {points} is not a font size")) }
}

pub(crate) fn rgb(color: &str) -> Result<ibis_doc_model::drawing::Rgb, String> {
    let hex = color.strip_prefix('#').unwrap_or(color);
    if hex.len() != 6 {
        return Err(format!("{color:?} is not a colour (#RRGGBB)"));
    }
    u32::from_str_radix(hex, 16).map(ibis_doc_model::drawing::Rgb).map_err(|_| format!("{color:?} is not a colour (#RRGGBB)"))
}

/// The character positions of each `find` in `text`, not overlapping.
pub(crate) fn occurrences(text: &str, find: &str) -> Vec<usize> {
    text.match_indices(find).map(|(byte, _)| text[..byte].chars().count()).collect()
}
