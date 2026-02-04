//! Generic tree-sitter based extractor
//!
//! This module provides a single, configurable extractor that works with any
//! language supported by tree-sitter. Language-specific behavior is driven
//! by configuration, not separate extractor implementations.

use crate::error::ExtractionError;
use crate::languages::{get_language_config, LanguageConfig};
use crate::parser::TreeSitterParser;
use codegraph_types::{
    Edge, EdgeKind, ExtractionResult, Language, Node, NodeId, NodeKind, Visibility,
};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use tree_sitter::Node as TsNode;

/// Generic tree-sitter extractor that works with any configured language
pub struct TreeSitterExtractor {
    parser: TreeSitterParser,
}

impl Default for TreeSitterExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeSitterExtractor {
    /// Create a new tree-sitter extractor
    pub fn new() -> Self {
        Self {
            parser: TreeSitterParser::new(),
        }
    }

    /// Extract nodes and edges from source code
    pub fn extract(
        &mut self,
        source: &str,
        file_path: &str,
        lang: Language,
    ) -> Result<ExtractionResult, ExtractionError> {
        let config = get_language_config(lang).ok_or_else(|| {
            ExtractionError::UnsupportedLanguage(lang.as_str().to_string())
        })?;

        let mut result = ExtractionResult::default();
        let mut seen_ids = HashSet::new();

        // Create file node
        let file_id = generate_node_id(file_path, NodeKind::File, file_path, 0);
        seen_ids.insert(file_id.as_str().to_string());
        let file_node = Node::new(
            file_id.clone(),
            NodeKind::File,
            file_path,
            file_path,
            file_path,
            lang,
            1,
            source.lines().count() as u32,
        );
        result.nodes.push(file_node);

        // Parse with tree-sitter
        let tree = self.parser.parse(source, lang)?;
        let root = tree.root_node();

        // Walk the AST
        self.walk_ast(root, source, file_path, &file_id, lang, &config, &mut result, &mut seen_ids);

        Ok(result)
    }

    /// Walk the AST and extract nodes based on configuration
    fn walk_ast(
        &self,
        node: TsNode<'_>,
        source: &str,
        file_path: &str,
        parent_id: &NodeId,
        lang: Language,
        config: &LanguageConfig,
        result: &mut ExtractionResult,
        seen_ids: &mut HashSet<String>,
    ) {
        let node_kind = node.kind();

        // Check if this node type is mapped
        if let Some(mapping) = config.node_mappings.get(node_kind) {
            // Extract this node
            if let Some(name) = self.get_node_name(node, source, config) {
                let line = node.start_position().row as u32 + 1;
                let end_line = node.end_position().row as u32 + 1;
                let decorators = self.extract_decorators(node, source, config);
                let is_exported = self.is_exported(node, source, config, &name, lang);

                let node_id = generate_node_id(file_path, mapping.kind, &name, line);

                // Skip if already seen (prevents duplicates from multiple AST paths)
                if seen_ids.contains(node_id.as_str()) {
                    return;
                }
                seen_ids.insert(node_id.as_str().to_string());

                let mut extracted_node = Node::new(
                    node_id.clone(),
                    mapping.kind,
                    &name,
                    format!("{file_path}::{name}"),
                    file_path,
                    lang,
                    line,
                    end_line,
                );
                extracted_node.decorators = decorators;
                extracted_node.is_exported = is_exported;
                extracted_node.visibility = if is_exported {
                    Some(Visibility::Public)
                } else {
                    Some(Visibility::Private)
                };

                // Add contains edge from parent
                result.edges.push(Edge::new(
                    parent_id.clone(),
                    node_id.clone(),
                    EdgeKind::Contains,
                ));

                // If this node has children, process them with this node as parent
                if mapping.has_children {
                    if let Some(body_field) = config.body_field {
                        if let Some(body) = node.child_by_field_name(body_field) {
                            self.process_children(
                                body, source, file_path, &node_id, lang, config,
                                &mapping.child_types, result, seen_ids,
                            );
                        }
                    }
                }

                result.nodes.push(extracted_node);
                return; // Don't recurse into children we've already processed
            }
        }

        // Handle export statements specially (they wrap other declarations)
        if config.export_indicators.contains(&node_kind) {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.walk_ast(child, source, file_path, parent_id, lang, config, result, seen_ids);
            }
            return;
        }

