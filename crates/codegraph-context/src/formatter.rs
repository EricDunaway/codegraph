//! Context formatting for different output formats

use crate::error::ContextError;
use codegraph_types::{Edge, Node, Subgraph};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Output format for context
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContextFormat {
    /// Markdown format (human readable, good for LLMs)
    #[default]
    Markdown,
    /// JSON format (structured, good for tools)
    Json,
    /// Plain text (minimal formatting)
    Plain,
}

/// A formatted node with source code
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormattedNode {
    /// Node ID
    pub id: String,
    /// Node kind
    pub kind: String,
    /// Node name
    pub name: String,
    /// Qualified name
    pub qualified_name: String,
    /// File path
    pub file_path: String,
    /// Line range
    pub lines: (u32, u32),
    /// Source code snippet (if available)
    pub source: Option<String>,
    /// Docstring (if available)
    pub docstring: Option<String>,
    /// Signature (if available)
    pub signature: Option<String>,
}

impl From<&Node> for FormattedNode {
    fn from(node: &Node) -> Self {
        Self {
            id: node.id.0.clone(),
            kind: node.kind.as_str().to_string(),
            name: node.name.clone(),
            qualified_name: node.qualified_name.clone(),
            file_path: node.file_path.clone(),
            lines: (node.start_line, node.end_line),
            source: None,
            docstring: node.docstring.clone(),
            signature: node.signature.clone(),
        }
    }
}

/// A formatted edge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormattedEdge {
    /// Source node ID
    pub source: String,
    /// Target node ID
    pub target: String,
    /// Edge kind
    pub kind: String,
}

impl From<&Edge> for FormattedEdge {
    fn from(edge: &Edge) -> Self {
        Self {
            source: edge.source.0.clone(),
            target: edge.target.0.clone(),
            kind: edge.kind.as_str().to_string(),
        }
    }
}

/// Formatted context output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormattedContext {
    /// Query or task that generated this context
    pub query: String,
    /// Root nodes (entry points)
    pub roots: Vec<String>,
    /// All nodes in context
    pub nodes: Vec<FormattedNode>,
    /// Relationships between nodes
    pub edges: Vec<FormattedEdge>,
    /// File summaries (file path -> symbols)
    pub files: HashMap<String, Vec<String>>,
    /// Total token estimate
    pub estimated_tokens: usize,
}

/// Context formatter
pub struct ContextFormatter {
    format: ContextFormat,
    include_source: bool,
    max_source_lines: usize,
}

impl Default for ContextFormatter {
    fn default() -> Self {
        Self::new(ContextFormat::Markdown)
    }
}

impl ContextFormatter {
    /// Create a new formatter
    pub fn new(format: ContextFormat) -> Self {
        Self {
            format,
            include_source: true,
            max_source_lines: 50,
        }
    }

    /// Set whether to include source code
    pub fn with_source(mut self, include: bool) -> Self {
        self.include_source = include;
        self
    }

    /// Set maximum lines of source to include per node
    pub fn with_max_source_lines(mut self, max: usize) -> Self {
        self.max_source_lines = max;
        self
    }

    /// Format a context result
    pub fn format(&self, context: &FormattedContext) -> Result<String, ContextError> {
        match self.format {
            ContextFormat::Markdown => self.format_markdown(context),
            ContextFormat::Json => self.format_json(context),
            ContextFormat::Plain => self.format_plain(context),
        }
    }

    /// Format as markdown
    fn format_markdown(&self, context: &FormattedContext) -> Result<String, ContextError> {
        let mut output = String::new();

        // Header
        output.push_str(&format!("# Context: {}\n\n", context.query));

        // Summary
        output.push_str(&format!(
            "**{} symbols** across **{} files**\n\n",
            context.nodes.len(),
            context.files.len()
        ));

        // Files section
        if !context.files.is_empty() {
            output.push_str("## Files\n\n");
            for (file, symbols) in &context.files {
                output.push_str(&format!("- `{}` ({} symbols)\n", file, symbols.len()));
            }
            output.push('\n');
        }

        // Nodes section
        output.push_str("## Symbols\n\n");
        for node in &context.nodes {
            output.push_str(&self.format_node_markdown(node));
        }

        // Relationships section
        if !context.edges.is_empty() {
            output.push_str("## Relationships\n\n");
            for edge in &context.edges {
                output.push_str(&format!(
                    "- `{}` --[{}]--> `{}`\n",
                    edge.source, edge.kind, edge.target
                ));
            }
            output.push('\n');
        }

        Ok(output)
    }

    /// Format a single node as markdown
    fn format_node_markdown(&self, node: &FormattedNode) -> String {
        let mut output = String::new();

        output.push_str(&format!(
            "### {} `{}`\n\n",
            node.kind, node.qualified_name
        ));

        output.push_str(&format!(
            "- **File:** `{}:{}:{}`\n",
            node.file_path, node.lines.0, node.lines.1
        ));

        if let Some(ref sig) = node.signature {
            output.push_str(&format!("- **Signature:** `{}`\n", sig));
        }

        if let Some(ref doc) = node.docstring {
            output.push_str(&format!("\n> {}\n", doc.replace('\n', "\n> ")));
        }

        if self.include_source {
            if let Some(ref source) = node.source {
                let lang = Self::detect_language(&node.file_path);
                output.push_str(&format!("\n```{}\n{}\n```\n", lang, source));
            }
        }

        output.push('\n');
        output
    }

    /// Format as JSON
    fn format_json(&self, context: &FormattedContext) -> Result<String, ContextError> {
        serde_json::to_string_pretty(context)
            .map_err(|e| ContextError::Serialization(e.to_string()))
    }

