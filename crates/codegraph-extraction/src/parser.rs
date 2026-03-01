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
        if let std::collections::hash_map::Entry::Vacant(e) = self.parsers.entry(lang) {
            let mut parser = Parser::new();
            let ts_lang = get_tree_sitter_language(lang)?;
            parser.set_language(&ts_lang).map_err(|e| {
                ExtractionError::ParseFailed {
                    language: lang.as_str().to_string(),
                    message: format!("Failed to set language: {e}"),
                }
            })?;
            e.insert(parser);
        }
        Ok(self.parsers.get_mut(&lang).unwrap())
    }

    /// Check if a language is supported
    pub fn supports(&self, lang: Language) -> bool {
        get_tree_sitter_language(lang).is_ok()
    }
}

/// Get the tree-sitter language for a CodeGraph language
/// Note: tree-sitter 0.23+ uses LANGUAGE constants instead of language() functions
fn get_tree_sitter_language(lang: Language) -> Result<tree_sitter::Language, ExtractionError> {
    match lang {
        #[cfg(feature = "lang-typescript")]
        Language::TypeScript | Language::Tsx => {
            Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        }
        #[cfg(feature = "lang-typescript")]
        Language::JavaScript | Language::Jsx => {
            Ok(tree_sitter_javascript::LANGUAGE.into())
        }
        #[cfg(feature = "lang-rust")]
        Language::Rust => Ok(tree_sitter_rust::LANGUAGE.into()),
        #[cfg(feature = "lang-python")]
        Language::Python => Ok(tree_sitter_python::LANGUAGE.into()),
        #[cfg(feature = "lang-go")]
        Language::Go => Ok(tree_sitter_go::LANGUAGE.into()),
        #[cfg(feature = "lang-php")]
        Language::Php => Ok(tree_sitter_php::LANGUAGE_PHP.into()),
        #[cfg(feature = "lang-java")]
        Language::Java => Ok(tree_sitter_java::LANGUAGE.into()),
        #[cfg(feature = "lang-c")]
        Language::C => Ok(tree_sitter_c::LANGUAGE.into()),
        #[cfg(feature = "lang-cpp")]
        Language::Cpp => Ok(tree_sitter_cpp::LANGUAGE.into()),
        #[cfg(feature = "lang-csharp")]
        Language::CSharp => Ok(tree_sitter_c_sharp::LANGUAGE.into()),
        #[cfg(feature = "lang-ruby")]
        Language::Ruby => Ok(tree_sitter_ruby::LANGUAGE.into()),
        #[cfg(feature = "lang-bash")]
        Language::Bash => Ok(tree_sitter_bash::LANGUAGE.into()),
        #[cfg(feature = "lang-dart")]
        Language::Dart => Ok(tree_sitter_dart::LANGUAGE.into()),
        #[cfg(feature = "lang-swift")]
        Language::Swift => Ok(tree_sitter_swift::LANGUAGE.into()),
        #[cfg(feature = "lang-graphql")]
        Language::GraphQL => Ok(tree_sitter_graphql::LANGUAGE.into()),
        #[cfg(feature = "lang-hcl")]
        Language::Hcl => Ok(tree_sitter_hcl::LANGUAGE.into()),
        // Note: Kotlin blocked by tree-sitter version constraint
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

    #[test]
    #[cfg(feature = "lang-dart")]
    fn test_parse_dart() {
        let mut parser = TreeSitterParser::new();
        let source = r#"
            class MyWidget extends StatelessWidget {
                @override
                Widget build(BuildContext context) {
                    return Container();
                }
            }
        "#;
        let tree = parser.parse(source, Language::Dart).unwrap();
        assert!(!tree.root_node().has_error());
    }
}
