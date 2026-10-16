//! Shared line segmentation for byte-preserving edits and automatic chunks.

/// Includes each original LF, CRLF or CR terminator, without inventing a
/// trailing empty line. Empty text has no lines.
pub(super) fn lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    };
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let (mut start, mut index) = (0, 0);
    while index < bytes.len() {
        let length = match bytes[index] {
            b'\n' => 1,
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => 2,
            b'\r' => 1,
            _ => {
                index += 1;
                continue;
            }
        };
        result.push(&text[start..index + length]);
        index += length;
        start = index;
    }
    if start < text.len() {
        result.push(&text[start..]);
    }
    result
}
