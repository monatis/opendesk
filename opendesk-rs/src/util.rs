//! Small shared helpers.

/// The longest prefix of `s` that is at most `max_bytes` long and ends on a
/// char boundary. Slicing `&s[..n]` directly panics inside a multi-byte
/// character (e.g. Turkish "ş").
pub fn truncate_bytes(s: &str, max_bytes: usize) -> &str {
    let mut end = max_bytes.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_never_splits_a_char() {
        assert_eq!(truncate_bytes("abc", 10), "abc");
        assert_eq!(truncate_bytes("abc", 2), "ab");
        // "ş" is two bytes; cutting at 1 must not split it.
        assert_eq!(truncate_bytes("şa", 1), "");
        assert_eq!(truncate_bytes("aşb", 2), "a");
        assert_eq!(truncate_bytes("aşb", 3), "aş");
    }
}
