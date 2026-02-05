//! Rust LSP enricher (rust-analyzer)
//!
//! Uses rust-analyzer to provide:
//! - Type inference from hover queries
//! - Import resolution from definition queries

use crate::base::BaseEnricher;
use crate::enricher::{DefinitionResult, HoverResult, LspEnricher};
use crate::error::LspError;
use async_trait::async_trait;
use codegraph_types::{Language, LspServerConfig};
use std::path::Path;

/// Rust LSP enricher with concurrent request support (I5)
pub struct RustEnricher {
    base: BaseEnricher,
}

impl RustEnricher {
    /// Create a new Rust enricher with the given configuration
    pub fn new(config: LspServerConfig) -> Self {
        Self {
            base: BaseEnricher::new(config, Language::Rust, "rust"),
        }
    }

    /// Parse a Rust type declaration to extract the type
    pub fn parse_type_declaration(decl: &str) -> Option<String> {
        let decl = decl.trim();

        if let Some(rest) = decl.strip_prefix("let ") {
            let rest = rest.strip_prefix("mut ").unwrap_or(rest);
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

        for keyword in &["const ", "static "] {
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

        if decl.starts_with("fn ") || decl.starts_with("pub fn ") || decl.starts_with("async fn ") {
            if let Some(arrow_pos) = decl.find(" -> ") {
                let return_type = &decl[arrow_pos + 4..];
                let return_type = return_type.split('{').next().unwrap_or(return_type).trim();
                return Some(return_type.to_string());
            } else {
                return Some("()".to_string());
            }
        }

        for keyword in &["struct ", "enum ", "trait ", "union "] {
            if decl.contains(keyword) {
                if let Some(start) = decl.find(keyword) {
                    let rest = &decl[start + keyword.len()..];
                    let name = rest
                        .split(|c: char| c == '<' || c == '{' || c == '(' || c.is_whitespace())
                        .next()
                        .filter(|s| !s.is_empty())?;
                    return Some(name.to_string());
                }
            }
        }

        if decl.starts_with("type ") || decl.starts_with("pub type ") {
            if let Some(eq_pos) = decl.find('=') {
                let target = decl[eq_pos + 1..].trim();
                return Some(target.to_string());
            }
        }

        if decl.starts_with("impl ") || decl.starts_with("impl<") {
            let rest = decl.strip_prefix("impl").unwrap_or(decl);
            let rest = if let Some(gt_pos) = rest.find('>') {
                &rest[gt_pos + 1..]
            } else {
                rest
            }
            .trim();

            if let Some(for_pos) = rest.find(" for ") {
                let type_name = &rest[for_pos + 5..];
                let type_name = type_name
                    .split(|c: char| c == '{' || c.is_whitespace())
                    .next()
                    .filter(|s| !s.is_empty())?;
                return Some(type_name.to_string());
            }

            let type_name = rest
                .split(|c: char| c == '{' || c.is_whitespace())
                .next()
                .filter(|s| !s.is_empty())?;
            return Some(type_name.to_string());
        }

        if let Some(colon_pos) = decl.find(": ") {
            let type_str = &decl[colon_pos + 2..];
            let type_str = type_str.split(',').next().unwrap_or(type_str).trim();
            if !type_str.is_empty() {
                return Some(type_str.to_string());
            }
        }

        None
    }
}

#[async_trait]
impl LspEnricher for RustEnricher {
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
        Language::Rust
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_let_type() {
        let parsed = RustEnricher::parse_type_declaration("let x: i32");
        assert_eq!(parsed.as_deref(), Some("i32"));
    }

    #[test]
    fn test_parse_fn_return_type() {
        let parsed = RustEnricher::parse_type_declaration("fn add(a: i32, b: i32) -> i32");
        assert_eq!(parsed.as_deref(), Some("i32"));
    }

    #[test]
    fn test_parse_struct() {
        let parsed = RustEnricher::parse_type_declaration("struct User");
        assert_eq!(parsed.as_deref(), Some("User"));
    }

    #[test]
    fn test_enricher_creation() {
        let config = LspServerConfig {
            enabled: true,
            server: "rust-analyzer".to_string(),
            args: vec![],
        };

        let enricher = RustEnricher::new(config);
        assert_eq!(enricher.language(), Language::Rust);
    }

    #[tokio::test]
    async fn test_enricher_disabled() {
        let config = LspServerConfig {
            enabled: false,
            server: "rust-analyzer".to_string(),
            args: vec![],
        };

        let mut enricher = RustEnricher::new(config);
        let result = enricher.start(Path::new("/tmp")).await;
        assert!(result.is_err());
    }
}
