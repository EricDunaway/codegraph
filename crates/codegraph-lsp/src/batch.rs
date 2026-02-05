//! Batch LSP enrichment with dependency tracking
//!
//! This module provides batch enrichment functionality that:
//! - Enriches multiple nodes concurrently using the async LSP client (I5)
//! - Tracks which files each node depends on for incremental updates (I2)
//! - Returns enrichment results with dependencies to record

use crate::enricher::{DefinitionResult, HoverResult, LspEnricher};
use crate::error::LspError;
use std::collections::HashSet;
use std::path::Path;

/// Result of enriching a single node
#[derive(Debug, Clone, Default)]
pub struct NodeEnrichmentResult {
    /// Inferred type from hover query
    pub inferred_type: Option<String>,
    /// Documentation from hover query
    pub documentation: Option<String>,
    /// Resolved import path from definition query
    pub resolved_import_path: Option<String>,
    /// Files this node depends on (for enrichment_deps tracking)
    pub deps_to_record: Vec<String>,
}

impl NodeEnrichmentResult {
    /// Create a new empty result
    pub fn new() -> Self {
        Self::default()
    }

    /// Create from hover result
    pub fn from_hover(hover: HoverResult) -> Self {
        Self {
            inferred_type: hover.inferred_type,
            documentation: hover.documentation,
            ..Default::default()
        }
    }

    /// Add definition result and track dependency
    pub fn with_definition(mut self, def: DefinitionResult, source_file: &str) -> Self {
        // Only record dependency if definition is in a different file
        if def.file_path != source_file {
            self.deps_to_record.push(def.file_path.clone());
        }
        self.resolved_import_path = Some(def.file_path);
        self
    }

    /// Merge another result into this one
    pub fn merge(&mut self, other: NodeEnrichmentResult) {
        if self.inferred_type.is_none() {
            self.inferred_type = other.inferred_type;
        }
        if self.documentation.is_none() {
            self.documentation = other.documentation;
        }
        if self.resolved_import_path.is_none() {
            self.resolved_import_path = other.resolved_import_path;
        }
        // Merge deps, avoiding duplicates
        let existing: HashSet<_> = self.deps_to_record.iter().cloned().collect();
        for dep in other.deps_to_record {
            if !existing.contains(&dep) {
                self.deps_to_record.push(dep);
            }
        }
    }
}

/// Position in a file for LSP queries
#[derive(Debug, Clone, Copy)]
pub struct FilePosition {
    pub line: u32,
    pub column: u32,
}

/// Request to enrich a node
#[derive(Debug, Clone)]
pub struct EnrichmentRequest {
    /// Node ID for tracking
    pub node_id: String,
    /// File containing the node
    pub file_path: String,
    /// Position for hover query (typically the symbol name)
    pub hover_position: Option<FilePosition>,
    /// Position for definition query (typically an import)
    pub definition_position: Option<FilePosition>,
}

impl EnrichmentRequest {
    /// Create a request for hover enrichment only
    pub fn hover_only(node_id: impl Into<String>, file_path: impl Into<String>, line: u32, column: u32) -> Self {
        Self {
            node_id: node_id.into(),
            file_path: file_path.into(),
            hover_position: Some(FilePosition { line, column }),
            definition_position: None,
        }
    }

    /// Create a request for definition enrichment only
    pub fn definition_only(node_id: impl Into<String>, file_path: impl Into<String>, line: u32, column: u32) -> Self {
        Self {
            node_id: node_id.into(),
            file_path: file_path.into(),
            hover_position: None,
            definition_position: Some(FilePosition { line, column }),
        }
    }

    /// Create a request for both hover and definition
    pub fn full(
        node_id: impl Into<String>,
        file_path: impl Into<String>,
        hover_line: u32,
        hover_column: u32,
        def_line: u32,
        def_column: u32,
    ) -> Self {
        Self {
            node_id: node_id.into(),
            file_path: file_path.into(),
            hover_position: Some(FilePosition { line: hover_line, column: hover_column }),
            definition_position: Some(FilePosition { line: def_line, column: def_column }),
        }
    }
}

