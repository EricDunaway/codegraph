//! TypeScript/JavaScript LSP enricher
//!
//! Uses typescript-language-server to provide:
//! - Type inference from hover queries
//! - Import resolution from definition queries

use crate::base::BaseEnricher;
use crate::enricher::{DefinitionResult, HoverResult, LspEnricher};
use crate::error::LspError;
use async_trait::async_trait;
use codegraph_types::{Language, LspServerConfig};
use std::path::Path;

/// TypeScript/JavaScript LSP enricher with concurrent request support (I5)
pub struct TypeScriptEnricher {
    base: BaseEnricher,
}

impl TypeScriptEnricher {
    /// Create a new TypeScript enricher with the given configuration
    pub fn new(config: LspServerConfig) -> Self {
        Self {
            base: BaseEnricher::new(config, Language::TypeScript, "typescript"),
        }
    }

    /// Parse a TypeScript type declaration to extract the type
    pub fn parse_type_declaration(decl: &str) -> Option<String> {
        let decl = decl.trim();

        // Handle `(parameter) foo: Type` pattern
        if let Some(rest) = decl.strip_prefix("(parameter)") {
            return Self::parse_type_declaration(rest);
        }

        // Handle variable declarations: `const/let/var name: Type`
        for keyword in &["const ", "let ", "var "] {
            if let Some(rest) = decl.strip_prefix(keyword) {
                if let Some(colon_pos) = rest.find(':') {
                    let type_part = &rest[colon_pos + 1..];
                    let type_str = type_part
                        .split('=')
                        .next()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())?;
                    return Some(type_str.to_string());
                }
            }
        }

        // Handle function declarations
        if decl.starts_with("function ") || decl.contains("): ") {
            if let Some(paren_pos) = decl.rfind("): ") {
                let return_type = &decl[paren_pos + 3..];
                return Some(return_type.trim().to_string());
            }
        }

        // Handle method/arrow
        if let Some(paren_pos) = decl.find("): ") {
            let return_type = &decl[paren_pos + 3..];
            return Some(return_type.trim().to_string());
        }

        // Handle simple `: Type` pattern
        if let Some(colon_pos) = decl.rfind(": ") {
            let type_str = &decl[colon_pos + 2..];
            if !type_str.is_empty() && !type_str.contains('(') {
                return Some(type_str.trim().to_string());
            }
        }

        None
    }
}

#[async_trait]
impl LspEnricher for TypeScriptEnricher {
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
        Language::TypeScript
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_const_type() {
        let parsed = TypeScriptEnricher::parse_type_declaration("const foo: string");
        assert_eq!(parsed.as_deref(), Some("string"));
    }

    #[test]
    fn test_parse_let_type() {
        let parsed = TypeScriptEnricher::parse_type_declaration("let count: number");
        assert_eq!(parsed.as_deref(), Some("number"));
    }

    #[test]
    fn test_parse_function_return_type() {
        let parsed = TypeScriptEnricher::parse_type_declaration("function greet(): string");
        assert_eq!(parsed.as_deref(), Some("string"));
    }

    #[test]
    fn test_parse_complex_type() {
        let parsed = TypeScriptEnricher::parse_type_declaration("const data: Promise<User[]>");
        assert_eq!(parsed.as_deref(), Some("Promise<User[]>"));
    }

    #[test]
    fn test_parse_parameter_type() {
        let parsed = TypeScriptEnricher::parse_type_declaration("(parameter) userId: string");
        assert_eq!(parsed.as_deref(), Some("string"));
    }

    #[test]
    fn test_enricher_creation() {
        let config = LspServerConfig {
            enabled: true,
            server: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
        };

        let enricher = TypeScriptEnricher::new(config);
        assert_eq!(enricher.language(), Language::TypeScript);
    }

    #[tokio::test]
    async fn test_enricher_disabled() {
        let config = LspServerConfig {
            enabled: false,
            server: "typescript-language-server".to_string(),
            args: vec![],
        };

        let mut enricher = TypeScriptEnricher::new(config);
        let result = enricher.start(Path::new("/tmp")).await;
        assert!(result.is_err());
    }
}
