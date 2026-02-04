//! Language-specific configuration for tree-sitter extraction
//!
//! This module provides language configurations that map tree-sitter AST node
//! types to CodeGraph node kinds. The actual extraction is handled by a generic
//! TreeSitterExtractor that uses these configurations.

use codegraph_types::{Language, NodeKind};
use std::collections::HashMap;

/// Configuration for extracting from a specific language
#[derive(Debug, Clone)]
pub struct LanguageConfig {
    /// The language this config applies to
    pub language: Language,

    /// Map of tree-sitter node types to CodeGraph node kinds
    pub node_mappings: HashMap<&'static str, NodeMapping>,

    /// Tree-sitter node type for decorators/attributes (e.g., "decorator", "attribute_item")
    pub decorator_node_type: Option<&'static str>,

    /// How to extract the name from a node (field name to use)
    pub name_field: &'static str,

    /// Tree-sitter node type for the body of a class/struct (to find methods)
    pub body_field: Option<&'static str>,

    /// Node types that indicate "exported" or "public" visibility
    pub export_indicators: Vec<&'static str>,
}

/// Mapping from tree-sitter node type to CodeGraph node kind
#[derive(Debug, Clone)]
pub struct NodeMapping {
    pub kind: NodeKind,
    /// If true, this node can contain children (like methods in a class)
    pub has_children: bool,
    /// Child node types to look for within this node
    pub child_types: Vec<&'static str>,
}

impl NodeMapping {
    pub fn simple(kind: NodeKind) -> Self {
        Self {
            kind,
            has_children: false,
            child_types: vec![],
        }
    }

    pub fn with_children(kind: NodeKind, child_types: Vec<&'static str>) -> Self {
        Self {
            kind,
            has_children: true,
            child_types,
        }
    }
}

/// Get the configuration for a language
pub fn get_language_config(lang: Language) -> Option<LanguageConfig> {
    match lang {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            Some(typescript_config())
        }
        Language::Rust => Some(rust_config()),
        Language::Python => Some(python_config()),
        Language::Go => Some(go_config()),
        Language::Php => Some(php_config()),
        Language::Java => Some(java_config()),
        Language::CSharp => Some(csharp_config()),
        Language::Ruby => Some(ruby_config()),
        // Note: Swift, Kotlin, Dart disabled due to tree-sitter version conflicts
        _ => None,
    }
}

fn typescript_config() -> LanguageConfig {
    let mut mappings = HashMap::new();

    mappings.insert(
        "class_declaration",
        NodeMapping::with_children(NodeKind::Class, vec!["method_definition", "public_field_definition"]),
    );
    mappings.insert(
        "abstract_class_declaration",
        NodeMapping::with_children(NodeKind::Class, vec!["method_definition", "public_field_definition"]),
    );
    mappings.insert("function_declaration", NodeMapping::simple(NodeKind::Function));
    mappings.insert("arrow_function", NodeMapping::simple(NodeKind::Function));
    mappings.insert("interface_declaration", NodeMapping::simple(NodeKind::Interface));
    mappings.insert("type_alias_declaration", NodeMapping::simple(NodeKind::TypeAlias));
    mappings.insert("enum_declaration", NodeMapping::simple(NodeKind::Enum));
    mappings.insert("method_definition", NodeMapping::simple(NodeKind::Method));
    mappings.insert("public_field_definition", NodeMapping::simple(NodeKind::Property));

    LanguageConfig {
        language: Language::TypeScript,
        node_mappings: mappings,
        decorator_node_type: Some("decorator"),
        name_field: "name",
        body_field: Some("body"),
        export_indicators: vec!["export_statement"],
    }
}

fn rust_config() -> LanguageConfig {
    let mut mappings = HashMap::new();

    mappings.insert("function_item", NodeMapping::simple(NodeKind::Function));
    mappings.insert(
        "struct_item",
        NodeMapping::with_children(NodeKind::Struct, vec!["field_declaration"]),
    );
    mappings.insert("enum_item", NodeMapping::simple(NodeKind::Enum));
    mappings.insert("trait_item", NodeMapping::simple(NodeKind::Trait));
    mappings.insert("impl_item", NodeMapping::with_children(NodeKind::Module, vec!["function_item"]));
    mappings.insert("mod_item", NodeMapping::simple(NodeKind::Module));
    mappings.insert("field_declaration", NodeMapping::simple(NodeKind::Field));

    LanguageConfig {
        language: Language::Rust,
        node_mappings: mappings,
        decorator_node_type: Some("attribute_item"),
        name_field: "name",
        body_field: Some("body"),
        export_indicators: vec!["visibility_modifier"],
    }
}

fn python_config() -> LanguageConfig {
    let mut mappings = HashMap::new();

    mappings.insert(
        "class_definition",
        NodeMapping::with_children(NodeKind::Class, vec!["function_definition"]),
    );
    mappings.insert("function_definition", NodeMapping::simple(NodeKind::Function));

    LanguageConfig {
        language: Language::Python,
        node_mappings: mappings,
        decorator_node_type: Some("decorator"),
        name_field: "name",
        body_field: Some("body"),
        export_indicators: vec![],
    }
}