        // Recurse into children for unmapped nodes
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_ast(child, source, file_path, parent_id, lang, config, result, seen_ids);
        }
    }

    /// Process children of a container node
    fn process_children(
        &self,
        node: TsNode<'_>,
        source: &str,
        file_path: &str,
        parent_id: &NodeId,
        lang: Language,
        config: &LanguageConfig,
        child_types: &[&str],
        result: &mut ExtractionResult,
        seen_ids: &mut HashSet<String>,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child_types.contains(&child.kind()) {
                if let Some(mapping) = config.node_mappings.get(child.kind()) {
                    if let Some(name) = self.get_node_name(child, source, config) {
                        let line = child.start_position().row as u32 + 1;
                        let end_line = child.end_position().row as u32 + 1;

                        let node_id = generate_node_id(file_path, mapping.kind, &name, line);

                        // Skip if already seen
                        if seen_ids.contains(node_id.as_str()) {
                            continue;
                        }
                        seen_ids.insert(node_id.as_str().to_string());

                        let decorators = self.extract_decorators(child, source, config);

                        // For methods, determine visibility based on name or modifiers
                        let is_private = self.is_private_member(&name, child, source, lang);

                        let mut extracted_node = Node::new(
                            node_id.clone(),
                            mapping.kind,
                            &name,
                            format!("{file_path}::{name}"),
                            file_path,
                            lang,
                            line,
                            end_line,
                        );
                        extracted_node.decorators = decorators;
                        extracted_node.visibility = if is_private {
                            Some(Visibility::Private)
                        } else {
                            Some(Visibility::Public)
                        };

                        result.edges.push(Edge::new(
                            parent_id.clone(),
                            node_id.clone(),
                            EdgeKind::Contains,
                        ));
                        result.nodes.push(extracted_node);
                    }
                }
            }
        }
    }

    /// Get the name of a node
    fn get_node_name(&self, node: TsNode<'_>, source: &str, config: &LanguageConfig) -> Option<String> {
        // Try the configured field name
        node.child_by_field_name(config.name_field)
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(|s| s.to_string())
            .or_else(|| {
                // Fallback: try common field names
                for field in &["name", "identifier", "simple_identifier"] {
                    if let Some(name) = node
                        .child_by_field_name(field)
                        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                        .map(|s| s.to_string())
                    {
                        return Some(name);
                    }
                }
                None
            })
    }

    /// Extract decorators/attributes from preceding siblings
    fn extract_decorators(&self, node: TsNode<'_>, source: &str, config: &LanguageConfig) -> Vec<String> {
        let decorator_type = match config.decorator_node_type {
            Some(t) => t,
            None => return vec![],
        };

        let mut decorators = Vec::new();

        if let Some(parent) = node.parent() {
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
                // Stop when we reach our node
                if child.id() == node.id() {
                    break;
                }
                // Collect decorators
                if child.kind() == decorator_type {
                    if let Ok(text) = child.utf8_text(source.as_bytes()) {
                        // Clean up the decorator text based on language
                        let decorator_text = match config.language {
                            Language::Rust => text
                                .trim_start_matches("#[")
                                .trim_end_matches(']')
                                .to_string(),
                            _ => text.trim_start_matches('@').to_string(),
                        };
                        decorators.push(decorator_text);
                    }
                }
            }
        }

        decorators
    }

    /// Check if a node is exported/public
    fn is_exported(
        &self,
        node: TsNode<'_>,
        source: &str,
        config: &LanguageConfig,
        name: &str,
        lang: Language,
    ) -> bool {
        // Go uses capitalization for exports
        if lang == Language::Go {
            return name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
        }

        // Check parent for export statement
        if let Some(parent) = node.parent() {
            if config.export_indicators.contains(&parent.kind()) {
                return true;
            }
        }

        // Check for visibility modifiers in children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let child_kind = child.kind();
            if config.export_indicators.contains(&child_kind) {
                return true;
            }
            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                if text == "export" || text == "pub" || text == "public" {
                    return true;
                }
            }
        }

        false
    }

    /// Check if a member is private (for methods/fields)
    fn is_private_member(&self, name: &str, node: TsNode<'_>, source: &str, lang: Language) -> bool {
        match lang {
            Language::Python => name.starts_with('_') && !name.starts_with("__"),
            Language::Go => name.chars().next().map(|c| c.is_lowercase()).unwrap_or(true),
            Language::Rust => {
                // Check for pub modifier
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "visibility_modifier" {
                        return false; // Has pub modifier, so not private
                    }
                }
                true // Default to private in Rust
            }
            _ => {
                // Check for private keyword or modifiers
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if let Ok(text) = child.utf8_text(source.as_bytes()) {
                        if text == "private" || text == "#" {
                            return true;
                        }
                    }
                }
                false
            }
        }
    }
}

