//! Code-fence-aware and UTF-8 scalar-safe markdown paginator for Discord messages.

use std::sync::LazyLock;

use regex::Regex;

use crate::text::{char_count, slice_chars};

/// Headroom reserved on each page so prepending `"Page X/Y\n"` (or `**[Part X/Y]**\n`)
/// and appending closing code fences (`"\n```"`) never exceeds `max_chars` or Discord's 2,000-char limit.
const HEADER_AND_FENCE_SLACK: usize = 32;

#[allow(clippy::expect_used)]
static FENCE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^ {0,3}(`{3,}|~{3,})([a-zA-Z0-9_+-]*)\s*$").expect("valid fence regex"));

#[derive(Debug, Clone)]
struct OpenFence {
    delimiter: String, // e.g. "```" or "~~~~"
    lang: String,      // e.g. "rust"
}

impl OpenFence {
    fn opening_line(&self) -> String {
        format!("{}{}\n", self.delimiter, self.lang)
    }

    fn closing_line(&self) -> String {
        format!("\n{}", self.delimiter)
    }
}

/// Splits markdown text into pages where every page's Unicode scalar count (`char_count`)
/// is guaranteed `<= max_chars` (clamped to `100..=1850`), while cleanly closing and
/// re-opening fenced code blocks (` ```lang `) across page boundaries.
pub fn paginate_markdown(text: &str, max_chars: usize) -> Vec<String> {
    let hard_limit = max_chars.clamp(100, 1850);
    let body_budget = hard_limit.saturating_sub(HEADER_AND_FENCE_SLACK).max(60);

    if text.is_empty() {
        return vec![String::new()];
    }
    if char_count(text) <= hard_limit {
        return vec![text.to_owned()];
    }

    let mut pages: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;
    let mut active_fence: Option<OpenFence> = None;

    for raw_line in text.split_inclusive('\n') {
        let line_without_nl = raw_line.trim_end_matches(&['\r', '\n'][..]);
        let fence_transition = detect_fence_transition(line_without_nl, active_fence.as_ref());

        let mut remaining_line = raw_line;
        while !remaining_line.is_empty() {
            let rem_chars = char_count(remaining_line);
            if current_chars.saturating_add(rem_chars) <= body_budget {
                current.push_str(remaining_line);
                current_chars = current_chars.saturating_add(rem_chars);
                break;
            }

            // If current page already has content and the remaining line itself fits on a fresh page,
            // flush the current page first so we don't split a line unnecessarily.
            let fresh_fence_overhead = active_fence
                .as_ref()
                .map_or(0, |f| char_count(&f.opening_line()));
            if !current.is_empty()
                && rem_chars.saturating_add(fresh_fence_overhead) <= body_budget
            {
                flush_page(
                    &mut pages,
                    &mut current,
                    &mut current_chars,
                    active_fence.as_ref(),
                );
                continue;
            }

            // Otherwise, remaining_line is longer than the remaining space on this page.
            let available = body_budget.saturating_sub(current_chars);
            if available < 20 && !current.is_empty() {
                flush_page(
                    &mut pages,
                    &mut current,
                    &mut current_chars,
                    active_fence.as_ref(),
                );
                continue;
            }

            let take_limit = available.max(1);
            let split_chars = find_clean_char_split(remaining_line, take_limit);
            let part = slice_chars(remaining_line, 0, split_chars);
            remaining_line = slice_chars(remaining_line, split_chars, rem_chars);

            current.push_str(part);
            current_chars = current_chars.saturating_add(char_count(part));

            if !remaining_line.is_empty() {
                flush_page(
                    &mut pages,
                    &mut current,
                    &mut current_chars,
                    active_fence.as_ref(),
                );
            }
        }

        // Update fence state after processing the line
        match fence_transition {
            FenceTransition::Opened(f) => active_fence = Some(f),
            FenceTransition::Closed => active_fence = None,
            FenceTransition::None => {}
        }
    }

    if !current.is_empty() {
        if let Some(ref fence) = active_fence {
            // Ensure unclosed fence in raw text is closed cleanly
            if !current.trim_end().ends_with(&fence.delimiter) {
                current.push_str(&fence.closing_line());
            }
        }
        pages.push(current);
    }

    if pages.is_empty() {
        pages.push(String::new());
    }
    pages
}

enum FenceTransition {
    None,
    Opened(OpenFence),
    Closed,
}

fn detect_fence_transition(line: &str, active: Option<&OpenFence>) -> FenceTransition {
    let Some(caps) = FENCE_RE.captures(line) else {
        return FenceTransition::None;
    };
    let delim = caps.get(1).map_or("", |m| m.as_str());
    let lang = caps.get(2).map_or("", |m| m.as_str());

    match active {
        None => FenceTransition::Opened(OpenFence {
            delimiter: delim.to_owned(),
            lang: lang.to_owned(),
        }),
        Some(open) => {
            let same_char = delim.chars().next() == open.delimiter.chars().next();
            if same_char && delim.len() >= open.delimiter.len() && lang.is_empty() {
                FenceTransition::Closed
            } else {
                FenceTransition::None
            }
        }
    }
}

fn flush_page(
    pages: &mut Vec<String>,
    current: &mut String,
    current_chars: &mut usize,
    active_fence: Option<&OpenFence>,
) {
    if let Some(fence) = active_fence {
        current.push_str(&fence.closing_line());
    }
    pages.push(std::mem::take(current));
    if let Some(fence) = active_fence {
        let open_line = fence.opening_line();
        *current_chars = char_count(&open_line);
        current.push_str(&open_line);
    } else {
        *current_chars = 0;
    }
}

fn find_clean_char_split(s: &str, max_chars: usize) -> usize {
    let candidate = slice_chars(s, 0, max_chars);
    let total = char_count(candidate);
    if total < max_chars {
        return total;
    }
    // Look for the last whitespace character in the second half of `candidate`
    let half = max_chars / 2;
    let mut best_split = None;
    for (char_idx, ch) in candidate.chars().enumerate() {
        if char_idx >= half && ch.is_whitespace() {
            best_split = Some(char_idx.saturating_add(1));
        }
    }
    best_split.unwrap_or(max_chars)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_paginate_closes_and_reopens_code_fences() {
        let mut code = String::from("Here is some Rust code:\n```rust\n");
        for i in 0..40 {
            code.push_str(&format!("let value_{i} = \"🦀 emoji test {i}\";\n"));
        }
        code.push_str("```\nDone!");

        let pages = paginate_markdown(&code, 500);
        assert!(pages.len() >= 2);
        for p in &pages {
            assert!(char_count(p) <= 500);
        }
        // First page must end with closing ```
        assert!(pages[0].trim_end().ends_with("```"));
        // Second page must start with reopening ```rust
        assert!(pages[1].starts_with("```rust\n"));
    }

    #[test]
    fn test_paginate_long_unbroken_multibyte_line() {
        let unbroken = "🚀✨🦀".repeat(500); // 1500 chars
        let pages = paginate_markdown(&unbroken, 600);
        assert!(pages.len() >= 3);
        for p in &pages {
            assert!(char_count(p) <= 600);
        }
        let rejoined: String = pages.concat();
        assert_eq!(rejoined, unbroken);
    }
}
