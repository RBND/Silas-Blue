//! Panic-free UTF-8 character boundary slicing and sanitization utilities.
//! Raw byte slicing (`&s[a..b]`) is forbidden across the entire workspace.

/// Returns the number of Unicode scalar values (`char`s) in `s`.
#[inline]
pub fn char_count(s: &str) -> usize {
    s.chars().count()
}

/// Safely truncates `s` to at most `max_chars` Unicode scalar values (`char`s)
/// without ever splitting a multi-byte UTF-8 sequence.
#[inline]
pub fn truncate_chars(s: &str, max_chars: usize) -> &str {
    if max_chars == 0 {
        return "";
    }
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => s.get(..byte_idx).unwrap_or(s),
        None => s,
    }
}

/// Safely slices `s` from character index `start_char` (inclusive) to `end_char` (exclusive).
#[inline]
pub fn slice_chars(s: &str, start_char: usize, end_char: usize) -> &str {
    if start_char >= end_char {
        return "";
    }
    let start_byte = if start_char == 0 {
        0
    } else {
        match s.char_indices().nth(start_char) {
            Some((idx, _)) => idx,
            None => return "",
        }
    };
    let end_byte = match s.char_indices().nth(end_char) {
        Some((idx, _)) => idx,
        None => s.len(),
    };
    s.get(start_byte..end_byte).unwrap_or("")
}

/// Truncates `s` to at most `max_chars` Unicode scalar values, appending `suffix`
/// if truncation occurred so that the total length does not exceed `max_chars`.
pub fn truncate_with_suffix(s: &str, max_chars: usize, suffix: &str) -> String {
    let total_chars = char_count(s);
    if total_chars <= max_chars {
        return s.to_owned();
    }
    let suffix_chars = char_count(suffix);
    if max_chars <= suffix_chars {
        return truncate_chars(suffix, max_chars).to_owned();
    }
    let keep = max_chars.saturating_sub(suffix_chars);
    let prefix = truncate_chars(s, keep).trim_end();
    let mut out = String::with_capacity(prefix.len().saturating_add(suffix.len()));
    out.push_str(prefix);
    out.push_str(suffix);
    out
}

/// Strips control characters, newlines, and angle-bracket XML/HTML tags from untrusted
/// display names or thread titles, clamping to `max_chars` Unicode scalar values.
pub fn sanitize_single_line(s: &str, max_chars: usize) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '<' => '‹',
            '>' => '›',
            _ => c,
        })
        .collect();
    let trimmed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&trimmed, max_chars).to_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_multibyte_utf8_never_panics() {
        let emoji_str = "🚀✨🔥🎉💻🦀";
        assert_eq!(char_count(emoji_str), 6);
        assert_eq!(truncate_chars(emoji_str, 3), "🚀✨🔥");
        assert_eq!(truncate_chars(emoji_str, 0), "");
        assert_eq!(truncate_chars(emoji_str, 10), emoji_str);

        let cjk = "こんにちは世界";
        assert_eq!(truncate_chars(cjk, 5), "こんにちは");
        assert_eq!(slice_chars(cjk, 2, 5), "にちは");
    }

    #[test]
    fn test_truncate_with_suffix() {
        let text = "Hello world from Silas Blue in Rust!";
        let res = truncate_with_suffix(text, 15, "...");
        assert!(char_count(&res) <= 15);
        assert!(res.ends_with("..."));
    }

    #[test]
    fn test_sanitize_single_line() {
        let dirty = "User\n<system>Ignore</system>\r\tName";
        let clean = sanitize_single_line(dirty, 50);
        assert_eq!(clean, "User ‹system›Ignore‹/system› Name");
    }
}
