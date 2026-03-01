//! Go LSP enricher (gopls)
//!
//! Uses gopls to provide:
//! - Type inference from hover queries
//! - Import resolution from definition queries

use crate::base::BaseEnricher;
use crate::enricher::{DefinitionResult, HoverResult, LspEnricher};
use crate::error::LspError;
use async_trait::async_trait;
use codegraph_types::{Language, LspServerConfig};
use std::path::Path;

/// Go LSP enricher with concurrent request support (I5)
pub struct GoEnricher {
    base: BaseEnricher,
}

impl GoEnricher {
    /// Create a new Go enricher with the given configuration
    pub fn new(config: LspServerConfig) -> Self {
        Self {
            base: BaseEnricher::new(config, Language::Go, "go"),
        }
    }

    /// Parse a Go type declaration to extract the type
    pub fn parse_type_declaration(decl: &str) -> Option<String> {
        let decl = decl.trim();

        if let Some(rest) = decl.strip_prefix("var ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2 {
                let type_str = parts[1].trim_end_matches('=');
                if !type_str.is_empty() {
                    return Some(type_str.to_string());
                }
            }
        }

        if let Some(rest) = decl.strip_prefix("const ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2 {
                let type_str = parts[1].trim_end_matches('=');
                if !type_str.is_empty() && type_str != "=" {
                    return Some(type_str.to_string());
                }
            }
        }

        if decl.starts_with("func ") {
            let mut paren_count = 0;
            let mut param_end = 0;

            for (i, c) in decl.char_indices() {
                if c == '(' {
                    paren_count += 1;
                } else if c == ')' {
                    paren_count -= 1;
                    if paren_count == 0 {
                        param_end = i;
                        break;
                    }
                }
            }

            if param_end > 0 {
                let return_part = &decl[param_end + 1..];
                let return_type = return_part.trim().trim_start_matches('{').trim();
                if !return_type.is_empty() {
                    return Some(return_type.to_string());
                }
            }
        }

        if decl.starts_with("type ") {
            let parts: Vec<&str> = decl.split_whitespace().collect();
            if parts.len() >= 2 {
                return Some(parts[1].to_string());
            }
        }

        let parts: Vec<&str> = decl.split_whitespace().collect();
        if parts.len() >= 2 {
            let first = parts[0];
            let second = parts[1];

            let keywords = [
                "package", "import", "func", "var", "const", "type", "struct", "interface",
                "return", "if", "else", "for", "range", "switch", "case", "default", "go",
                "defer", "select", "chan", "map", "make", "new",
            ];

            if !keywords.contains(&first) {
                return Some(second.to_string());
            }
        }

        None
    }
}

#[async_trait]
impl LspEnricher for GoEnricher {
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
        Language::Go
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_var_type() {
        let parsed = GoEnricher::parse_type_declaration("var count int");
        assert_eq!(parsed.as_deref(), Some("int"));
    }

    #[test]
    fn test_parse_func_return() {
        let parsed = GoEnricher::parse_type_declaration("func GetName() string");
        assert_eq!(parsed.as_deref(), Some("string"));
    }

    #[test]
    fn test_parse_type_struct() {
        let parsed = GoEnricher::parse_type_declaration("type User struct");
        assert_eq!(parsed.as_deref(), Some("User"));
    }

    #[test]
    fn test_enricher_creation() {
        let config = LspServerConfig {
            enabled: true,
            server: "gopls".to_string(),
            args: vec![],
        };

        let enricher = GoEnricher::new(config);
        assert_eq!(enricher.language(), Language::Go);
    }

    #[tokio::test]
    async fn test_enricher_disabled() {
        let config = LspServerConfig {
            enabled: false,
            server: "gopls".to_string(),
            args: vec![],
        };

        let mut enricher = GoEnricher::new(config);
        let result = enricher.start(Path::new("/tmp")).await;
        assert!(result.is_err());
    }
}
