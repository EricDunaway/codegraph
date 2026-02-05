//! Dart LSP enricher
//!
//! Uses dart language-server to provide:
//! - Type inference from hover queries
//! - Import resolution from definition queries

use crate::base::BaseEnricher;
use crate::enricher::{DefinitionResult, HoverResult, LspEnricher};
use crate::error::LspError;
use async_trait::async_trait;
use codegraph_types::{Language, LspServerConfig};
use std::path::Path;

/// Dart LSP enricher with concurrent request support (I5)
pub struct DartEnricher {
    base: BaseEnricher,
}

impl DartEnricher {
    /// Create a new Dart enricher with the given configuration
    pub fn new(config: LspServerConfig) -> Self {
        Self {
            base: BaseEnricher::new(config, Language::Dart, "dart"),
        }
    }

    /// Parse a Dart type declaration to extract the type
    pub fn parse_type_declaration(decl: &str) -> Option<String> {
        let decl = decl.trim();

        let decl = decl
            .strip_prefix("final ")
            .or_else(|| decl.strip_prefix("const "))
            .unwrap_or(decl);

        let decl = decl.strip_prefix("late ").unwrap_or(decl);

        if decl.starts_with("var ") {
            return Some("dynamic".to_string());
        }

        if let Some(paren_pos) = decl.find('(') {
            if decl.contains(" Function(") || decl.contains(" Function<") {
                return Some(decl.to_string());
            }

            let before_paren = &decl[..paren_pos];
            let parts: Vec<&str> = before_paren.split_whitespace().collect();
            if parts.len() >= 2 {
                let type_parts = &parts[..parts.len() - 1];
                return Some(type_parts.join(" "));
            }
        }

        let parts: Vec<&str> = decl.split_whitespace().collect();
        if parts.len() >= 2 {
            let type_str = parts[0];
            if type_str.contains('<') && !type_str.contains('>') {
                let mut full_type = type_str.to_string();
                for part in &parts[1..] {
                    full_type.push_str(part);
                    if part.contains('>') {
                        break;
                    }
                }
                return Some(full_type);
            }
            return Some(type_str.to_string());
        }

        if !decl.is_empty() && !decl.contains(' ') {
            return Some(decl.to_string());
        }

        None
    }
}

#[async_trait]
impl LspEnricher for DartEnricher {
    async fn start(&mut self, workspace_root: &Path) -> Result<(), LspError> {
        self.base.start(workspace_root).await
    }

    async fn hover(
        &self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Option<HoverResult>, LspError> {
        self.base
            .hover(file, line, column, Self::parse_type_declaration)
            .await
    }

    async fn definition(
        &self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Option<DefinitionResult>, LspError> {
        self.base.definition(file, line, column).await
    }

    async fn shutdown(&mut self) -> Result<(), LspError> {
        self.base.shutdown().await
    }

    async fn is_ready(&self) -> bool {
        self.base.is_ready().await
    }

    fn language(&self) -> Language {
        Language::Dart
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_type() {
        let parsed = DartEnricher::parse_type_declaration("String name");
        assert_eq!(parsed.as_deref(), Some("String"));
    }

    #[test]
    fn test_parse_var_type() {
        let parsed = DartEnricher::parse_type_declaration("var x");
        assert_eq!(parsed.as_deref(), Some("dynamic"));
    }

    #[test]
    fn test_parse_generic_type() {
        let parsed = DartEnricher::parse_type_declaration("List<String> items");
        assert_eq!(parsed.as_deref(), Some("List<String>"));
    }

    #[test]
    fn test_enricher_creation() {
        let config = LspServerConfig {
            enabled: true,
            server: "dart".to_string(),
            args: vec!["language-server".to_string()],
        };

        let enricher = DartEnricher::new(config);
        assert_eq!(enricher.language(), Language::Dart);
    }

    #[tokio::test]
    async fn test_enricher_disabled() {
        let config = LspServerConfig {
            enabled: false,
            server: "dart".to_string(),
            args: vec![],
        };

        let mut enricher = DartEnricher::new(config);
        let result = enricher.start(Path::new("/tmp")).await;
        assert!(result.is_err());
    }
}
