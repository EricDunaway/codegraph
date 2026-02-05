//! LSP enrichment for CodeGraph
//!
//! This crate provides language server integration for enriching code nodes with:
//! - Inferred types (from hover queries)
//! - Resolved import paths (from definition queries)
//!
//! # Architecture
//!
//! The crate is organized around the [`LspEnricher`] trait, which defines the interface
//! for language-specific LSP clients. Each supported language has its own enricher
//! implementation that handles the specifics of that language's LSP server.
//!
//! # Supported Languages
//!
//! - TypeScript/JavaScript (via typescript-language-server)
//! - Dart/Flutter (via dart language-server)
//! - Rust (via rust-analyzer)
//! - Python (via pyright-langserver)
//! - Go (via gopls)

use codegraph_types::{LspConfig, LspServerConfig};

pub mod base;
pub mod batch;
pub mod client;
pub mod dart;
pub mod encoding;
pub mod enricher;
pub mod error;
pub mod go;
pub mod lifecycle;
pub mod python;
pub mod rust_analyzer;
pub mod sync_bridge;
pub mod typescript;

pub use base::{parse_definition, uri_to_path, BaseEnricher};
pub use batch::{
    enrich_batch, enrich_node, BatchEnrichmentResult, EnrichmentRequest, FilePosition,
    NodeEnrichmentResult,
};
pub use client::LspClient;
pub use dart::DartEnricher;
pub use encoding::{byte_offset_to_position, byte_to_utf16, position_to_byte_offset, utf16_to_byte};
pub use enricher::{DefinitionResult, HoverResult, LspEnricher};
pub use error::LspError;
pub use go::GoEnricher;
pub use lifecycle::LspServerManager;
pub use python::PythonEnricher;
pub use rust_analyzer::RustEnricher;
pub use sync_bridge::LspSyncBridge;
pub use typescript::TypeScriptEnricher;

/// Validation error for LSP configuration
#[derive(Debug, Clone)]
pub struct LspConfigError {
    /// The language that has the invalid configuration
    pub language: String,
    /// Description of the error
    pub message: String,
}

impl std::fmt::Display for LspConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid {} LSP config: {}", self.language, self.message)
    }
}

impl std::error::Error for LspConfigError {}

/// Validate a single language's LSP server configuration
fn validate_server_config(name: &str, config: &LspServerConfig) -> Result<(), LspConfigError> {
    if config.enabled && config.server.trim().is_empty() {
        return Err(LspConfigError {
            language: name.to_string(),
            message: "server path cannot be empty when enabled".to_string(),
        });
    }
    Ok(())
}

/// Validate the LSP configuration
///
/// Checks that all enabled language servers have valid configurations:
/// - Server path must not be empty when enabled
/// - Returns Ok(()) if validation passes
/// - Returns Err with details if validation fails
///
/// # Example
///
/// ```rust
/// use codegraph_types::{LspConfig, LspServerConfig};
/// use codegraph_lsp::validate_lsp_config;
///
/// let config = LspConfig {
///     enabled: true,
///     typescript: Some(LspServerConfig {
///         enabled: true,
///         server: "typescript-language-server".to_string(),
///         args: vec!["--stdio".to_string()],
///     }),
///     ..Default::default()
/// };
///
/// assert!(validate_lsp_config(&config).is_ok());
/// ```
pub fn validate_lsp_config(config: &LspConfig) -> Result<(), LspConfigError> {
    // If LSP is disabled globally, no validation needed
    if !config.enabled {
        return Ok(());
    }

    // Validate each language's configuration
    if let Some(ref ts_config) = config.typescript {
        validate_server_config("TypeScript", ts_config)?;
    }

    if let Some(ref dart_config) = config.dart {
        validate_server_config("Dart", dart_config)?;
    }

    if let Some(ref rust_config) = config.rust {
        validate_server_config("Rust", rust_config)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crate_compiles() {
        // Verify the crate structure works
        let _err: Option<LspError> = None;
    }

    #[test]
    fn test_exports_accessible() {
        // Verify public exports
        let _ = HoverResult {
            inferred_type: None,
            documentation: None,
        };

        let _ = DefinitionResult {
            file_path: String::new(),
            line: 0,
            column: 0,
        };
    }

    #[test]
    fn test_validate_typescript_config() {
        let config = LspConfig {
            enabled: true,
            typescript: Some(LspServerConfig {
                enabled: true,
                server: "typescript-language-server".to_string(),
                args: vec!["--stdio".to_string()],
            }),
            ..Default::default()
        };

        assert!(validate_lsp_config(&config).is_ok());
    }

    #[test]
    fn test_validate_empty_server_fails() {
        let config = LspConfig {
            enabled: true,
            typescript: Some(LspServerConfig {
                enabled: true,
                server: "".to_string(), // Empty!
                args: vec![],
            }),
            ..Default::default()
        };

        assert!(validate_lsp_config(&config).is_err());
    }

    #[test]
    fn test_validate_whitespace_server_fails() {
        let config = LspConfig {
            enabled: true,
            typescript: Some(LspServerConfig {
                enabled: true,
                server: "   ".to_string(), // Just whitespace
                args: vec![],
            }),
            ..Default::default()
        };

        assert!(validate_lsp_config(&config).is_err());
    }

    #[test]
    fn test_validate_disabled_config_passes() {
        // Even with invalid server path, disabled config should pass
        let config = LspConfig {
            enabled: false, // Globally disabled
            typescript: Some(LspServerConfig {
                enabled: true,
                server: "".to_string(), // Would be invalid if enabled
                args: vec![],
            }),
            ..Default::default()
        };

        assert!(validate_lsp_config(&config).is_ok());
    }

    #[test]
    fn test_validate_disabled_language_passes() {
        // Language-specific disabled should pass even with empty server
        let config = LspConfig {
            enabled: true,
            typescript: Some(LspServerConfig {
                enabled: false, // Language disabled
                server: "".to_string(), // Would be invalid if enabled
                args: vec![],
            }),
            ..Default::default()
        };

        assert!(validate_lsp_config(&config).is_ok());
    }

    #[test]
    fn test_validate_multiple_languages() {
        let config = LspConfig {
            enabled: true,
            typescript: Some(LspServerConfig {
                enabled: true,
                server: "typescript-language-server".to_string(),
                args: vec!["--stdio".to_string()],
            }),
            dart: Some(LspServerConfig {
                enabled: true,
                server: "dart".to_string(),
                args: vec!["language-server".to_string()],
            }),
            rust: Some(LspServerConfig {
                enabled: true,
                server: "rust-analyzer".to_string(),
                args: vec![],
            }),
        };

        assert!(validate_lsp_config(&config).is_ok());
    }

    #[test]
    fn test_validate_error_message() {
        let config = LspConfig {
            enabled: true,
            dart: Some(LspServerConfig {
                enabled: true,
                server: "".to_string(),
                args: vec![],
            }),
            ..Default::default()
        };

        let err = validate_lsp_config(&config).unwrap_err();
        assert_eq!(err.language, "Dart");
        assert!(err.message.contains("empty"));
    }
}