    /// Format as plain text
    fn format_plain(&self, context: &FormattedContext) -> Result<String, ContextError> {
        let mut output = String::new();

        output.push_str(&format!("Context: {}\n", context.query));
        output.push_str(&format!(
            "{} symbols, {} files\n\n",
            context.nodes.len(),
            context.files.len()
        ));

        for node in &context.nodes {
            output.push_str(&format!(
                "[{}] {} ({}:{})\n",
                node.kind, node.qualified_name, node.file_path, node.lines.0
            ));

            if let Some(ref source) = node.source {
                for line in source.lines().take(self.max_source_lines) {
                    output.push_str(&format!("  {}\n", line));
                }
            }
            output.push('\n');
        }

        Ok(output)
    }

    /// Detect language from file extension
    fn detect_language(file_path: &str) -> &'static str {
        let ext = file_path.rsplit('.').next().unwrap_or("");
        match ext {
            "rs" => "rust",
            "ts" | "tsx" => "typescript",
            "js" | "jsx" => "javascript",
            "py" => "python",
            "go" => "go",
            "java" => "java",
            "rb" => "ruby",
            "php" => "php",
            "swift" => "swift",
            "kt" | "kts" => "kotlin",
            "c" | "h" => "c",
            "cpp" | "cc" | "cxx" | "hpp" => "cpp",
            "cs" => "csharp",
            _ => "",
        }
    }

    /// Estimate token count for a string (rough approximation)
    pub fn estimate_tokens(text: &str) -> usize {
        // Rough estimate: 1 token per 4 characters
        text.len() / 4
    }
}

/// Format a subgraph into a formatted context
pub fn format_subgraph(
    subgraph: &Subgraph,
    query: &str,
    source_loader: Option<&dyn Fn(&str, u32, u32) -> Option<String>>,
) -> FormattedContext {
    let mut nodes: Vec<FormattedNode> = Vec::new();
    let mut files: HashMap<String, Vec<String>> = HashMap::new();

    for (_, node) in &subgraph.nodes {
        let mut formatted = FormattedNode::from(node);

        // Load source if available
        if let Some(loader) = source_loader {
            formatted.source = loader(&node.file_path, node.start_line, node.end_line);
        }

        // Track files
        files
            .entry(node.file_path.clone())
            .or_default()
            .push(node.name.clone());

        nodes.push(formatted);
    }

    let edges: Vec<FormattedEdge> = subgraph.edges.iter().map(FormattedEdge::from).collect();

    let roots: Vec<String> = subgraph.roots.iter().map(|id| id.0.clone()).collect();

    // Estimate tokens
    let estimated_tokens = nodes.iter().fold(0, |acc, n| {
        acc + ContextFormatter::estimate_tokens(&n.qualified_name)
            + n.source
                .as_ref()
                .map(|s| ContextFormatter::estimate_tokens(s))
                .unwrap_or(0)
    });

    FormattedContext {
        query: query.to_string(),
        roots,
        nodes,
        edges,
        files,
        estimated_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_types::{Language, NodeKind};

    fn create_test_node(id: &str, name: &str, kind: NodeKind) -> Node {
        Node::new(
            id,
            kind,
            name,
            format!("test.rs::{}", name),
            "test.rs",
            Language::Rust,
            1,
            10,
        )
    }

    #[test]
    fn test_formatted_node_from_node() {
        let node = create_test_node("n1", "myFunc", NodeKind::Function);
        let formatted = FormattedNode::from(&node);

        assert_eq!(formatted.id, "n1");
        assert_eq!(formatted.name, "myFunc");
        assert_eq!(formatted.kind, "function");
    }

    #[test]
    fn test_markdown_formatting() {
        let context = FormattedContext {
            query: "test query".to_string(),
            roots: vec!["n1".to_string()],
            nodes: vec![FormattedNode {
                id: "n1".to_string(),
                kind: "function".to_string(),
                name: "myFunc".to_string(),
                qualified_name: "test::myFunc".to_string(),
                file_path: "test.rs".to_string(),
                lines: (1, 10),
                source: Some("fn myFunc() { }".to_string()),
                docstring: Some("Does something".to_string()),
                signature: Some("fn myFunc()".to_string()),
            }],
            edges: vec![],
            files: [("test.rs".to_string(), vec!["myFunc".to_string()])]
                .into_iter()
                .collect(),
            estimated_tokens: 50,
        };

        let formatter = ContextFormatter::new(ContextFormat::Markdown);
        let output = formatter.format(&context).unwrap();

        assert!(output.contains("# Context: test query"));
        assert!(output.contains("test::myFunc"));
        assert!(output.contains("```rust"));
    }

    #[test]
    fn test_json_formatting() {
        let context = FormattedContext {
            query: "test".to_string(),
            roots: vec![],
            nodes: vec![],
            edges: vec![],
            files: HashMap::new(),
            estimated_tokens: 0,
        };

        let formatter = ContextFormatter::new(ContextFormat::Json);
        let output = formatter.format(&context).unwrap();

        assert!(output.contains("\"query\": \"test\""));
    }

    #[test]
    fn test_token_estimation() {
        let text = "This is a test string with multiple words";
        let tokens = ContextFormatter::estimate_tokens(text);
        // 42 chars / 4 = 10 tokens (approx)
        assert!(tokens >= 10 && tokens <= 12);
    }

    #[test]
    fn test_language_detection() {
        assert_eq!(ContextFormatter::detect_language("test.rs"), "rust");
        assert_eq!(ContextFormatter::detect_language("test.ts"), "typescript");
        assert_eq!(ContextFormatter::detect_language("test.py"), "python");
        assert_eq!(ContextFormatter::detect_language("test.go"), "go");
    }
}
