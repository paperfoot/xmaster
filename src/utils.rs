/// Truncate a string at a safe UTF-8 boundary.
/// Returns at most `max_chars` characters (not bytes).
pub fn safe_truncate(s: &str, max_chars: usize) -> &str {
    if s.chars().count() <= max_chars {
        return s;
    }
    let byte_idx = s
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(s.len());
    &s[..byte_idx]
}
