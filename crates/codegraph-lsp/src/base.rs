//! Shared utilities for LSP enrichers
//!
//! This module provides common functionality used by all language-specific enrichers,
//! eliminating code duplication across TypeScript, Dart, Rust, Python, and Go enrichers.

use crate::client::LspClient;
use crate::enricher::{DefinitionResult, HoverResult};
use crate::error::LspError;
use codegraph_types::{Language, LspServerConfig};
use lsp_types::{GotoDefinitionResponse, Hover, HoverContents, MarkedString};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use url::Url;

/// Parse a definition response to extract location
pub fn parse_definition(response: &GotoDefinitionResponse) -> Option<DefinitionResult> {
    match response {
        GotoDefinitionResponse::Scalar(location) => location_to_definition_result(location),
        GotoDefinitionResponse::Array(locations) => {
            locations.first().and_then(location_to_definition_result)
        }
        GotoDefinitionResponse::Link(links) => links.first().map(|link| DefinitionResult {
            file_path: uri_to_path(&link.target_uri),
            line: link.target_selection_range.start.line,
            column: link.target_selection_range.start.character,
        }),
    }
}

/// Convert an LSP Location to a DefinitionResult
pub fn location_to_definition_result(location: &lsp_types::Location) -> Option<DefinitionResult> {
    Some(DefinitionResult {
        file_path: uri_to_path(&location.uri),
        line: location.range.start.line,
        column: location.range.start.character,
    })
}

/// Convert an LSP URI to a file path string
pub fn uri_to_path(uri: &lsp_types::Uri) -> String {
    if let Ok(url) = Url::parse(uri.as_str()) {
        if url.scheme() == "file" {
            if let Ok(path) = url.to_file_path() {
                return path.display().to_string();
            }
        }
    }
    uri.as_str().to_string()
}

/// Extract documentation from hover contents
pub fn extract_documentation(hover: &Hover) -> Option<String> {
    match &hover.contents {
        HoverContents::Markup(markup) => Some(markup.value.clone()),
        HoverContents::Scalar(MarkedString::String(s)) => Some(s.clone()),
        HoverContents::Scalar(MarkedString::LanguageString(ls)) => Some(ls.value.clone()),
        HoverContents::Array(contents) => contents.first().map(|ms| match ms {
            MarkedString::String(s) => s.clone(),
            MarkedString::LanguageString(ls) => ls.value.clone(),
        }),
    }
}

/// Type parser function signature
pub type TypeParser = fn(&str) -> Option<String>;

/// Parse hover contents to extract type information using a language-specific parser
pub fn parse_hover_type(hover: &Hover, language: &str, parser: TypeParser) -> Option<String> {
    match &hover.contents {
        HoverContents::Scalar(content) => extract_type_from_marked_string(content, language, parser),
        HoverContents::Array(contents) => contents
            .iter()
            .find_map(|c| extract_type_from_marked_string(c, language, parser)),
        HoverContents::Markup(markup) => extract_type_from_markdown(&markup.value, language, parser),
    }
}

/// Extract type from a MarkedString
fn extract_type_from_marked_string(
    ms: &MarkedString,
    language: &str,
    parser: TypeParser,
) -> Option<String> {
    match ms {
        MarkedString::String(s) => extract_type_from_markdown(s, language, parser),
        MarkedString::LanguageString(ls) => {
            if ls.language == language
                || (language == "typescript" && ls.language == "javascript")
            {
                parser(&ls.value)
            } else {
                None
            }
        }
    }
}

/// Extract type from markdown content
fn extract_type_from_markdown(md: &str, language: &str, parser: TypeParser) -> Option<String> {
    for block in md.split("```") {
        let trimmed = block.trim();
        if trimmed.starts_with(language)
            || (language == "typescript" && trimmed.starts_with("javascript"))
        {
            let code = trimmed
                .trim_start_matches(language)
                .trim_start_matches("javascript")
                .trim();
            if let Some(t) = parser(code) {
                return Some(t);
            }
        }
    }
    None
}

/// Base enricher that handles common functionality with async concurrent support (I5)
///
/// Language-specific enrichers can embed this and delegate common operations.
/// Supports true concurrent requests to a single LSP server by storing the
/// client in an Arc and releasing the initialization lock before making requests.
pub struct BaseEnricher {
    config: LspServerConfig,
    /// Client wrapped in Arc for concurrent access. The Mutex only protects
    /// the Option (for initialization/shutdown), not the client operations.
    client: Arc<Mutex<Option<Arc<LspClient>>>>,
    language: Language,
    language_id: &'static str,
}

impl BaseEnricher {
    /// Create a new base enricher
    pub fn new(config: LspServerConfig, language: Language, language_id: &'static str) -> Self {
        Self {
            config,
            client: Arc::new(Mutex::new(None)),
            language,
            language_id,
        }
    }

    /// Get the language this enricher handles
    pub fn language(&self) -> Language {
        self.language
    }

