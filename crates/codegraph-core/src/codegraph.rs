//! Main CodeGraph struct - the public API entry point

use crate::config::CodeGraphConfig;
use crate::error::CodeGraphError;
use codegraph_context::{ContextBuilder, ContextOptions, ContextResult};
use codegraph_db::{DatabaseConnection, QueryBuilder};
use codegraph_extraction::{ExtractionOrchestrator, IndexResult as ExtractionIndexResult};
use codegraph_graph::{GraphQueryManager, GraphTraverser, ImpactRadius};
use codegraph_resolution::ReferenceResolver;
use codegraph_types::{Config, Node, NodeKind, SearchResult};
use codegraph_vectors::{EmbedderConfig, SimilarityResult, TextEmbedder, VectorError, VectorStorage};
use std::fs;
use std::path::{Path, PathBuf};

/// Main CodeGraph API
///
/// This is the primary entry point for using CodeGraph. It provides methods for:
/// - Initializing and opening projects
/// - Indexing code and building the knowledge graph
/// - Searching and traversing the graph
/// - Building context for AI assistants
pub struct CodeGraph {
    /// Configuration
    config: CodeGraphConfig,
    /// Database connection
    db: DatabaseConnection,
    /// Query builder (prepared statements)
    queries: QueryBuilder,
    /// Whether the project is initialized
    initialized: bool,
}

impl CodeGraph {
    /// Initialize a new CodeGraph project at the given path
    ///
    /// Creates the `.codegraph` directory and database if they don't exist.
    pub fn init(path: impl AsRef<Path>) -> Result<Self, CodeGraphError> {
        let root = path.as_ref().canonicalize().map_err(|_| {
            CodeGraphError::InvalidPath(path.as_ref().to_path_buf())
        })?;

        let config = CodeGraphConfig::new(root);

        // Create .codegraph directory
        if !config.data_dir.exists() {
            fs::create_dir_all(&config.data_dir)?;
        }

        // Open/create database
        let db = DatabaseConnection::open(&config.db_path)?;
        let queries = QueryBuilder::new(db.conn())?;

        log::info!("Initialized CodeGraph at {:?}", config.root);

        Ok(Self {
            config,
            db,
            queries,
            initialized: true,
        })
    }

    /// Open an existing CodeGraph project
    ///
    /// Returns an error if the project is not initialized.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CodeGraphError> {
        let root = path.as_ref().canonicalize().map_err(|_| {
            CodeGraphError::InvalidPath(path.as_ref().to_path_buf())
        })?;

        let config = CodeGraphConfig::new(root.clone());

        if !config.data_dir.exists() {
            return Err(CodeGraphError::NotInitialized(root));
        }

        let db = DatabaseConnection::open(&config.db_path)?;
        let queries = QueryBuilder::new(db.conn())?;

