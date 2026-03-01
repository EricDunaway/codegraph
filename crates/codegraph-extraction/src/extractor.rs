//! Language extractor trait and registry
//!
//! This module provides the extraction interface. The actual extraction logic
//! is handled by the TreeSitterExtractor which uses language configurations
//! rather than language-specific extractor implementations.

use crate::error::ExtractionError;
use crate::languages::get_language_config;
use crate::tree_sitter_extractor::TreeSitterExtractor;
use codegraph_types::{ExtractionResult, Language, NodeId, NodeKind};
use sha2::{Digest, Sha256};
use std::sync::Mutex;

/// Trait for language-specific extractors (kept for API compatibility)
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

/// Registry that manages extraction for all supported languages
///
/// Uses a single TreeSitterExtractor with language configurations
/// rather than separate extractors per language.
pub struct ExtractorRegistry {
    extractor: Mutex<TreeSitterExtractor>,
}

impl Default for ExtractorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtractorRegistry {
    /// Create a new registry
    pub fn new() -> Self {
        Self {
            extractor: Mutex::new(TreeSitterExtractor::new()),
        }
    }

    /// Check if a language is supported
    pub fn supports(&self, lang: Language) -> bool {
        get_language_config(lang).is_some()
    }

    /// Get supported languages
    pub fn supported_languages(&self) -> Vec<Language> {
        vec![
            #[cfg(feature = "lang-typescript")]
            Language::TypeScript,
            #[cfg(feature = "lang-typescript")]
            Language::JavaScript,
            #[cfg(feature = "lang-typescript")]
            Language::Tsx,
            #[cfg(feature = "lang-typescript")]
            Language::Jsx,
            #[cfg(feature = "lang-rust")]
            Language::Rust,
            #[cfg(feature = "lang-python")]
            Language::Python,
            #[cfg(feature = "lang-go")]
            Language::Go,
            #[cfg(feature = "lang-php")]
            Language::Php,
            #[cfg(feature = "lang-java")]
            Language::Java,
            #[cfg(feature = "lang-csharp")]
            Language::CSharp,
            #[cfg(feature = "lang-ruby")]
            Language::Ruby,
            #[cfg(feature = "lang-dart")]
            Language::Dart,
            #[cfg(feature = "lang-swift")]
            Language::Swift,
            #[cfg(feature = "lang-graphql")]
            Language::GraphQL,
            #[cfg(feature = "lang-hcl")]
            Language::Hcl,
            // Note: Kotlin blocked by tree-sitter version constraint
            #[cfg(feature = "lang-c")]
            Language::C,
            #[cfg(feature = "lang-cpp")]
            Language::Cpp,
        ]
    }

    /// Extract from source using the tree-sitter extractor
    pub fn extract(
        &self,
        source: &str,
        file_path: &str,
        lang: Language,
    ) -> Result<ExtractionResult, ExtractionError> {
        let mut extractor = self.extractor.lock().map_err(|_| {
            ExtractionError::ParseFailed {
                language: lang.as_str().to_string(),
                message: "Failed to acquire extractor lock".to_string(),
            }
        })?;

        extractor.extract(source, file_path, lang)
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
    fn test_registry_supports_configured_languages() {
        let registry = ExtractorRegistry::new();

        #[cfg(feature = "lang-typescript")]
        assert!(registry.supports(Language::TypeScript));

        #[cfg(feature = "lang-rust")]
        assert!(registry.supports(Language::Rust));

        #[cfg(feature = "lang-python")]
        assert!(registry.supports(Language::Python));

        #[cfg(feature = "lang-go")]
        assert!(registry.supports(Language::Go));
    }

    #[test]
    #[cfg(feature = "lang-typescript")]
    fn test_extract_typescript() {
        let registry = ExtractorRegistry::new();
        let source = r#"
export function greet(name: string): string {
    return `Hello, ${name}!`;
}
"#;
        let result = registry.extract(source, "test.ts", Language::TypeScript).unwrap();
        assert!(!result.nodes.is_empty());
    }

    #[test]
    #[cfg(feature = "lang-rust")]
    fn test_extract_rust() {
        let registry = ExtractorRegistry::new();
        let source = r#"
pub fn main() {
    println!("Hello, world!");
}
"#;
        let result = registry.extract(source, "main.rs", Language::Rust).unwrap();
        assert!(!result.nodes.is_empty());
    }

    #[test]
    fn test_generate_node_id() {
        let id1 = generate_node_id("test.rs", NodeKind::Function, "foo", 1);
        let id2 = generate_node_id("test.rs", NodeKind::Function, "foo", 1);
        let id3 = generate_node_id("test.rs", NodeKind::Function, "bar", 1);

        // Same inputs should produce same ID
        assert_eq!(id1, id2);
        // Different inputs should produce different ID
        assert_ne!(id1, id3);
    }
}
