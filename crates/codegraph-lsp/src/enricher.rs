//! LSP enricher trait and result types

use crate::error::LspError;
use async_trait::async_trait;
use codegraph_types::Language;
use std::path::Path;

/// Result of an LSP hover query
#[derive(Debug, Clone)]
pub struct HoverResult {
    /// Inferred type from hover contents
    pub inferred_type: Option<String>,
    /// Documentation from hover
    pub documentation: Option<String>,
}

/// Result of an LSP definition query
#[derive(Debug, Clone)]
pub struct DefinitionResult {
    /// File path of the definition
    pub file_path: String,
    /// Line number (0-indexed)
    pub line: u32,
    /// Column number (0-indexed)
    pub column: u32,
}

/// LSP enricher trait for language-specific implementations
///
/// This trait defines the interface for language-specific LSP clients
/// that provide type inference and definition resolution.
///
/// All methods are async to support concurrent requests (I5).
#[async_trait]
pub trait LspEnricher: Send + Sync {
    /// Start the LSP server for a workspace
    async fn start(&mut self, workspace_root: &Path) -> Result<(), LspError>;

    /// Query hover information at a position (can be called concurrently)
    ///
    /// Returns type and documentation info from the language server.
    async fn hover(
        &self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Option<HoverResult>, LspError>;

    /// Query definition at a position (can be called concurrently)
    ///
    /// Returns the location of the symbol definition (for import resolution).
    async fn definition(
        &self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Option<DefinitionResult>, LspError>;

    /// Shutdown the LSP server
    async fn shutdown(&mut self) -> Result<(), LspError>;

    /// Check if server is ready (workspace fully initialized)
    async fn is_ready(&self) -> bool;

    /// Get the language this enricher handles
    fn language(&self) -> Language;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hover_result_creation() {
        let result = HoverResult {
            inferred_type: Some("Promise<void>".to_string()),
            documentation: Some("Does something async".to_string()),
        };

        assert_eq!(result.inferred_type.as_deref(), Some("Promise<void>"));
        assert!(result.documentation.is_some());
    }

    #[test]
    fn test_definition_result_creation() {
        let result = DefinitionResult {
            file_path: "src/lib.rs".to_string(),
            line: 10,
            column: 4,
        };

        assert_eq!(result.file_path, "src/lib.rs");
        assert_eq!(result.line, 10);
        assert_eq!(result.column, 4);
    }
}