/// Generate a unique node ID
pub fn generate_node_id(file_path: &str, kind: NodeKind, name: &str, line: u32) -> NodeId {
    let mut hasher = Sha256::new();
    hasher.update(format!("{file_path}:{kind}:{name}:{line}"));
    let hash = format!("{:x}", hasher.finalize());
    NodeId::new(format!("{}:{}", kind.as_str(), &hash[..32]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "lang-typescript")]
    fn test_extract_typescript_class_with_decorator() {
        let mut extractor = TreeSitterExtractor::new();
        let source = r#"
@Injectable()
export class MyService {
    constructor() {}

    greet(): string {
        return "hello";
    }
}
"#;
        let result = extractor.extract(source, "test.ts", Language::TypeScript).unwrap();

        let class_node = result.nodes.iter().find(|n| n.kind == NodeKind::Class).unwrap();
        assert_eq!(class_node.name, "MyService");
        assert!(class_node.decorators.iter().any(|d| d.contains("Injectable")));
        assert!(class_node.is_exported);
    }

    #[test]
    #[cfg(feature = "lang-typescript")]
    fn test_extract_appsync_decorator() {
        let mut extractor = TreeSitterExtractor::new();
        let source = r#"
@AppSyncQuery({
  methodName: "guests_searchEntitlementInstances",
  input: GuestsSearchEntitlementInstancesInput,
})
export class GuestsSearchEntitlementInstancesService {
    async execute(): Promise<void> {}
}
"#;
        let result = extractor.extract(source, "test.ts", Language::TypeScript).unwrap();

        let class_node = result.nodes.iter().find(|n| n.kind == NodeKind::Class).unwrap();
        assert_eq!(class_node.name, "GuestsSearchEntitlementInstancesService");
        assert!(class_node.decorators.iter().any(|d| d.contains("AppSyncQuery")));
        assert!(class_node.decorators.iter().any(|d| d.contains("methodName")));
    }

    #[test]
    #[cfg(feature = "lang-rust")]
    fn test_extract_rust_with_derive() {
        let mut extractor = TreeSitterExtractor::new();
        let source = r#"
#[derive(Debug, Clone, Serialize)]
pub struct MyStruct {
    pub name: String,
    age: u32,
}
"#;
        let result = extractor.extract(source, "test.rs", Language::Rust).unwrap();

        let struct_node = result.nodes.iter().find(|n| n.kind == NodeKind::Struct).unwrap();
        assert_eq!(struct_node.name, "MyStruct");
        assert!(struct_node.decorators.iter().any(|d| d.contains("derive")));
        assert!(struct_node.is_exported);
    }

    #[test]
    #[cfg(feature = "lang-rust")]
    fn test_extract_rust_test_attribute() {
        let mut extractor = TreeSitterExtractor::new();
        let source = r#"
#[test]
fn test_something() {
    assert!(true);
}
"#;
        let result = extractor.extract(source, "test.rs", Language::Rust).unwrap();

        let func = result.nodes.iter().find(|n| n.kind == NodeKind::Function).unwrap();
        assert_eq!(func.name, "test_something");
        assert!(func.decorators.iter().any(|d| d == "test"));
    }

    #[test]
    #[cfg(feature = "lang-python")]
    fn test_extract_python_class_with_decorator() {
        let mut extractor = TreeSitterExtractor::new();
        let source = r#"
@dataclass
class User:
    name: str
    age: int

    def greet(self):
        return f"Hello, {self.name}"
"#;
        let result = extractor.extract(source, "test.py", Language::Python).unwrap();

        let class_node = result.nodes.iter().find(|n| n.kind == NodeKind::Class).unwrap();
        assert_eq!(class_node.name, "User");
        assert!(class_node.decorators.iter().any(|d| d == "dataclass"));
    }

    #[test]
    #[cfg(feature = "lang-go")]
    fn test_extract_go_struct() {
        let mut extractor = TreeSitterExtractor::new();
        let source = r#"
package main

type User struct {
    Name string
    Age  int
}

func (u *User) Greet() string {
    return "Hello"
}
"#;
        let result = extractor.extract(source, "main.go", Language::Go).unwrap();

        // Go export detection uses capitalization
        let funcs: Vec<_> = result.nodes.iter().filter(|n| n.kind == NodeKind::Function || n.kind == NodeKind::Method).collect();
        assert!(!funcs.is_empty());
    }
}
