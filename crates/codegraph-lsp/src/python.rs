//! Python LSP enricher (pyright/pylsp)
//!
//! Uses Pyright or Python Language Server to provide:
//! - Type inference from hover queries
//! - Import resolution from definition queries

use crate::base::BaseEnricher;
use crate::enricher::{DefinitionResult, HoverResult, LspEnricher};
use crate::error::LspError;
use async_trait::async_trait;
use codegraph_types::{Language, LspServerConfig};
use std::path::Path;

/// Python LSP enricher with concurrent request support (I5)
pub struct PythonEnricher {
    base: BaseEnricher,
}

impl PythonEnricher {
    /// Create a new Python enricher with the given configuration
    pub fn new(config: LspServerConfig) -> Self {
        Self {
            base: BaseEnricher::new(config, Language::Python, "python"),
        }
    }

    /// Parse a Python type annotation to extract the type
    pub fn parse_type_annotation(decl: &str) -> Option<String> {
        let decl = decl.trim();

        let decl = if decl.starts_with('(') {
            if let Some(paren_end) = decl.find(')') {
                decl[paren_end + 1..].trim()
            } else {
                decl
            }
        } else {
            decl
        };

        if decl.starts_with("def ") || decl.starts_with("async def ") {
            if let Some(arrow_pos) = decl.find(" -> ") {
                let return_type = &decl[arrow_pos + 4..];
                let return_type = return_type.trim_end_matches(':').trim();
                return Some(return_type.to_string());
            } else {
                return Some("None".to_string());
            }
        }

        if decl.starts_with("class ") {
            let rest = decl.strip_prefix("class ").unwrap_or(decl);
            let name = rest
                .split(|c: char| c == ':' || c == '(' || c.is_whitespace())
                .next()
                .filter(|s| !s.is_empty())?;
            return Some(name.to_string());
        }

        if let Some(colon_pos) = decl.find(": ") {
            let type_part = &decl[colon_pos + 2..];
            let type_str = type_part.split('=').next().map(str::trim)?;
            if !type_str.is_empty() {
                return Some(type_str.to_string());
            }
        }

        if let Some(type_comment_pos) = decl.find("# type:") {
            let type_str = &decl[type_comment_pos + 7..];
            return Some(type_str.trim().to_string());
        }

        None
    }
}

#[async_trait]
impl LspEnricher for PythonEnricher {
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
            .hover(file, line, column, Self::parse_type_annotation)
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
        Language::Python
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_type() {
        let parsed = PythonEnricher::parse_type_annotation("name: str");
        assert_eq!(parsed.as_deref(), Some("str"));
    }

    #[test]
    fn test_parse_function_return() {
        let parsed = PythonEnricher::parse_type_annotation("def get_name() -> str:");
        assert_eq!(parsed.as_deref(), Some("str"));
    }

    #[test]
    fn test_parse_class() {
        let parsed = PythonEnricher::parse_type_annotation("class User:");
        assert_eq!(parsed.as_deref(), Some("User"));
    }

    #[test]
    fn test_enricher_creation() {
        let config = LspServerConfig {
            enabled: true,
            server: "pyright-langserver".to_string(),
            args: vec!["--stdio".to_string()],
        };

        let enricher = PythonEnricher::new(config);
        assert_eq!(enricher.language(), Language::Python);
    }

    #[tokio::test]
    async fn test_enricher_disabled() {
        let config = LspServerConfig {
            enabled: false,
            server: "pyright-langserver".to_string(),
            args: vec![],
        };

        let mut enricher = PythonEnricher::new(config);
        let result = enricher.start(Path::new("/tmp")).await;
        assert!(result.is_err());
    }
}