        Ok(Self {
            config,
            db,
            queries,
            initialized: true,
        })
    }

    /// Create CodeGraph with in-memory database (for testing)
    pub fn in_memory() -> Result<Self, CodeGraphError> {
        let db = DatabaseConnection::open_in_memory()?;
        let queries = QueryBuilder::new(db.conn())?;

        Ok(Self {
            config: CodeGraphConfig::new(PathBuf::from(".")),
            db,
            queries,
            initialized: true,
        })
    }

    /// Create CodeGraph with custom configuration
    pub fn with_config(config: CodeGraphConfig) -> Result<Self, CodeGraphError> {
        if !config.data_dir.exists() {
            fs::create_dir_all(&config.data_dir)?;
        }

        let db = DatabaseConnection::open(&config.db_path)?;
        let queries = QueryBuilder::new(db.conn())?;

        Ok(Self {
            config,
            db,
            queries,
            initialized: true,
        })
    }

    // ========== Indexing ==========

    /// Index all files in the project
    ///
    /// This scans the project, extracts AST information, and stores it in the database.
    /// Call this after `init()` or when you want to rebuild the index.
    pub fn index_all(&mut self) -> Result<IndexingResult, CodeGraphError> {
        use codegraph_types::FileRecord;
        use std::time::{SystemTime, UNIX_EPOCH};

        let extraction_config = Config {
            root_dir: self.config.root.to_string_lossy().to_string(),
            exclude: self.config.exclude_patterns.clone(),
            max_file_size: self.config.max_file_size as u64,
            ..Config::default()
        };

        // Create orchestrator
        let orchestrator = ExtractionOrchestrator::new(&self.config.root, extraction_config.clone())?;
        let mut nodes_created = 0;
        let mut edges_created = 0;
        let mut files_indexed = 0;

        // Scan and extract files individually for storage
        let scan_result = codegraph_extraction::FileScanner::new(
            &self.config.root,
            &extraction_config,
        )?
        .scan()?;

        log::info!("Found {} files to index", scan_result.files.len());

        // Clear previous unresolved refs before re-indexing
        self.queries.clear_unresolved_refs(self.db.conn())?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        for file in &scan_result.files {
            match orchestrator.extract_file(file) {
                Ok(extraction_result) => {
                    let node_count = extraction_result.nodes.len();

                    // Insert nodes (upsert semantics)
                    for node in &extraction_result.nodes {
                        self.queries.insert_node(self.db.conn(), node)?;
                        nodes_created += 1;
                    }

                    // Insert edges
                    for edge in &extraction_result.edges {
                        self.queries.insert_edge(self.db.conn(), edge)?;
                        edges_created += 1;
                    }

                    // Store unresolved references for later resolution
                    for ref_info in &extraction_result.unresolved_references {
                        let _ = self.queries.insert_unresolved_ref(self.db.conn(), ref_info);
                    }

                    // Track file in files table
                    let file_record = FileRecord {
                        path: file.path.clone(),
                        content_hash: file.content_hash.clone(),
                        language: file.language,
                        size: file.size,
                        modified_at: now,
                        indexed_at: now,
                        node_count: node_count as u32,
                        errors: extraction_result.errors.clone(),
                    };
                    self.queries.upsert_file(self.db.conn(), &file_record)?;
                    files_indexed += 1;
                }
                Err(e) => {
                    log::warn!("Failed to extract {}: {}", file.path, e);
                }
            }
        }

        // Resolve references if enabled
        let mut resolved_count = 0;
        if self.config.resolve_references {
            let mut resolver = ReferenceResolver::new(self.db.conn(), &mut self.queries);
            let stats = resolver.resolve_all()?;
            resolved_count = stats.resolved;
            log::info!(
                "Resolved {} references ({} ambiguous, {} unresolved)",
                stats.resolved,
                stats.ambiguous,
                stats.unresolved
            );
        }

        // Generate embeddings if model is available
        let embeddings_generated = self.generate_embeddings()?;

        let stats = IndexingResult {
            files_indexed,
            nodes_created,
            edges_created,
            references_resolved: resolved_count,
            embeddings_generated,
        };

        log::info!(
            "Indexing complete: {} files, {} nodes, {} edges, {} embeddings",
            stats.files_indexed,
            stats.nodes_created,
            stats.edges_created,
            stats.embeddings_generated,
        );

        Ok(stats)
    }

    /// Get the extraction index result without storing
    pub fn index_all_dry_run(&self) -> Result<ExtractionIndexResult, CodeGraphError> {
        let orchestrator = ExtractionOrchestrator::with_defaults(&self.config.root)?;
        let result = orchestrator.index_all(|_progress| {})?;
        Ok(result)
    }

    /// Incrementally sync changes since last index
    ///
    /// This detects files that have been added, modified, or deleted since the last
    /// index/sync operation and updates the database accordingly. Much faster than
    /// a full re-index for small changes.
    pub fn sync(&mut self) -> Result<codegraph_sync::SyncResult, CodeGraphError> {
        use codegraph_sync::{SyncConfig, SyncManager};

        let sync_config = SyncConfig {
            excludes: self.config.exclude_patterns.clone(),
            continue_on_error: true,
            ..SyncConfig::default()
        };

        let manager = SyncManager::with_config(
            self.config.root.to_string_lossy().to_string(),
            sync_config,
        );

        let result = manager.sync_with_codegraph_dir(
            self.db.conn(),
            &mut self.queries,
            Some(&self.config.data_dir),
        )?;

        // Resolve references for changed files if enabled
        if self.config.resolve_references && result.had_changes {
            let mut resolver = ReferenceResolver::new(self.db.conn(), &mut self.queries);
            let _ = resolver.resolve_all();
        }

        Ok(result)
    }

    // ========== Search ==========

    /// Search for symbols by name
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, CodeGraphError> {
        let results = self.queries.search_nodes(
            self.db.conn(),
            query,
            None,
            None,
            limit,
            0,
        )?;
        Ok(results)
    }

    /// Search for symbols by name with kind filter
    pub fn search_by_kind(
        &mut self,
        query: &str,
        kind: NodeKind,
        limit: usize,
    ) -> Result<Vec<SearchResult>, CodeGraphError> {
        let kinds = [kind];
        let results = self.queries.search_nodes(
            self.db.conn(),
            query,
            Some(&kinds),
            None,
            limit,
            0,
        )?;
        Ok(results)
    }

    /// Get a node by ID
    pub fn get_node(&mut self, node_id: &str) -> Result<Option<Node>, CodeGraphError> {
        let node = self.queries.get_node_by_id(self.db.conn(), node_id)?;
        Ok(node)
    }

    /// Get all nodes in a file
    pub fn get_nodes_in_file(&self, file_path: &str) -> Result<Vec<Node>, CodeGraphError> {
        let nodes = self.queries.get_nodes_by_file(self.db.conn(), file_path)?;
        Ok(nodes)
    }

    // ========== Graph Traversal ==========

    /// Get callers of a function/method
    pub fn get_callers(&mut self, node_id: &str) -> Result<Vec<Node>, CodeGraphError> {
        let mut traverser = GraphTraverser::new(self.db.conn(), &mut self.queries);
        let callers = traverser.get_callers(node_id)?;
        Ok(callers)
    }

    /// Get callees of a function/method
    pub fn get_callees(&mut self, node_id: &str) -> Result<Vec<Node>, CodeGraphError> {
        let mut traverser = GraphTraverser::new(self.db.conn(), &mut self.queries);
        let callees = traverser.get_callees(node_id)?;
        Ok(callees)
    }

    /// Get the call graph for a node (what it calls and what calls it)
    pub fn get_call_graph(&mut self, node_id: &str) -> Result<codegraph_graph::CallGraph, CodeGraphError> {
        let mut manager = GraphQueryManager::new(self.db.conn(), &mut self.queries);
        let graph = manager.build_call_graph(&[node_id])?;
        Ok(graph)
    }

    /// Get the impact radius of changing a node
    pub fn get_impact_radius(&mut self, node_id: &str, max_depth: u32) -> Result<ImpactRadius, CodeGraphError> {
        let mut manager = GraphQueryManager::new(self.db.conn(), &mut self.queries);
        let impact = manager.get_impact_radius(node_id, max_depth)?;
        Ok(impact)
    }

    // ========== Context Building ==========

    /// Build context for a query/task
    pub fn build_context(&mut self, query: &str) -> Result<ContextResult, CodeGraphError> {
        let options = ContextOptions::default();
        let mut builder = ContextBuilder::with_options(self.db.conn(), &mut self.queries, options);
        let result = builder.build_for_query(query)?;
        Ok(result)
    }

    /// Build context with custom options
    pub fn build_context_with_options(
        &mut self,
        query: &str,
        options: ContextOptions,
    ) -> Result<ContextResult, CodeGraphError> {
        let mut builder = ContextBuilder::with_options(self.db.conn(), &mut self.queries, options);
        let result = builder.build_for_query(query)?;
        Ok(result)
    }

    /// Build context for a specific node
    pub fn build_context_for_node(&mut self, node_id: &str, query: &str) -> Result<ContextResult, CodeGraphError> {
        let options = ContextOptions::default();
        let mut builder = ContextBuilder::with_options(self.db.conn(), &mut self.queries, options);
        let result = builder.build_for_node(node_id, query)?;
        Ok(result)
    }

    // ========== Embedding Generation ==========

    /// Node kinds that should get embeddings for semantic search
    const EMBEDDABLE_KINDS: &'static [NodeKind] = &[
        NodeKind::Function,
        NodeKind::Method,
        NodeKind::Class,
        NodeKind::Struct,
        NodeKind::Interface,
        NodeKind::Trait,
        NodeKind::Enum,
        NodeKind::Module,
    ];

    /// Generate embeddings for all embeddable nodes
    ///
    /// Tries to load the ONNX model. If the model is not found or the ONNX
    /// feature is not enabled, logs a message and returns 0 (graceful skip).
    fn generate_embeddings(&mut self) -> Result<usize, CodeGraphError> {
        let mut embedder = TextEmbedder::new(EmbedderConfig::default());
        match embedder.load() {
            Ok(()) => {}
            Err(VectorError::ModelNotFound { .. }) => {
                log::info!("Embedding model not found, skipping embedding generation. \
                    Place nomic-embed-text-v1.5.onnx in .codegraph/models/ or ~/.codegraph/models/");
                return Ok(0);
            }
            Err(VectorError::FeatureNotEnabled { .. }) => {
                log::debug!("ONNX feature not enabled, skipping embedding generation");
                return Ok(0);
            }
            Err(e) => return Err(CodeGraphError::Vector(e)),
        }

        // Initialize vectors table and clear stale vectors from previous index
        let dimension = embedder.dimension();
        let storage = VectorStorage::new(dimension);
        storage.init(self.db.conn())?;
        storage.clear(self.db.conn())?;

        let model_name = "nomic-embed-text-v1.5";
        let mut count = 0;

        for kind in Self::EMBEDDABLE_KINDS {
            let nodes = self.queries.get_nodes_by_kind(self.db.conn(), *kind)?;

            for node in &nodes {
                // Build rich embedding text with graph context
                let text = match crate::embedding::build_embedding_text(
                    self.db.conn(),
                    &mut self.queries,
                    node,
                    &self.config.embedding,
                ) {
                    Ok(text) => text,
                    Err(e) => {
                        log::debug!("Failed to build embedding text for {}: {}", node.id.0, e);
                        continue;
                    }
                };

                // Generate embedding with document prefix
                let embedding = match embedder.embed_document(&text) {
                    Ok(e) => e,
                    Err(e) => {
                        log::warn!("Failed to embed {}: {}", node.id.0, e);
                        continue;
                    }
                };

                // Store embedding
                storage.store(self.db.conn(), &node.id.0, &embedding, model_name)?;
                count += 1;
            }
        }

        log::info!("Generated {} embeddings", count);
        Ok(count)
    }

    // ========== Vector/Semantic Search ==========

    /// Store embedding for a node
    pub fn store_embedding(&self, node_id: &str, embedding: &[f32], model: &str) -> Result<(), CodeGraphError> {
        let dimension = embedding.len();
        let storage = VectorStorage::new(dimension);
        storage.store(self.db.conn(), node_id, embedding, model)?;
        Ok(())
    }

    /// Search by embedding vector (manual search without embedder)
    pub fn search_by_vector(&self, query_vector: &[f32], limit: usize) -> Result<Vec<SimilarityResult>, CodeGraphError> {
        let dimension = query_vector.len();
        let storage = VectorStorage::new(dimension);
        let all_vectors = storage.get_all(self.db.conn())?;

        let mut results: Vec<SimilarityResult> = all_vectors
            .into_iter()
            .map(|record| {
                let score = codegraph_vectors::cosine_similarity(query_vector, &record.vector);
                SimilarityResult {
                    node_id: record.node_id,
                    score,
                }
            })
            .collect();

        // Sort by score descending
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        Ok(results)
    }

    /// Semantic search using embeddings
    ///
    /// Loads the embedding model, embeds the query with `search_query:` prefix,
    /// and finds the most similar nodes.
    ///
    /// Returns `None` if embeddings are unavailable (no vectors, no model, ONNX
    /// feature disabled). Returns `Some(vec)` with results (possibly empty if
    /// no nodes exceed the similarity threshold).
    pub fn semantic_search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Option<Vec<SimilarityResult>>, CodeGraphError> {
        use codegraph_vectors::SimilaritySearch;

        let dimension = EmbedderConfig::default().dimension;
        let storage = VectorStorage::new(dimension);

        // Check if vectors table has data
        let count = storage.count(self.db.conn()).unwrap_or(0);
        if count == 0 {
            return Ok(None); // Embeddings not available
        }

        let mut embedder = TextEmbedder::new(EmbedderConfig::default());
        match embedder.load() {
            Ok(()) => {}
            Err(VectorError::ModelNotFound { .. } | VectorError::FeatureNotEnabled { .. }) => {
                return Ok(None); // Model not available
            }
            Err(e) => return Err(CodeGraphError::Vector(e)),
        }

        let config = codegraph_vectors::search::SearchConfig {
            max_results: limit,
            min_score: 0.3, // Reasonable threshold for code search
        };
        let mut search = SimilaritySearch::with_config(&storage, &mut embedder, config);
        let results = search.search_by_text(self.db.conn(), query)?;
        Ok(Some(results))
    }

    /// Build context using semantic search as primary, FTS as fallback
    ///
    /// 1. Try semantic search to find the most relevant seed nodes
    /// 2. If semantic search returns results, build context around those seeds
    /// 3. If embeddings unavailable (None), fall back to FTS-based context
    /// 4. If embeddings available but no hits (Some(empty)), still fall back to FTS
    pub fn build_context_semantic(
        &mut self,
        query: &str,
    ) -> Result<ContextResult, CodeGraphError> {
        self.build_context_semantic_with_options(query, ContextOptions::default())
    }

    /// Build context using semantic search with custom options
    pub fn build_context_semantic_with_options(
        &mut self,
        query: &str,
        options: ContextOptions,
    ) -> Result<ContextResult, CodeGraphError> {
        // Try semantic search first
        if let Some(semantic_results) = self.semantic_search(query, options.max_nodes)? {
            if !semantic_results.is_empty() {
                // Use semantic results as seeds
                let seed_ids: Vec<&str> = semantic_results
                    .iter()
                    .map(|r| r.node_id.as_str())
                    .collect();

                let mut builder = ContextBuilder::with_options(self.db.conn(), &mut self.queries, options);
                let result = builder.build_for_seed_nodes(&seed_ids, query)?;
                return Ok(result);
            }
        }

        // Fall back to FTS-based context (embeddings unavailable OR no semantic hits)
        let mut builder = ContextBuilder::with_options(self.db.conn(), &mut self.queries, options);
        let result = builder.build_for_query(query)?;
        Ok(result)
    }

    // ========== Statistics ==========

    /// Get project statistics
    pub fn get_stats(&self) -> Result<ProjectStats, CodeGraphError> {
        // Count nodes
        let node_count: usize = self.db.conn()
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
            .unwrap_or(0);

        // Count edges
        let edge_count: usize = self.db.conn()
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
            .unwrap_or(0);

        // Count unique files
        let file_count: usize = self.db.conn()
            .query_row("SELECT COUNT(DISTINCT file_path) FROM nodes", [], |row| row.get(0))
            .unwrap_or(0);

        Ok(ProjectStats {
            node_count,
            edge_count,
            file_count,
        })
    }

    // ========== Accessors ==========

    /// Get the configuration
    pub fn config(&self) -> &CodeGraphConfig {
        &self.config
    }

    /// Get the database connection
    pub fn conn(&self) -> &codegraph_db::rusqlite::Connection {
        self.db.conn()
    }

    /// Get the query builder
    pub fn queries(&self) -> &QueryBuilder {
        &self.queries
    }

    /// Get mutable query builder
    pub fn queries_mut(&mut self) -> &mut QueryBuilder {
        &mut self.queries
    }

    /// Check if the project is initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Get the project root path
    pub fn root(&self) -> &Path {
        &self.config.root
    }
}

