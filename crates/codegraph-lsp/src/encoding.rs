//! UTF-16 position encoding conversion (I9)
//!
//! LSP uses UTF-16 code units for positions, but Rust strings are UTF-8.
//! This module provides conversion between byte offsets and UTF-16 offsets.

/// Convert a byte offset to a UTF-16 code unit offset
///
/// LSP positions are specified in UTF-16 code units. This function converts
/// a byte offset (as used in Rust strings) to the equivalent UTF-16 offset.
pub fn byte_to_utf16(text: &str, byte_offset: usize) -> u32 {
    let mut utf16_offset = 0u32;
    let mut current_byte = 0usize;

    for ch in text.chars() {
        if current_byte >= byte_offset {
            break;
        }
        current_byte += ch.len_utf8();
        utf16_offset += ch.len_utf16() as u32;
    }

    utf16_offset
}

/// Convert a UTF-16 code unit offset to a byte offset
///
/// This is the inverse of `byte_to_utf16`. Converts an LSP position
/// back to a byte offset for use with Rust strings.
pub fn utf16_to_byte(text: &str, utf16_offset: u32) -> usize {
    let mut current_utf16 = 0u32;
    let mut byte_offset = 0usize;

    for ch in text.chars() {
        if current_utf16 >= utf16_offset {
            break;
        }
        current_utf16 += ch.len_utf16() as u32;
        byte_offset += ch.len_utf8();
    }

    byte_offset
}

/// Convert line and column (0-indexed) to byte offset
///
/// Useful for converting LSP positions to byte offsets.
pub fn position_to_byte_offset(text: &str, line: u32, column_utf16: u32) -> Option<usize> {
    let mut current_line = 0u32;
    let mut line_start = 0usize;

    for (i, ch) in text.char_indices() {
        if current_line == line {
            // Found the line, now find the column
            let line_text = &text[line_start..];
            let line_end = line_text.find('\n').unwrap_or(line_text.len());
            let line_content = &line_text[..line_end];

            let byte_in_line = utf16_to_byte(line_content, column_utf16);
            return Some(line_start + byte_in_line);
        }

        if ch == '\n' {
            current_line += 1;
            line_start = i + 1;
        }
    }

    // Handle last line (or only line with no newline)
    if current_line == line {
        let line_text = &text[line_start..];
        let byte_in_line = utf16_to_byte(line_text, column_utf16);
        return Some(line_start + byte_in_line);
    }

    None
}

/// Convert byte offset to line and column (0-indexed, UTF-16 column)
///
/// Useful for converting Rust positions to LSP positions.
pub fn byte_offset_to_position(text: &str, byte_offset: usize) -> (u32, u32) {
    let mut line = 0u32;
    let mut line_start = 0usize;

    for (i, ch) in text.char_indices() {
        if i >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            line_start = i + 1;
        }
    }

    let line_text = &text[line_start..byte_offset.min(text.len())];
    let column = byte_to_utf16(line_text, line_text.len());

    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_byte_to_utf16_ascii() {
        let text = "Hello World";
        let byte_offset = 6; // Start of "World"
        let utf16_offset = byte_to_utf16(text, byte_offset);
        assert_eq!(utf16_offset, 6); // Same for ASCII
    }

    #[test]
    fn test_byte_to_utf16_emoji() {
        // "Hello 🌍 World" - emoji is 4 bytes but 2 UTF-16 code units
        let text = "Hello 🌍 World";
        let byte_offset = 11; // Start of "World" (after "Hello " + 4-byte emoji + " ")
        let utf16_offset = byte_to_utf16(text, byte_offset);
        // "Hello " = 6, "🌍" = 2 (surrogate pair), " " = 1 = 9
        assert_eq!(utf16_offset, 9);
    }

    #[test]
    fn test_utf16_to_byte_ascii() {
        let text = "Hello World";
        let utf16_offset = 6;
        let byte_offset = utf16_to_byte(text, utf16_offset);
        assert_eq!(byte_offset, 6);
    }

    #[test]
    fn test_utf16_to_byte_emoji() {
        let text = "Hello 🌍 World";
        let utf16_offset = 9; // After "Hello 🌍 "
        let byte_offset = utf16_to_byte(text, utf16_offset);
        assert_eq!(byte_offset, 11); // "Hello " (6) + emoji (4) + " " (1)
    }

    #[test]
    fn test_utf16_to_byte_roundtrip() {
        let text = "Hello 🌍 World 你好";

        for (byte_idx, _) in text.char_indices() {
            let utf16 = byte_to_utf16(text, byte_idx);
            let back = utf16_to_byte(text, utf16);
            assert_eq!(back, byte_idx, "Roundtrip failed at byte {}", byte_idx);
        }
    }

    #[test]
    fn test_position_to_byte_offset() {
        let text = "line 1\nline 2\nline 3";

        // First line, column 0
        assert_eq!(position_to_byte_offset(text, 0, 0), Some(0));
        // First line, column 4
        assert_eq!(position_to_byte_offset(text, 0, 4), Some(4));
        // Second line, column 0
        assert_eq!(position_to_byte_offset(text, 1, 0), Some(7));
        // Third line, column 2
        assert_eq!(position_to_byte_offset(text, 2, 2), Some(16));
    }

    #[test]
    fn test_byte_offset_to_position() {
        let text = "line 1\nline 2\nline 3";

        // Start of file
        assert_eq!(byte_offset_to_position(text, 0), (0, 0));
        // Middle of first line
        assert_eq!(byte_offset_to_position(text, 4), (0, 4));
        // Start of second line
        assert_eq!(byte_offset_to_position(text, 7), (1, 0));
        // Middle of third line
        assert_eq!(byte_offset_to_position(text, 16), (2, 2));
    }

    #[test]
    fn test_position_with_unicode() {
        let text = "你好\nworld";

        // "你好" is 6 bytes, 2 chars, 2 UTF-16 units each = 4 UTF-16 units
        // Start of "world" is at byte 7 (6 + newline)
        assert_eq!(position_to_byte_offset(text, 1, 0), Some(7));

        // Line 0, column 2 (after first Chinese char)
        // "你" is 3 bytes, 1 UTF-16 unit
        let offset = position_to_byte_offset(text, 0, 1);
        assert_eq!(offset, Some(3)); // After first Chinese char
    }
}