fn go_config() -> LanguageConfig {
    let mut mappings = HashMap::new();

    mappings.insert("function_declaration", NodeMapping::simple(NodeKind::Function));
    mappings.insert("method_declaration", NodeMapping::simple(NodeKind::Method));
    mappings.insert("type_spec", NodeMapping::simple(NodeKind::Struct)); // Will be refined based on content

    LanguageConfig {
        language: Language::Go,
        node_mappings: mappings,
        decorator_node_type: None, // Go doesn't have decorators
        name_field: "name",
        body_field: None,
        export_indicators: vec![], // Go uses capitalization
    }
}

fn php_config() -> LanguageConfig {
    let mut mappings = HashMap::new();

    mappings.insert(
        "class_declaration",
        NodeMapping::with_children(NodeKind::Class, vec!["method_declaration", "property_declaration"]),
    );
    mappings.insert("function_definition", NodeMapping::simple(NodeKind::Function));
    mappings.insert("method_declaration", NodeMapping::simple(NodeKind::Method));
    mappings.insert("property_declaration", NodeMapping::simple(NodeKind::Property));
    mappings.insert("interface_declaration", NodeMapping::simple(NodeKind::Interface));
    mappings.insert("trait_declaration", NodeMapping::simple(NodeKind::Trait));

    LanguageConfig {
        language: Language::Php,
        node_mappings: mappings,
        decorator_node_type: Some("attribute_list"),
        name_field: "name",
        body_field: Some("body"),
        export_indicators: vec!["visibility_modifier"],
    }
}

fn java_config() -> LanguageConfig {
    let mut mappings = HashMap::new();

    mappings.insert(
        "class_declaration",
        NodeMapping::with_children(NodeKind::Class, vec!["method_declaration", "field_declaration"]),
    );
    mappings.insert(
        "interface_declaration",
        NodeMapping::with_children(NodeKind::Interface, vec!["method_declaration"]),
    );
    mappings.insert("method_declaration", NodeMapping::simple(NodeKind::Method));
    mappings.insert("field_declaration", NodeMapping::simple(NodeKind::Field));
    mappings.insert("enum_declaration", NodeMapping::simple(NodeKind::Enum));

    LanguageConfig {
        language: Language::Java,
        node_mappings: mappings,
        decorator_node_type: Some("annotation"),
        name_field: "name",
        body_field: Some("body"),
        export_indicators: vec!["public"],
    }
}

fn csharp_config() -> LanguageConfig {
    let mut mappings = HashMap::new();

    mappings.insert(
        "class_declaration",
        NodeMapping::with_children(NodeKind::Class, vec!["method_declaration", "field_declaration", "property_declaration"]),
    );
    mappings.insert("interface_declaration", NodeMapping::simple(NodeKind::Interface));
    mappings.insert("method_declaration", NodeMapping::simple(NodeKind::Method));
    mappings.insert("field_declaration", NodeMapping::simple(NodeKind::Field));
    mappings.insert("property_declaration", NodeMapping::simple(NodeKind::Property));
    mappings.insert("struct_declaration", NodeMapping::simple(NodeKind::Struct));
    mappings.insert("enum_declaration", NodeMapping::simple(NodeKind::Enum));

    LanguageConfig {
        language: Language::CSharp,
        node_mappings: mappings,
        decorator_node_type: Some("attribute_list"),
        name_field: "name",
        body_field: Some("body"),
        export_indicators: vec!["public"],
    }
}

fn ruby_config() -> LanguageConfig {
    let mut mappings = HashMap::new();

    mappings.insert(
        "class",
        NodeMapping::with_children(NodeKind::Class, vec!["method"]),
    );
    mappings.insert(
        "module",
        NodeMapping::with_children(NodeKind::Module, vec!["method"]),
    );
    mappings.insert("method", NodeMapping::simple(NodeKind::Method));
    mappings.insert("singleton_method", NodeMapping::simple(NodeKind::Method));

    LanguageConfig {
        language: Language::Ruby,
        node_mappings: mappings,
        decorator_node_type: None, // Ruby uses method calls for decorations
        name_field: "name",
        body_field: Some("body"),
        export_indicators: vec![],
    }
}

// Note: Swift, Kotlin, Dart configs removed due to tree-sitter version conflicts
// They can be re-added when compatible tree-sitter grammars are available

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typescript_config() {
        let config = get_language_config(Language::TypeScript).unwrap();
        assert!(config.node_mappings.contains_key("class_declaration"));
        assert!(config.node_mappings.contains_key("function_declaration"));
        assert_eq!(config.decorator_node_type, Some("decorator"));
    }

    #[test]
    fn test_rust_config() {
        let config = get_language_config(Language::Rust).unwrap();
        assert!(config.node_mappings.contains_key("function_item"));
        assert!(config.node_mappings.contains_key("struct_item"));
        assert_eq!(config.decorator_node_type, Some("attribute_item"));
    }
}