/// Result of a batch enrichment operation
#[derive(Debug)]
pub struct BatchEnrichmentResult {
    /// Results keyed by node ID
    pub results: Vec<(String, Result<NodeEnrichmentResult, LspError>)>,
    /// Total number of successful enrichments
    pub success_count: usize,
    /// Total number of failed enrichments
    pub failure_count: usize,
}

impl BatchEnrichmentResult {
    /// Get successful results only
    pub fn successful(&self) -> impl Iterator<Item = (&str, &NodeEnrichmentResult)> {
        self.results.iter().filter_map(|(id, r)| {
            r.as_ref().ok().map(|result| (id.as_str(), result))
        })
    }

    /// Get failed results only
    pub fn failed(&self) -> impl Iterator<Item = (&str, &LspError)> {
        self.results.iter().filter_map(|(id, r)| {
            r.as_ref().err().map(|err| (id.as_str(), err))
        })
    }
}

/// Enrich a single node using an LSP enricher
pub async fn enrich_node<E: LspEnricher>(
    enricher: &E,
    request: &EnrichmentRequest,
) -> Result<NodeEnrichmentResult, LspError> {
    let file_path = Path::new(&request.file_path);
    let mut result = NodeEnrichmentResult::new();

    // Execute hover query if requested
    if let Some(pos) = request.hover_position {
        if let Some(hover) = enricher.hover(file_path, pos.line, pos.column).await? {
            result.inferred_type = hover.inferred_type;
            result.documentation = hover.documentation;
        }
    }

    // Execute definition query if requested
    if let Some(pos) = request.definition_position {
        if let Some(def) = enricher.definition(file_path, pos.line, pos.column).await? {
            // Track dependency if definition is in a different file
            if def.file_path != request.file_path {
                result.deps_to_record.push(def.file_path.clone());
            }
            result.resolved_import_path = Some(def.file_path);
        }
    }

    Ok(result)
}

/// Default concurrency limit to prevent memory exhaustion
const DEFAULT_CONCURRENCY: usize = 50;

/// Enrich multiple nodes concurrently (I5: single LSP handles parallel requests)
///
/// This function leverages the async concurrent LSP client to execute
/// multiple enrichment requests in parallel, maximizing throughput while
/// limiting memory usage via bounded concurrency.
pub async fn enrich_batch<E: LspEnricher>(
    enricher: &E,
    requests: Vec<EnrichmentRequest>,
) -> BatchEnrichmentResult {
    enrich_batch_with_concurrency(enricher, requests, DEFAULT_CONCURRENCY).await
}

