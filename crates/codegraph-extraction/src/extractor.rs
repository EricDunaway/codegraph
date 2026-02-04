//! Language extractor trait and registry

use crate::error::ExtractionError;
use codegraph_types::{
    Edge, EdgeKind, ExtractionResult, Language, Node, NodeId, NodeKind, Visibility,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Trait for language-specific extractors
pub trait LanguageExtractor: Send + Sync {
    /// Get the language this extractor handles
    fn language(&self) -> Language;

    /// Extract nodes and edges from source code
    fn extract(&self, source: &str, file_path: &str) -> Result<ExtractionResult, ExtractionError>;

    /// Check if this extractor supports the given language
    fn supports(&self, lang: Language) -> bool {
        self.language() == lang
    }
}

/// Registry of language extractors
pub struct ExtractorRegistry {
    extractors: HashMap<Language, Box<dyn LanguageExtractor>>,
}

impl Default for ExtractorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtractorRegistry {
    /// Create a new registry with default extractors
    pub fn new() -> Self {
        let mut registry = Self {
            extractors: HashMap::new(),
        };

        // Register default extractors (simple regex-based for now)
        registry.register(Box::new(SimpleRustExtractor));
        registry.register(Box::new(SimpleTypeScriptExtractor));
        registry.register(Box::new(SimplePythonExtractor));
        registry.register(Box::new(SimpleGoExtractor));

        registry
    }

    /// Register an extractor
    pub fn register(&mut self, extractor: Box<dyn LanguageExtractor>) {
        self.extractors.insert(extractor.language(), extractor);
    }

    /// Get an extractor for a language
    pub fn get(&self, lang: Language) -> Option<&dyn LanguageExtractor> {
        self.extractors.get(&lang).map(|e| e.as_ref())
    }

    /// Check if a language is supported
    pub fn supports(&self, lang: Language) -> bool {
        self.extractors.contains_key(&lang)
    }

    /// Extract from source using appropriate extractor
    pub fn extract(
        &self,
        source: &str,
        file_path: &str,
        lang: Language,
    ) -> Result<ExtractionResult, ExtractionError> {
        match self.get(lang) {
            Some(extractor) => extractor.extract(source, file_path),
            None => Err(ExtractionError::UnsupportedLanguage(lang.as_str().to_string())),
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

// =============================================================================
// Simple Extractors (regex-based, will be replaced with tree-sitter)
// =============================================================================

/// Simple Rust extractor using regex patterns
struct SimpleRustExtractor;

impl LanguageExtractor for SimpleRustExtractor {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn extract(&self, source: &str, file_path: &str) -> Result<ExtractionResult, ExtractionError> {
        let start = std::time::Instant::now();
        let mut result = ExtractionResult::default();

        // Create file node
        let file_id = generate_node_id(file_path, NodeKind::File, file_path, 0);
        let file_node = Node::new(
            file_id.clone(),
            NodeKind::File,
            file_path,
            file_path,
            file_path,
            Language::Rust,
            1,
            source.lines().count() as u32,
        );
        result.nodes.push(file_node);

        // Extract functions: fn name(...)
        let fn_regex = regex_lite::Regex::new(
            r"(?m)^[ \t]*(pub(?:\s*\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)"
        ).unwrap();

        for cap in fn_regex.captures_iter(source) {
            let full_match = cap.get(0).unwrap();
            let line = source[..full_match.start()].lines().count() as u32 + 1;
            let name = cap.get(2).unwrap().as_str();
            let is_pub = cap.get(1).map(|m| m.as_str().contains("pub")).unwrap_or(false);

            let node_id = generate_node_id(file_path, NodeKind::Function, name, line);
            let mut node = Node::new(
                node_id.clone(),
                NodeKind::Function,
                name,
                format!("{file_path}::{name}"),
                file_path,
                Language::Rust,
                line,
                line + 10, // Approximate end line
            );
            node.visibility = if is_pub { Some(Visibility::Public) } else { Some(Visibility::Private) };
            node.is_exported = is_pub;

            // Add contains edge from file to function
            result.edges.push(Edge::new(file_id.clone(), node_id.clone(), EdgeKind::Contains));
            result.nodes.push(node);
        }

        // Extract structs: struct Name
        let struct_regex = regex_lite::Regex::new(
            r"(?m)^[ \t]*(pub(?:\s*\([^)]*\))?\s+)?struct\s+(\w+)"
        ).unwrap();

        for cap in struct_regex.captures_iter(source) {
            let full_match = cap.get(0).unwrap();
            let line = source[..full_match.start()].lines().count() as u32 + 1;
            let name = cap.get(2).unwrap().as_str();
            let is_pub = cap.get(1).map(|m| m.as_str().contains("pub")).unwrap_or(false);

            let node_id = generate_node_id(file_path, NodeKind::Struct, name, line);
            let mut node = Node::new(
                node_id.clone(),
                NodeKind::Struct,
                name,
                format!("{file_path}::{name}"),
                file_path,
                Language::Rust,
                line,
                line + 5,
            );
            node.visibility = if is_pub { Some(Visibility::Public) } else { Some(Visibility::Private) };
            node.is_exported = is_pub;

            result.edges.push(Edge::new(file_id.clone(), node_id.clone(), EdgeKind::Contains));
            result.nodes.push(node);
        }

        // Extract enums
        let enum_regex = regex_lite::Regex::new(
            r"(?m)^[ \t]*(pub(?:\s*\([^)]*\))?\s+)?enum\s+(\w+)"
        ).unwrap();

        for cap in enum_regex.captures_iter(source) {
            let full_match = cap.get(0).unwrap();
            let line = source[..full_match.start()].lines().count() as u32 + 1;
            let name = cap.get(2).unwrap().as_str();
            let is_pub = cap.get(1).map(|m| m.as_str().contains("pub")).unwrap_or(false);

            let node_id = generate_node_id(file_path, NodeKind::Enum, name, line);
            let mut node = Node::new(
                node_id.clone(),
                NodeKind::Enum,
                name,
                format!("{file_path}::{name}"),
                file_path,
                Language::Rust,
                line,
                line + 5,
            );
            node.visibility = if is_pub { Some(Visibility::Public) } else { Some(Visibility::Private) };
            node.is_exported = is_pub;

            result.edges.push(Edge::new(file_id.clone(), node_id.clone(), EdgeKind::Contains));
            result.nodes.push(node);
        }

        // Extract traits
        let trait_regex = regex_lite::Regex::new(
            r"(?m)^[ \t]*(pub(?:\s*\([^)]*\))?\s+)?trait\s+(\w+)"
        ).unwrap();

        for cap in trait_regex.captures_iter(source) {
            let full_match = cap.get(0).unwrap();
            let line = source[..full_match.start()].lines().count() as u32 + 1;
            let name = cap.get(2).unwrap().as_str();
            let is_pub = cap.get(1).map(|m| m.as_str().contains("pub")).unwrap_or(false);

            let node_id = generate_node_id(file_path, NodeKind::Trait, name, line);
            let mut node = Node::new(
                node_id.clone(),
                NodeKind::Trait,
                name,
                format!("{file_path}::{name}"),
                file_path,
                Language::Rust,
                line,
                line + 5,
            );
            node.visibility = if is_pub { Some(Visibility::Public) } else { Some(Visibility::Private) };
            node.is_exported = is_pub;

            result.edges.push(Edge::new(file_id.clone(), node_id.clone(), EdgeKind::Contains));
            result.nodes.push(node);
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        Ok(result)
    }
}

/// Simple TypeScript/JavaScript extractor
struct SimpleTypeScriptExtractor;

impl LanguageExtractor for SimpleTypeScriptExtractor {
    fn language(&self) -> Language {
        Language::TypeScript
    }

    fn supports(&self, lang: Language) -> bool {
        matches!(lang, Language::TypeScript | Language::JavaScript | Language::Tsx | Language::Jsx)
    }

    fn extract(&self, source: &str, file_path: &str) -> Result<ExtractionResult, ExtractionError> {
        let start = std::time::Instant::now();
        let mut result = ExtractionResult::default();

        let lang = Language::from_extension(
            std::path::Path::new(file_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("ts"),
        );

        // Create file node
        let file_id = generate_node_id(file_path, NodeKind::File, file_path, 0);
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

        // Extract functions
        let fn_regex = regex_lite::Regex::new(
            r"(?m)^[ \t]*(export\s+)?(async\s+)?function\s+(\w+)"
        ).unwrap();

        for cap in fn_regex.captures_iter(source) {
            let full_match = cap.get(0).unwrap();
            let line = source[..full_match.start()].lines().count() as u32 + 1;
            let name = cap.get(3).unwrap().as_str();
            let is_exported = cap.get(1).is_some();
            let is_async = cap.get(2).is_some();

            let node_id = generate_node_id(file_path, NodeKind::Function, name, line);
            let mut node = Node::new(
                node_id.clone(),
                NodeKind::Function,
                name,
                format!("{file_path}::{name}"),
                file_path,
                lang,
                line,
                line + 10,
            );
            node.is_exported = is_exported;
            node.is_async = is_async;

            result.edges.push(Edge::new(file_id.clone(), node_id.clone(), EdgeKind::Contains));
            result.nodes.push(node);
        }

        // Extract classes
        let class_regex = regex_lite::Regex::new(
            r"(?m)^[ \t]*(export\s+)?class\s+(\w+)"
        ).unwrap();

        for cap in class_regex.captures_iter(source) {
            let full_match = cap.get(0).unwrap();
            let line = source[..full_match.start()].lines().count() as u32 + 1;
            let name = cap.get(2).unwrap().as_str();
            let is_exported = cap.get(1).is_some();

            let node_id = generate_node_id(file_path, NodeKind::Class, name, line);
            let mut node = Node::new(
                node_id.clone(),
                NodeKind::Class,
                name,
                format!("{file_path}::{name}"),
                file_path,
                lang,
                line,
                line + 20,
            );
            node.is_exported = is_exported;

            result.edges.push(Edge::new(file_id.clone(), node_id.clone(), EdgeKind::Contains));
            result.nodes.push(node);
        }

        // Extract interfaces
        let interface_regex = regex_lite::Regex::new(
            r"(?m)^[ \t]*(export\s+)?interface\s+(\w+)"
        ).unwrap();

        for cap in interface_regex.captures_iter(source) {
            let full_match = cap.get(0).unwrap();
            let line = source[..full_match.start()].lines().count() as u32 + 1;
            let name = cap.get(2).unwrap().as_str();
            let is_exported = cap.get(1).is_some();

            let node_id = generate_node_id(file_path, NodeKind::Interface, name, line);
            let mut node = Node::new(
                node_id.clone(),
                NodeKind::Interface,
                name,
                format!("{file_path}::{name}"),
                file_path,
                lang,
                line,
                line + 10,
            );
            node.is_exported = is_exported;

            result.edges.push(Edge::new(file_id.clone(), node_id.clone(), EdgeKind::Contains));
            result.nodes.push(node);
        }

        // Extract type aliases
        let type_regex = regex_lite::Regex::new(
            r"(?m)^[ \t]*(export\s+)?type\s+(\w+)"
        ).unwrap();

        for cap in type_regex.captures_iter(source) {
            let full_match = cap.get(0).unwrap();
            let line = source[..full_match.start()].lines().count() as u32 + 1;
            let name = cap.get(2).unwrap().as_str();
            let is_exported = cap.get(1).is_some();

            let node_id = generate_node_id(file_path, NodeKind::TypeAlias, name, line);
            let mut node = Node::new(
                node_id.clone(),
                NodeKind::TypeAlias,
                name,
                format!("{file_path}::{name}"),
                file_path,
                lang,
                line,
                line + 1,
            );
            node.is_exported = is_exported;

            result.edges.push(Edge::new(file_id.clone(), node_id.clone(), EdgeKind::Contains));
            result.nodes.push(node);
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        Ok(result)
    }
}

/// Simple Python extractor
struct SimplePythonExtractor;

impl LanguageExtractor for SimplePythonExtractor {
    fn language(&self) -> Language {
        Language::Python
    }

    fn extract(&self, source: &str, file_path: &str) -> Result<ExtractionResult, ExtractionError> {
        let start = std::time::Instant::now();
        let mut result = ExtractionResult::default();

        // Create file node
        let file_id = generate_node_id(file_path, NodeKind::File, file_path, 0);
        let file_node = Node::new(
            file_id.clone(),
            NodeKind::File,
            file_path,
            file_path,
            file_path,
            Language::Python,
            1,
            source.lines().count() as u32,
        );
        result.nodes.push(file_node);

        // Extract functions/methods
        let fn_regex = regex_lite::Regex::new(
            r"(?m)^[ \t]*(async\s+)?def\s+(\w+)"
        ).unwrap();

        for cap in fn_regex.captures_iter(source) {
            let full_match = cap.get(0).unwrap();
            let line = source[..full_match.start()].lines().count() as u32 + 1;
            let name = cap.get(2).unwrap().as_str();
            let is_async = cap.get(1).is_some();

            let node_id = generate_node_id(file_path, NodeKind::Function, name, line);
            let mut node = Node::new(
                node_id.clone(),
                NodeKind::Function,
                name,
                format!("{file_path}::{name}"),
                file_path,
                Language::Python,
                line,
                line + 10,
            );
            node.is_async = is_async;
            // Python uses leading underscore for private
            node.visibility = if name.starts_with('_') && !name.starts_with("__") {
                Some(Visibility::Private)
            } else {
                Some(Visibility::Public)
            };

            result.edges.push(Edge::new(file_id.clone(), node_id.clone(), EdgeKind::Contains));
            result.nodes.push(node);
        }

        // Extract classes
        let class_regex = regex_lite::Regex::new(
            r"(?m)^class\s+(\w+)"
        ).unwrap();

        for cap in class_regex.captures_iter(source) {
            let full_match = cap.get(0).unwrap();
            let line = source[..full_match.start()].lines().count() as u32 + 1;
            let name = cap.get(1).unwrap().as_str();

            let node_id = generate_node_id(file_path, NodeKind::Class, name, line);
            let node = Node::new(
                node_id.clone(),
                NodeKind::Class,
                name,
                format!("{file_path}::{name}"),
                file_path,
                Language::Python,
                line,
                line + 20,
            );

            result.edges.push(Edge::new(file_id.clone(), node_id.clone(), EdgeKind::Contains));
            result.nodes.push(node);
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        Ok(result)
    }
}

/// Simple Go extractor
struct SimpleGoExtractor;

impl LanguageExtractor for SimpleGoExtractor {
    fn language(&self) -> Language {
        Language::Go
    }

    fn extract(&self, source: &str, file_path: &str) -> Result<ExtractionResult, ExtractionError> {
        let start = std::time::Instant::now();
        let mut result = ExtractionResult::default();

        // Create file node
        let file_id = generate_node_id(file_path, NodeKind::File, file_path, 0);
        let file_node = Node::new(
            file_id.clone(),
            NodeKind::File,
            file_path,
            file_path,
            file_path,
            Language::Go,
            1,
            source.lines().count() as u32,
        );
        result.nodes.push(file_node);

        // Extract functions
        let fn_regex = regex_lite::Regex::new(
            r"(?m)^func\s+(\w+)"
        ).unwrap();

        for cap in fn_regex.captures_iter(source) {
            let full_match = cap.get(0).unwrap();
            let line = source[..full_match.start()].lines().count() as u32 + 1;
            let name = cap.get(1).unwrap().as_str();

            let node_id = generate_node_id(file_path, NodeKind::Function, name, line);
            let mut node = Node::new(
                node_id.clone(),
                NodeKind::Function,
                name,
                format!("{file_path}::{name}"),
                file_path,
                Language::Go,
                line,
                line + 10,
            );
            // Go uses capital letter for exported
            node.is_exported = name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
            node.visibility = if node.is_exported {
                Some(Visibility::Public)
            } else {
                Some(Visibility::Private)
            };

            result.edges.push(Edge::new(file_id.clone(), node_id.clone(), EdgeKind::Contains));
            result.nodes.push(node);
        }

        // Extract structs
        let struct_regex = regex_lite::Regex::new(
            r"(?m)^type\s+(\w+)\s+struct"
        ).unwrap();

        for cap in struct_regex.captures_iter(source) {
            let full_match = cap.get(0).unwrap();
            let line = source[..full_match.start()].lines().count() as u32 + 1;
            let name = cap.get(1).unwrap().as_str();

            let node_id = generate_node_id(file_path, NodeKind::Struct, name, line);
            let mut node = Node::new(
                node_id.clone(),
                NodeKind::Struct,
                name,
                format!("{file_path}::{name}"),
                file_path,
                Language::Go,
                line,
                line + 10,
            );
            node.is_exported = name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);

            result.edges.push(Edge::new(file_id.clone(), node_id.clone(), EdgeKind::Contains));
            result.nodes.push(node);
        }

        // Extract interfaces
        let interface_regex = regex_lite::Regex::new(
            r"(?m)^type\s+(\w+)\s+interface"
        ).unwrap();

        for cap in interface_regex.captures_iter(source) {
            let full_match = cap.get(0).unwrap();
            let line = source[..full_match.start()].lines().count() as u32 + 1;
            let name = cap.get(1).unwrap().as_str();

            let node_id = generate_node_id(file_path, NodeKind::Interface, name, line);
            let mut node = Node::new(
                node_id.clone(),
                NodeKind::Interface,
                name,
                format!("{file_path}::{name}"),
                file_path,
                Language::Go,
                line,
                line + 10,
            );
            node.is_exported = name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);

            result.edges.push(Edge::new(file_id.clone(), node_id.clone(), EdgeKind::Contains));
            result.nodes.push(node);
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_extractor() {
        let source = r#"
pub fn hello_world() {
    println!("Hello!");
}

fn private_func() {}

pub struct MyStruct {
    field: i32,
}

enum MyEnum {
    A,
    B,
}

pub trait MyTrait {
    fn method(&self);
}
"#;

        let extractor = SimpleRustExtractor;
        let result = extractor.extract(source, "test.rs").unwrap();

        // File + 3 functions (hello_world, private_func, method in trait) + 1 struct + 1 enum + 1 trait = 7 nodes
        // Note: Simple regex extractor doesn't have context awareness, so trait methods are captured too
        assert_eq!(result.nodes.len(), 7);

        let funcs: Vec<_> = result.nodes.iter().filter(|n| n.kind == NodeKind::Function).collect();
        assert_eq!(funcs.len(), 3);

        let hello = funcs.iter().find(|n| n.name == "hello_world").unwrap();
        assert!(hello.is_exported);

        let private = funcs.iter().find(|n| n.name == "private_func").unwrap();
        assert!(!private.is_exported);
    }

    #[test]
    fn test_typescript_extractor() {
        let source = r#"
export function greet(name: string): string {
    return `Hello, ${name}!`;
}

export class Greeter {
    name: string;
}

export interface IGreeter {
    greet(): string;
}

export type GreetFn = (name: string) => string;
"#;

        let extractor = SimpleTypeScriptExtractor;
        let result = extractor.extract(source, "test.ts").unwrap();

        // File + 1 function + 1 class + 1 interface + 1 type = 5 nodes
        assert_eq!(result.nodes.len(), 5);

        let func = result.nodes.iter().find(|n| n.name == "greet").unwrap();
        assert!(func.is_exported);
        assert_eq!(func.kind, NodeKind::Function);
    }

    #[test]
    fn test_python_extractor() {
        let source = r#"
def public_function():
    pass

def _private_function():
    pass

async def async_function():
    pass

class MyClass:
    def method(self):
        pass
"#;

        let extractor = SimplePythonExtractor;
        let result = extractor.extract(source, "test.py").unwrap();

        // File + 4 functions + 1 class = 6 nodes
        assert_eq!(result.nodes.len(), 6);

        let async_fn = result.nodes.iter().find(|n| n.name == "async_function").unwrap();
        assert!(async_fn.is_async);

        let private = result.nodes.iter().find(|n| n.name == "_private_function").unwrap();
        assert_eq!(private.visibility, Some(Visibility::Private));
    }

    #[test]
    fn test_go_extractor() {
        let source = r#"
package main

func main() {
    fmt.Println("Hello")
}

func privateFunc() {}

type MyStruct struct {
    Field int
}

type myInterface interface {
    Method()
}
"#;

        let extractor = SimpleGoExtractor;
        let result = extractor.extract(source, "main.go").unwrap();

        // File + 2 functions + 1 struct + 1 interface = 5 nodes
        assert_eq!(result.nodes.len(), 5);

        let main_fn = result.nodes.iter().find(|n| n.name == "main").unwrap();
        assert!(!main_fn.is_exported); // lowercase = private in Go

        let my_struct = result.nodes.iter().find(|n| n.name == "MyStruct").unwrap();
        assert!(my_struct.is_exported); // Uppercase = exported in Go
    }

    #[test]
    fn test_node_id_generation() {
        let id1 = generate_node_id("test.rs", NodeKind::Function, "foo", 10);
        let id2 = generate_node_id("test.rs", NodeKind::Function, "foo", 10);
        let id3 = generate_node_id("test.rs", NodeKind::Function, "bar", 10);

        assert_eq!(id1, id2); // Same inputs = same ID
        assert_ne!(id1, id3); // Different name = different ID
        assert!(id1.as_str().starts_with("function:"));
    }
}