/// Result of indexing operation
#[derive(Debug, Clone)]
pub struct IndexingResult {
    /// Number of files indexed
    pub files_indexed: usize,
    /// Number of nodes created
    pub nodes_created: usize,
    /// Number of edges created
    pub edges_created: usize,
    /// Number of references resolved
    pub references_resolved: usize,
    /// Number of embeddings generated
    pub embeddings_generated: usize,
}

/// Project statistics
#[derive(Debug, Clone)]
pub struct ProjectStats {
    /// Total number of nodes
    pub node_count: usize,
    /// Total number of edges
    pub edge_count: usize,
    /// Total number of files
    pub file_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_memory_creation() {
        let cg = CodeGraph::in_memory().unwrap();
        assert!(cg.is_initialized());
    }

    #[test]
    fn test_stats_empty() {
        let cg = CodeGraph::in_memory().unwrap();
        let stats = cg.get_stats().unwrap();
        assert_eq!(stats.node_count, 0);
        assert_eq!(stats.edge_count, 0);
    }

    #[test]
    fn test_search_empty() {
        let cg = CodeGraph::in_memory().unwrap();
        let results = cg.search("test", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_node_not_found() {
        let mut cg = CodeGraph::in_memory().unwrap();
        let node = cg.get_node("nonexistent").unwrap();
        assert!(node.is_none());
    }

    #[test]
    fn test_init_creates_directory() {
        let temp = tempfile::tempdir().unwrap();
        let project_path = temp.path().join("test_project");
        fs::create_dir(&project_path).unwrap();

        let cg = CodeGraph::init(&project_path).unwrap();

        assert!(cg.config.data_dir.exists());
        assert!(cg.config.db_path.exists());
    }

    #[test]
    fn test_open_not_initialized() {
        let temp = tempfile::tempdir().unwrap();
        let project_path = temp.path().join("uninitialized");
        fs::create_dir(&project_path).unwrap();

        let result = CodeGraph::open(&project_path);
        assert!(matches!(result, Err(CodeGraphError::NotInitialized(_))));
    }

    #[test]
    fn test_init_then_open() {
        let temp = tempfile::tempdir().unwrap();
        let project_path = temp.path().join("test_project2");
        fs::create_dir(&project_path).unwrap();

        // Initialize
        let _cg1 = CodeGraph::init(&project_path).unwrap();

        // Open
        let cg2 = CodeGraph::open(&project_path).unwrap();
        assert!(cg2.is_initialized());
    }

    #[test]
    fn test_insert_and_search() {
        let cg = CodeGraph::in_memory().unwrap();

        // Insert a node
        let node = Node::new(
            "test_id",
            NodeKind::Function,
            "myTestFunction",
            "test::myTestFunction",
            "test.rs",
            codegraph_types::Language::Rust,
            1,
            10,
        );
        cg.queries.insert_node(cg.conn(), &node).unwrap();

        // Search for it
        let results = cg.search("myTestFunction", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node.name, "myTestFunction");
    }
}