/// Enrich multiple nodes with a custom concurrency limit
///
/// Use this when you need to control the level of parallelism.
///
/// # Panics
/// Panics if `concurrency` is 0 (would cause hang with buffer_unordered).
pub async fn enrich_batch_with_concurrency<E: LspEnricher>(
    enricher: &E,
    requests: Vec<EnrichmentRequest>,
    concurrency: usize,
) -> BatchEnrichmentResult {
    assert!(concurrency > 0, "concurrency must be at least 1");
    use futures::stream::{self, StreamExt};

    let results: Vec<_> = stream::iter(requests)
        .map(|req| async {
            let result = enrich_node(enricher, &req).await;
            (req.node_id, result)
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    let success_count = results.iter().filter(|(_, r)| r.is_ok()).count();
    let failure_count = results.len() - success_count;

    BatchEnrichmentResult {
        results,
        success_count,
        failure_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_enrichment_result_from_hover() {
        let hover = HoverResult {
            inferred_type: Some("string".to_string()),
            documentation: Some("A test function".to_string()),
        };

        let result = NodeEnrichmentResult::from_hover(hover);
        assert_eq!(result.inferred_type, Some("string".to_string()));
        assert_eq!(result.documentation, Some("A test function".to_string()));
        assert!(result.resolved_import_path.is_none());
        assert!(result.deps_to_record.is_empty());
    }

    #[test]
    fn test_node_enrichment_result_with_definition_same_file() {
        let result = NodeEnrichmentResult::new();
        let def = DefinitionResult {
            file_path: "src/api.ts".to_string(),
            line: 10,
            column: 5,
        };

        let result = result.with_definition(def, "src/api.ts");

        // Same file - no dependency recorded
        assert!(result.deps_to_record.is_empty());
        assert_eq!(result.resolved_import_path, Some("src/api.ts".to_string()));
    }

    #[test]
    fn test_node_enrichment_result_with_definition_different_file() {
        let result = NodeEnrichmentResult::new();
        let def = DefinitionResult {
            file_path: "src/types.ts".to_string(),
            line: 5,
            column: 0,
        };

        let result = result.with_definition(def, "src/api.ts");

        // Different file - dependency recorded
        assert_eq!(result.deps_to_record.len(), 1);
        assert!(result.deps_to_record.contains(&"src/types.ts".to_string()));
        assert_eq!(result.resolved_import_path, Some("src/types.ts".to_string()));
    }

    #[test]
    fn test_node_enrichment_result_merge() {
        let mut result1 = NodeEnrichmentResult {
            inferred_type: Some("string".to_string()),
            documentation: None,
            resolved_import_path: None,
            deps_to_record: vec!["a.ts".to_string()],
        };

        let result2 = NodeEnrichmentResult {
            inferred_type: Some("number".to_string()), // Should not overwrite
            documentation: Some("Docs".to_string()),
            resolved_import_path: Some("b.ts".to_string()),
            deps_to_record: vec!["a.ts".to_string(), "c.ts".to_string()],
        };

        result1.merge(result2);

        assert_eq!(result1.inferred_type, Some("string".to_string())); // Not overwritten
        assert_eq!(result1.documentation, Some("Docs".to_string())); // Merged
        assert_eq!(result1.resolved_import_path, Some("b.ts".to_string())); // Merged
        assert_eq!(result1.deps_to_record.len(), 2); // Deduplicated
        assert!(result1.deps_to_record.contains(&"a.ts".to_string()));
        assert!(result1.deps_to_record.contains(&"c.ts".to_string()));
    }

    #[test]
    fn test_enrichment_request_hover_only() {
        let req = EnrichmentRequest::hover_only("node_1", "src/test.ts", 10, 5);

        assert_eq!(req.node_id, "node_1");
        assert_eq!(req.file_path, "src/test.ts");
        assert!(req.hover_position.is_some());
        assert!(req.definition_position.is_none());
    }

    #[test]
    fn test_enrichment_request_definition_only() {
        let req = EnrichmentRequest::definition_only("node_1", "src/test.ts", 10, 5);

        assert_eq!(req.node_id, "node_1");
        assert!(req.hover_position.is_none());
        assert!(req.definition_position.is_some());
    }

    #[test]
    fn test_enrichment_request_full() {
        let req = EnrichmentRequest::full("node_1", "src/test.ts", 10, 5, 1, 0);

        assert!(req.hover_position.is_some());
        assert!(req.definition_position.is_some());

        let hover_pos = req.hover_position.unwrap();
        assert_eq!(hover_pos.line, 10);
        assert_eq!(hover_pos.column, 5);

        let def_pos = req.definition_position.unwrap();
        assert_eq!(def_pos.line, 1);
        assert_eq!(def_pos.column, 0);
    }

    #[test]
    fn test_batch_result_filtering() {
        let results = BatchEnrichmentResult {
            results: vec![
                ("node_1".to_string(), Ok(NodeEnrichmentResult::new())),
                ("node_2".to_string(), Err(LspError::Timeout("test".to_string()))),
                ("node_3".to_string(), Ok(NodeEnrichmentResult::new())),
            ],
            success_count: 2,
            failure_count: 1,
        };

        let successful: Vec<_> = results.successful().collect();
        assert_eq!(successful.len(), 2);

        let failed: Vec<_> = results.failed().collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, "node_2");
    }
}
