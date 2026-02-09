//! Generic tree-sitter based extractor
//!
//! This module provides a single, configurable extractor that works with any
//! language supported by tree-sitter. Language-specific behavior is driven
//! by configuration, not separate extractor implementations.

use crate::error::ExtractionError;
use crate::languages::{get_language_config, LanguageConfig};
use crate::parser::TreeSitterParser;
use crate::snippet::{extract_code_snippet_for_range, DEFAULT_MAX_LINES};
use codegraph_types::{
    Edge, EdgeKind, ExtractionResult, Language, Node, NodeId, NodeKind, UnresolvedReference,
    Visibility, BUILTIN_SYMBOLS,
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

        // Handle import statements before checking mapped nodes
        if config.import_node_types.contains(&node_kind) {
            self.extract_import(node, source, file_path, parent_id, lang, config, result, seen_ids);
            return;
        }

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
                extracted_node.code_snippet =
                    extract_code_snippet_for_range(source, line, end_line, DEFAULT_MAX_LINES);

                // Add contains edge from parent
                result.edges.push(Edge::new(
                    parent_id.clone(),
                    node_id.clone(),
                    EdgeKind::Contains,
                ));

                // Extract calls from function/method bodies
                if matches!(mapping.kind, NodeKind::Function | NodeKind::Method) {
                    if let Some(call_type) = config.call_node_type {
                        self.extract_calls_from_subtree(
                            node, source, file_path, &node_id, lang, config, call_type, result,
                        );
                    }
                }

                // Extract inheritance from class/interface/struct
                if matches!(
                    mapping.kind,
                    NodeKind::Class | NodeKind::Interface | NodeKind::Struct
                ) {
                    self.extract_inheritance(node, source, file_path, &node_id, lang, config, result);
                }

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
                        extracted_node.code_snippet =
                            extract_code_snippet_for_range(source, line, end_line, DEFAULT_MAX_LINES);

                        result.edges.push(Edge::new(
                            parent_id.clone(),
                            node_id.clone(),
                            EdgeKind::Contains,
                        ));

                        // Extract calls from method bodies
                        if matches!(mapping.kind, NodeKind::Function | NodeKind::Method) {
                            if let Some(call_type) = config.call_node_type {
                                self.extract_calls_from_subtree(
                                    child, source, file_path, &node_id, lang, config, call_type, result,
                                );
                            }
                        }

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

    // =========================================================================
    // Import Extraction
    // =========================================================================

    /// Extract an import statement node and create an unresolved reference
    fn extract_import(
        &self,
        node: TsNode<'_>,
        source: &str,
        file_path: &str,
        parent_id: &NodeId,
        lang: Language,
        _config: &LanguageConfig,
        result: &mut ExtractionResult,
        seen_ids: &mut HashSet<String>,
    ) {
        let import_name = match self.extract_import_name(node, source, lang) {
            Some(name) => name,
            None => return,
        };

        let line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;
        let node_id = generate_node_id(file_path, NodeKind::Import, &import_name, line);

        if seen_ids.contains(node_id.as_str()) {
            return;
        }
        seen_ids.insert(node_id.as_str().to_string());

        let mut import_node = Node::new(
            node_id.clone(),
            NodeKind::Import,
            &import_name,
            format!("{file_path}::{import_name}"),
            file_path,
            lang,
            line,
            end_line,
        );
        import_node.code_snippet =
            extract_code_snippet_for_range(source, line, end_line, DEFAULT_MAX_LINES);

        // Contains edge: file → import
        result.edges.push(Edge::new(
            parent_id.clone(),
            node_id.clone(),
            EdgeKind::Contains,
        ));

        // Create unresolved reference for the import
        result.unresolved_references.push(UnresolvedReference {
            from_node_id: node_id.clone(),
            reference_name: import_name.clone(),
            reference_kind: EdgeKind::Imports,
            line,
            column: node.start_position().column as u32,
            candidates: vec![],
        });

        result.nodes.push(import_node);
    }

    /// Extract the module/package name from an import statement
    fn extract_import_name(&self, node: TsNode<'_>, source: &str, lang: Language) -> Option<String> {
        match lang {
            Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
                // import ... from "source" → get "source" field
                node.child_by_field_name("source")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .map(|s| s.trim_matches(|c| c == '"' || c == '\'').to_string())
            }
            Language::Rust => {
                // use foo::bar::baz → find the path
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    match child.kind() {
                        "scoped_identifier" | "identifier" | "use_list"
                        | "scoped_use_list" => {
                            return child
                                .utf8_text(source.as_bytes())
                                .ok()
                                .map(|s| s.to_string());
                        }
                        _ => {}
                    }
                }
                None
            }
            Language::Python => {
                // import foo  OR  from foo import bar
                node.child_by_field_name("module_name")
                    .or_else(|| node.child_by_field_name("name"))
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .map(|s| s.to_string())
                    .or_else(|| {
                        // Fallback: find dotted_name child
                        let mut cursor = node.walk();
                        for child in node.children(&mut cursor) {
                            if child.kind() == "dotted_name" {
                                return child
                                    .utf8_text(source.as_bytes())
                                    .ok()
                                    .map(|s| s.to_string());
                            }
                        }
                        None
                    })
            }
            Language::Go => {
                // import "path" → get path field
                node.child_by_field_name("path")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .map(|s| s.trim_matches('"').to_string())
            }
            _ => {
                // Generic: try to get text of the whole import
                node.utf8_text(source.as_bytes())
                    .ok()
                    .map(|s| s.to_string())
            }
        }
    }

    // =========================================================================
    // Call Extraction
    // =========================================================================

    /// Extract function calls from within a function/method subtree
    fn extract_calls_from_subtree(
        &self,
        node: TsNode<'_>,
        source: &str,
        file_path: &str,
        from_node_id: &NodeId,
        lang: Language,
        config: &LanguageConfig,
        call_type: &str,
        result: &mut ExtractionResult,
    ) {
        self.find_calls_recursive(node, source, file_path, from_node_id, lang, config, call_type, result, 0);
    }

    /// Recursively find call expressions, skipping nested function definitions
    fn find_calls_recursive(
        &self,
        node: TsNode<'_>,
        source: &str,
        file_path: &str,
        from_node_id: &NodeId,
        lang: Language,
        config: &LanguageConfig,
        call_type: &str,
        result: &mut ExtractionResult,
        depth: usize,
    ) {
        // Skip nested function definitions to avoid attributing their calls to the parent
        if depth > 0 && config.node_mappings.contains_key(node.kind()) {
            let kind = &config.node_mappings[node.kind()].kind;
            if matches!(kind, NodeKind::Function | NodeKind::Method) {
                return;
            }
        }

        if node.kind() == call_type {
            if let Some(callee_name) = self.extract_callee_name(node, source, config) {
                // Skip built-in symbols
                let base_name = callee_name.split("::").last().unwrap_or(&callee_name);
                let base_name = base_name.split('.').last().unwrap_or(base_name);
                if !BUILTIN_SYMBOLS.contains(&base_name) {
                    let line = node.start_position().row as u32 + 1;
                    result.unresolved_references.push(UnresolvedReference {
                        from_node_id: from_node_id.clone(),
                        reference_name: callee_name,
                        reference_kind: EdgeKind::Calls,
                        line,
                        column: node.start_position().column as u32,
                        candidates: vec![],
                    });
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.find_calls_recursive(
                child, source, file_path, from_node_id, lang, config, call_type, result, depth + 1,
            );
        }
    }

    /// Extract the name of the called function from a call expression
    fn extract_callee_name(
        &self,
        node: TsNode<'_>,
        source: &str,
        config: &LanguageConfig,
    ) -> Option<String> {
        let func_field = config.call_function_field?;
        let func_node = node.child_by_field_name(func_field)?;

        match func_node.kind() {
            // Direct function call: foo()
            "identifier" => func_node
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string()),
            // Method call: obj.method() (JS/TS/Python)
            "member_expression" | "attribute" => {
                // Extract just the method name (rightmost part)
                func_node
                    .child_by_field_name("property")
                    .or_else(|| func_node.child_by_field_name("attribute"))
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .map(|s| s.to_string())
            }
            // Rust: foo::bar() or self.method()
            "scoped_identifier" => func_node
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string()),
            "field_expression" => func_node
                .child_by_field_name("field")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string()),
            // Go: selector expression pkg.Func()
            "selector_expression" => func_node
                .child_by_field_name("field")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string()),
            _ => {
                // Fallback: use the full text
                func_node
                    .utf8_text(source.as_bytes())
                    .ok()
                    .map(|s| s.to_string())
            }
        }
    }

    // =========================================================================
    // Inheritance Extraction
    // =========================================================================

    /// Extract inheritance relationships (extends/implements) from a class/interface/struct
    fn extract_inheritance(
        &self,
        node: TsNode<'_>,
        source: &str,
        _file_path: &str,
        from_node_id: &NodeId,
        lang: Language,
        config: &LanguageConfig,
        result: &mut ExtractionResult,
    ) {
        // Check config-defined inheritance node types
        for (inherit_type, edge_kind) in &config.inheritance_node_types {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == *inherit_type {
                    let type_names = self.extract_type_names(child, source);
                    for type_name in type_names {
                        let line = child.start_position().row as u32 + 1;
                        result.unresolved_references.push(UnresolvedReference {
                            from_node_id: from_node_id.clone(),
                            reference_name: type_name,
                            reference_kind: *edge_kind,
                            line,
                            column: child.start_position().column as u32,
                            candidates: vec![],
                        });
                    }
                }
            }
        }

        // Python special handling: class Foo(Bar, Baz)
        if lang == Language::Python {
            self.extract_python_bases(node, source, from_node_id, result);
        }
    }

    /// Extract type identifiers from an extends/implements clause
    fn extract_type_names(&self, node: TsNode<'_>, source: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "type_identifier" | "identifier" | "generic_type" | "nested_type_identifier" => {
                    // For generic_type, get just the type name (not the type params)
                    if child.kind() == "generic_type" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            if let Ok(text) = name_node.utf8_text(source.as_bytes()) {
                                names.push(text.to_string());
                            }
                        }
                    } else if let Ok(text) = child.utf8_text(source.as_bytes()) {
                        names.push(text.to_string());
                    }
                }
                _ => {
                    // Recurse to find nested type identifiers
                    let nested = self.extract_type_names(child, source);
                    names.extend(nested);
                }
            }
        }
        names
    }

    /// Extract base classes from Python class definition: class Foo(Bar, Baz):
    fn extract_python_bases(
        &self,
        node: TsNode<'_>,
        source: &str,
        from_node_id: &NodeId,
        result: &mut ExtractionResult,
    ) {
        if let Some(superclasses) = node.child_by_field_name("superclasses") {
            let mut cursor = superclasses.walk();
            for child in superclasses.children(&mut cursor) {
                if child.kind() == "identifier" || child.kind() == "attribute" {
                    if let Ok(text) = child.utf8_text(source.as_bytes()) {
                        let name = text.to_string();
                        if !BUILTIN_SYMBOLS.contains(&name.as_str()) {
                            let line = child.start_position().row as u32 + 1;
                            result.unresolved_references.push(UnresolvedReference {
                                from_node_id: from_node_id.clone(),
                                reference_name: name,
                                reference_kind: EdgeKind::Extends,
                                line,
                                column: child.start_position().column as u32,
                                candidates: vec![],
                            });
                        }
                    }
                }
            }
        }
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
