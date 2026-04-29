pub(super) fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

pub(super) fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{encode_path_segment, escape_html};

    #[test]
    fn escape_html_escapes_markup_sensitive_characters() {
        assert_eq!(
            escape_html("<button title=\"Ada's\">&</button>"),
            "&lt;button title=&quot;Ada&#39;s&quot;&gt;&amp;&lt;/button&gt;"
        );
    }

    #[test]
    fn encode_path_segment_percent_encodes_reserved_bytes() {
        assert_eq!(encode_path_segment("team leads/ä"), "team%20leads%2F%C3%A4");
    }
}
