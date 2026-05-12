pub mod ci;
pub mod dev;
pub mod issue;
pub mod pr;
pub mod repo;
pub mod status;
pub mod time;

use std::collections::HashSet;

use nucleo_matcher::{pattern::Atom, Matcher, Utf32Str};
use ratatui::{style::Style, text::Span};

/// Split text into spans, highlighting characters at the given indices with `highlight_style`.
pub fn highlight_spans(
    text: &str,
    indices: &[u32],
    base_style: Style,
    highlight_style: Style,
) -> Vec<Span<'static>> {
    if indices.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    let match_set: HashSet<u32> = indices.iter().copied().collect();
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut current_is_match = false;

    for (char_idx, ch) in text.chars().enumerate() {
        let is_match = match_set.contains(&(char_idx as u32));
        if is_match != current_is_match && !current.is_empty() {
            let style = if current_is_match {
                highlight_style
            } else {
                base_style
            };
            spans.push(Span::styled(std::mem::take(&mut current), style));
        }
        current_is_match = is_match;
        current.push(ch);
    }
    if !current.is_empty() {
        let style = if current_is_match {
            highlight_style
        } else {
            base_style
        };
        spans.push(Span::styled(current, style));
    }

    spans
}

/// Compute match indices for `text` against all search atoms.
/// Returns the union of matched char positions across all atoms.
pub fn search_match_indices(text: &str, atoms: &[Atom], matcher: &mut Matcher) -> Vec<u32> {
    let mut all_indices = Vec::new();
    for atom in atoms {
        let mut buffer = Vec::new();
        let haystack = Utf32Str::new(text, &mut buffer);
        let mut indices = Vec::new();
        if atom.indices(haystack, matcher, &mut indices).is_some() {
            all_indices.extend(indices);
        }
    }
    all_indices.sort_unstable();
    all_indices.dedup();
    all_indices
}
