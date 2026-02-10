//! Code snippet extraction (E4)
//!
//! Extracts source code snippets from nodes with configurable line limits.

use codegraph_types::Node;

/// Default maximum lines for code snippets
pub const DEFAULT_MAX_LINES: usize = 50;

/// Truncation marker appended to long snippets
pub const TRUNCATION_MARKER: &str = "// ... truncated";

/// Extract a code snippet for a node from source code
///
/// Returns the source code for the node's line range, truncated if needed.
/// Returns None if the source doesn't contain the node's lines.
pub fn extract_code_snippet(source: &str, node: &Node, max_lines: usize) -> Option<String> {
    extract_code_snippet_for_range(source, node.start_line, node.end_line, max_lines)
}

/// Extract a code snippet for a given line range
///
/// Lines are 1-indexed (matching Node's line numbering).
/// Returns None if the range is invalid or source is empty.
pub fn extract_code_snippet_for_range(
    source: &str,
    start_line: u32,
    end_line: u32,
    max_lines: usize,
) -> Option<String> {
    if source.is_empty() || start_line == 0 || end_line < start_line {
        return None;
    }

    let lines: Vec<&str> = source.lines().collect();
    let total_lines = lines.len();

    // Convert to 0-indexed
    let start_idx = (start_line as usize).saturating_sub(1);
    let end_idx = (end_line as usize).min(total_lines);

    if start_idx >= total_lines {
        return None;
    }

    let snippet_lines = &lines[start_idx..end_idx];
    let line_count = snippet_lines.len();

    if line_count == 0 {
        return None;
    }

    if line_count <= max_lines {
        // No truncation needed
        Some(snippet_lines.join("\n"))
    } else {
        // Truncate and add marker
        let truncated: Vec<&str> = snippet_lines.iter().take(max_lines).copied().collect();
        Some(format!("{}\n{}", truncated.join("\n"), TRUNCATION_MARKER))
    }
}

/// Check if a snippet was truncated
pub fn is_truncated(snippet: &str) -> bool {
    snippet.ends_with(TRUNCATION_MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_types::{Language, NodeKind};

    fn make_test_node(start_line: u32, end_line: u32) -> Node {
        Node::new(
            "test_node",
            NodeKind::Function,
            "testFunc",
            "test::testFunc",
            "test.ts",
            Language::TypeScript,
            start_line,
            end_line,
        )
    }

    #[test]
    fn test_extract_code_snippet_under_limit() {
        let source = r#"function add(a: number, b: number): number {
    return a + b;
}"#;

        let node = make_test_node(1, 3);
        let snippet = extract_code_snippet(source, &node, 50);

        assert!(snippet.is_some());
        let snippet = snippet.unwrap();
        assert!(snippet.contains("return a + b"));
        assert!(!is_truncated(&snippet));
    }

    #[test]
    fn test_extract_code_snippet_truncates_long_functions() {
        // Generate 100-line function
        let mut lines = vec!["function longFunc() {".to_string()];
        for i in 0..98 {
            lines.push(format!("    const x{} = {};", i, i));
        }
        lines.push("}".to_string());
        let source = lines.join("\n");

        let node = make_test_node(1, 100);
        let snippet = extract_code_snippet(&source, &node, 50);

        assert!(snippet.is_some());
        let snippet = snippet.unwrap();
        let line_count = snippet.lines().count();
        assert!(line_count <= 51, "Expected max 51 lines (50 + truncation marker), got {}", line_count);
        assert!(is_truncated(&snippet), "Should be truncated");
    }

    #[test]
    fn test_extract_code_snippet_empty_source() {
        let node = make_test_node(1, 5);
        let snippet = extract_code_snippet("", &node, 50);
        assert!(snippet.is_none());
    }

    #[test]
    fn test_extract_code_snippet_invalid_range() {
        let source = "line1\nline2\nline3";
        let node = make_test_node(10, 20); // Beyond source
        let snippet = extract_code_snippet(source, &node, 50);
        assert!(snippet.is_none());
    }

    #[test]
    fn test_extract_code_snippet_exact_limit() {
        let mut lines = Vec::new();
        for i in 1..=50 {
            lines.push(format!("line {}", i));
        }
        let source = lines.join("\n");

        let node = make_test_node(1, 50);
        let snippet = extract_code_snippet(&source, &node, 50);

        assert!(snippet.is_some());
        let snippet = snippet.unwrap();
        assert!(!is_truncated(&snippet), "Exact limit should not truncate");
        assert_eq!(snippet.lines().count(), 50);
    }

    #[test]
    fn test_extract_code_snippet_single_line() {
        let source = "const x = 1;";
        let node = make_test_node(1, 1);
        let snippet = extract_code_snippet(source, &node, 50);

        assert!(snippet.is_some());
        assert_eq!(snippet.unwrap(), "const x = 1;");
    }

    #[test]
    fn test_extract_code_snippet_middle_of_file() {
        let source = "line1\nline2\nline3\nline4\nline5";
        let node = make_test_node(2, 4);
        let snippet = extract_code_snippet(source, &node, 50);

        assert!(snippet.is_some());
        let snippet = snippet.unwrap();
        assert_eq!(snippet, "line2\nline3\nline4");
    }

    #[test]
    fn test_extract_code_snippet_preserves_indentation() {
        let source = r#"class Foo {
    constructor() {
        this.x = 1;
    }
}"#;

        let node = make_test_node(1, 5);
        let snippet = extract_code_snippet(source, &node, 50);

        assert!(snippet.is_some());
        let snippet = snippet.unwrap();
        assert!(snippet.contains("        this.x = 1;"), "Should preserve indentation");
    }

    #[test]
    fn test_is_truncated() {
        assert!(is_truncated("some code\n// ... truncated"));
        assert!(!is_truncated("some code"));
        assert!(!is_truncated("// ... truncated somewhere in middle\nmore code"));
    }
}
