//! Tree-sitter parser wrapper for multi-language support

use crate::error::ExtractionError;
use codegraph_types::Language;
use std::collections::HashMap;
use tree_sitter::{Parser, Tree};

/// Tree-sitter parser manager that handles multiple languages
pub struct TreeSitterParser {
    parsers: HashMap<Language, Parser>,
}

impl Default for TreeSitterParser {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeSitterParser {
    /// Create a new parser manager
    pub fn new() -> Self {
        Self {
            parsers: HashMap::new(),
        }
    }

    /// Parse source code for a given language
    pub fn parse(&mut self, source: &str, lang: Language) -> Result<Tree, ExtractionError> {
        let parser = self.get_or_create_parser(lang)?;
        parser
            .parse(source, None)
            .ok_or_else(|| ExtractionError::ParseFailed {
                language: lang.as_str().to_string(),
                message: "Parser returned None".to_string(),
            })
    }

    /// Get or create a parser for a language
    fn get_or_create_parser(&mut self, lang: Language) -> Result<&mut Parser, ExtractionError> {
        if !self.parsers.contains_key(&lang) {
            let mut parser = Parser::new();
            let ts_lang = get_tree_sitter_language(lang)?;
            parser.set_language(ts_lang).map_err(|e| {
                ExtractionError::ParseFailed {
                    language: lang.as_str().to_string(),
                    message: format!("Failed to set language: {e}"),
                }
            })?;
            self.parsers.insert(lang, parser);
        }
        Ok(self.parsers.get_mut(&lang).unwrap())
    }

    /// Check if a language is supported
    pub fn supports(&self, lang: Language) -> bool {
        get_tree_sitter_language(lang).is_ok()
    }
}

/// Get the tree-sitter language for a CodeGraph language
/// Note: tree-sitter 0.20 uses function-based API instead of constants
fn get_tree_sitter_language(lang: Language) -> Result<tree_sitter::Language, ExtractionError> {
    match lang {
        #[cfg(feature = "lang-typescript")]
        Language::TypeScript | Language::Tsx => {
            Ok(tree_sitter_typescript::language_typescript())
        }
        #[cfg(feature = "lang-typescript")]
        Language::JavaScript | Language::Jsx => {
            Ok(tree_sitter_javascript::language())
        }
        #[cfg(feature = "lang-rust")]
        Language::Rust => Ok(tree_sitter_rust::language()),
        #[cfg(feature = "lang-python")]
        Language::Python => Ok(tree_sitter_python::language()),
        #[cfg(feature = "lang-go")]
        Language::Go => Ok(tree_sitter_go::language()),
        #[cfg(feature = "lang-php")]
        Language::Php => Ok(tree_sitter_php::language_php()),
        #[cfg(feature = "lang-java")]
        Language::Java => Ok(tree_sitter_java::language()),
        #[cfg(feature = "lang-c")]
        Language::C => Ok(tree_sitter_c::language()),
        #[cfg(feature = "lang-cpp")]
        Language::Cpp => Ok(tree_sitter_cpp::language()),
        #[cfg(feature = "lang-csharp")]
        Language::CSharp => Ok(tree_sitter_c_sharp::language()),
        #[cfg(feature = "lang-ruby")]
        Language::Ruby => Ok(tree_sitter_ruby::language()),
        #[cfg(feature = "lang-bash")]
        Language::Bash => Ok(tree_sitter_bash::language()),
        // Note: Swift, Kotlin, Dart, GraphQL, HCL disabled due to tree-sitter version conflicts
        _ => Err(ExtractionError::UnsupportedLanguage(
            lang.as_str().to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "lang-rust")]
    fn test_parse_rust() {
        let mut parser = TreeSitterParser::new();
        let source = r#"
            fn main() {
                println!("Hello, world!");
            }
        "#;
        let tree = parser.parse(source, Language::Rust).unwrap();
        assert!(!tree.root_node().has_error());
    }

    #[test]
    #[cfg(feature = "lang-typescript")]
    fn test_parse_typescript() {
        let mut parser = TreeSitterParser::new();
        let source = r#"
            function greet(name: string): string {
                return `Hello, ${name}!`;
            }
        "#;
        let tree = parser.parse(source, Language::TypeScript).unwrap();
        assert!(!tree.root_node().has_error());
    }

    #[test]
    #[cfg(feature = "lang-typescript")]
    fn test_parse_typescript_decorator() {
        let mut parser = TreeSitterParser::new();
        let source = r#"
            @Injectable()
            class MyService {
                constructor() {}
            }
        "#;
        let tree = parser.parse(source, Language::TypeScript).unwrap();
        assert!(!tree.root_node().has_error());
    }
}