    /// Get the language ID string (for markdown parsing)
    pub fn language_id(&self) -> &'static str {
        self.language_id
    }

    /// Check if enricher is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Check if server is ready
    pub async fn is_ready(&self) -> bool {
        // Get Arc clone with minimal lock hold time
        let client = {
            let guard = self.client.lock().await;
            guard.as_ref().cloned()
        };
        if let Some(client) = client {
            client.is_initialized().await
        } else {
            false
        }
    }

    /// Start the LSP server
    pub async fn start(&self, workspace_root: &Path) -> Result<(), LspError> {
        if !self.config.enabled {
            return Err(LspError::UnsupportedLanguage(format!(
                "{:?} LSP not enabled",
                self.language
            )));
        }

        let root_url = Url::from_file_path(workspace_root)
            .map_err(|_| LspError::FileNotFound(workspace_root.display().to_string()))?;

        let client = LspClient::spawn(&self.config.server, &self.config.args).await?;
        client.initialize(&root_url).await?;

        *self.client.lock().await = Some(Arc::new(client));
        Ok(())
    }

    /// Get a clone of the client Arc (minimal lock hold time)
    async fn get_client(&self) -> Result<Arc<LspClient>, LspError> {
        let guard = self.client.lock().await;
        guard
            .as_ref()
            .cloned()
            .ok_or_else(|| LspError::NotReady("Server not started".to_string()))
    }

    /// Execute a hover query (can be called concurrently - I5)
    ///
    /// This method releases the initialization lock before making the LSP request,
    /// allowing multiple concurrent hover queries to the same server.
    pub async fn hover(
        &self,
        file: &Path,
        line: u32,
        column: u32,
        parser: TypeParser,
    ) -> Result<Option<HoverResult>, LspError> {
        // Get client with minimal lock hold time - CRITICAL for I5 concurrency
        let client = self.get_client().await?;

        // Lock released - now make the LSP request without holding any locks
        match client.hover(file, line, column).await? {
            Some(hover) => {
                let inferred_type = parse_hover_type(&hover, self.language_id, parser);
                let documentation = extract_documentation(&hover);

                Ok(Some(HoverResult {
                    inferred_type,
                    documentation,
                }))
            }
            None => Ok(None),
        }
    }

    /// Execute a definition query (can be called concurrently - I5)
    ///
    /// This method releases the initialization lock before making the LSP request,
    /// allowing multiple concurrent definition queries to the same server.
    pub async fn definition(
        &self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Option<DefinitionResult>, LspError> {
        // Get client with minimal lock hold time - CRITICAL for I5 concurrency
        let client = self.get_client().await?;

        // Lock released - now make the LSP request without holding any locks
        match client.definition(file, line, column).await? {
            Some(response) => Ok(parse_definition(&response)),
            None => Ok(None),
        }
    }

    /// Shutdown the LSP server
    pub async fn shutdown(&self) -> Result<(), LspError> {
        let client = {
            let mut guard = self.client.lock().await;
            guard.take()
        };
        if let Some(client) = client {
            client.shutdown().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uri_to_path_file_uri() {
        let uri: lsp_types::Uri = "file:///tmp/test.rs".parse().unwrap();
        let path = uri_to_path(&uri);
        assert_eq!(path, "/tmp/test.rs");
    }

    #[test]
    fn test_uri_to_path_non_file_uri() {
        let uri: lsp_types::Uri = "https://example.com/test".parse().unwrap();
        let path = uri_to_path(&uri);
        assert_eq!(path, "https://example.com/test");
    }

    #[test]
    fn test_parse_definition_scalar() {
        use lsp_types::{Location, Position, Range};

        let uri: lsp_types::Uri = "file:///tmp/test.rs".parse().unwrap();
        let location = Location {
            uri,
            range: Range {
                start: Position::new(10, 5),
                end: Position::new(10, 15),
            },
        };
        let response = GotoDefinitionResponse::Scalar(location);

        let result = parse_definition(&response);
        assert!(result.is_some());
        let def = result.unwrap();
        assert_eq!(def.file_path, "/tmp/test.rs");
        assert_eq!(def.line, 10);
        assert_eq!(def.column, 5);
    }

    #[test]
    fn test_parse_definition_empty_array() {
        let response = GotoDefinitionResponse::Array(vec![]);
        let result = parse_definition(&response);
        assert!(result.is_none());
    }

    fn test_parser(decl: &str) -> Option<String> {
        if decl.contains(':') {
            Some(decl.split(':').nth(1)?.trim().to_string())
        } else {
            None
        }
    }

    #[test]
    fn test_parse_hover_type_with_parser() {
        use lsp_types::{MarkupContent, MarkupKind};

        let hover = Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: "```rust\nlet x: i32\n```".to_string(),
            }),
            range: None,
        };

        let result = parse_hover_type(&hover, "rust", test_parser);
        assert_eq!(result, Some("i32".to_string()));
    }

    #[test]
    fn test_base_enricher_creation() {
        let config = LspServerConfig {
            enabled: true,
            server: "test-server".to_string(),
            args: vec![],
        };

        let enricher = BaseEnricher::new(config, Language::Rust, "rust");
        assert_eq!(enricher.language(), Language::Rust);
        assert_eq!(enricher.language_id(), "rust");
        assert!(enricher.is_enabled());
    }

    #[test]
    fn test_base_enricher_disabled() {
        let config = LspServerConfig {
            enabled: false,
            server: "test-server".to_string(),
            args: vec![],
        };

        let enricher = BaseEnricher::new(config, Language::Python, "python");
        assert!(!enricher.is_enabled());
    }
}
