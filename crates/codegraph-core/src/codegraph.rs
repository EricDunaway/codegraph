//! Main CodeGraph struct - the public API entry point

use crate::config::CodeGraphConfig;
use crate::error::CodeGraphError;
use codegraph_context::{ContextBuilder, ContextOptions, ContextResult};
use codegraph_db::{DatabaseConnection, QueryBuilder};
use codegraph_extraction::{ExtractionOrchestrator, IndexResult as ExtractionIndexResult};
use codegraph_graph::{GraphQueryManager, GraphTraverser, ImpactRadius};
use codegraph_resolution::ReferenceResolver;
use codegraph_sync::IndexLock;
use codegraph_types::{Config, Node, NodeKind, SearchResult};
use codegraph_vectors::{EmbedderConfig, SimilarityResult, TextEmbedder, VectorError, VectorStorage};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Options for controlling sync behavior
#[derive(Debug, Default)]
pub struct SyncOptions {
    /// Hook name if triggered by a git hook (enables hook-mode behavior)
    pub hook_name: Option<String>,
    /// Specific file list to sync (overrides detection)
    pub file_list: Option<Vec<String>>,
    /// Enable EdgeSnapshot verification mode
    pub verify_sync: bool,
}

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

        // Auto-add .codegraph/ to .gitignore if inside a git repo
        if config.root.join(".git").exists() {
            if let Err(e) = ensure_codegraph_in_gitignore(&config.root) {
                log::warn!("Failed to update .gitignore: {}", e);
            }
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

        // Acquire lock to prevent concurrent sync during full reindex
        let _lock = if self.config.data_dir.exists() {
            match IndexLock::acquire(&self.config.data_dir) {
                Ok(lock) => Some(lock),
                Err(codegraph_sync::SyncError::LockHeld) => {
                    return Err(CodeGraphError::Sync(
                        codegraph_sync::SyncError::LockHeld,
                    ));
                }
                Err(e) => return Err(CodeGraphError::Sync(e)),
            }
        } else {
            None
        };

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

        // Clear all graph data before full re-index (nodes, edges, files, unresolved_refs)
        self.queries.clear_all_graph_data(self.db.conn())?;

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
    ///
    /// Also handles incremental embedding updates:
    /// - Checks for full re-embed triggers (schema/config/model change)
    /// - Uses ImpactCapture (pre-delete neighbors + siblings) from SyncManager
    /// - Runs scoped reference resolution for changed files only
    /// - Computes embed candidates from impact + resolution results
    /// - Re-embeds new, modified, and ripple-affected nodes
    /// - Deletes vectors for removed nodes
    /// - Falls back to full reindex if >30% of files changed
    ///
    /// Returns a `FullSyncResult` containing both the file sync stats and embedding stats.
    pub fn sync(&mut self) -> Result<FullSyncResult, CodeGraphError> {
        self.sync_with_options(SyncOptions::default())
    }

    /// Incrementally sync changes with custom options.
    ///
    /// Implements the full Phase 0-7 sync pipeline:
    ///
    /// - **Phase 0 (Trigger):** Determine sync mode from `SyncOptions`.
    /// - **Phase 1 (Detect):** Git-diff (hook mode), file list (external), or hash scan (fallback).
    /// - **Phase 2 (Lock):** Acquire `IndexLock`. In hook mode, writes `sync.pending` on collision.
    /// - **Phase 3 (Extract):** Process changes via `SyncManager` (includes ImpactCapture).
    /// - **Phase 4 (Resolve):** Scoped reference resolution for changed files.
    /// - **Phase 5 (Embed):** Compute candidates, run incremental embedding sync.
    /// - **Phase 6 (Checkpoint):** Write `sync.last_head` and `sync.last_timestamp`.
    /// - **Phase 7 (Release + Drain):** Drop lock, drain `sync.pending` (max 3 iterations).
    pub fn sync_with_options(&mut self, options: SyncOptions) -> Result<FullSyncResult, CodeGraphError> {
        use codegraph_sync::{
            checkpoint, GitDiffDetector, PendingSync, ReembedConfig, SyncConfig, SyncManager,
        };

        let is_hook_mode = options.hook_name.is_some();

        // ====== Phase 0: Trigger ======
        // Determine sync mode based on options

        let sync_config = SyncConfig {
            excludes: self.config.exclude_patterns.clone(),
            continue_on_error: true,
            // Disable SyncManager's internal lock — we manage lock externally in Phase 2
            use_lock: false,
            ..SyncConfig::default()
        };

        let manager = SyncManager::with_config(
            self.config.root.to_string_lossy().to_string(),
            sync_config,
        );

        // Build ReembedConfig to detect full re-embed triggers
        let config_json = serde_json::to_string(&self.config.embedding).unwrap_or_default();
        let schema_version = codegraph_db::get_schema_version(self.db.conn())
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "0".to_string());
        let model_id = "nomic-embed-text-v1.5";
        let reembed_config = ReembedConfig {
            schema_version: &schema_version,
            config_json: &config_json,
            model_hash: model_id,
            force: false,
        };
        let needs_full_reembed = codegraph_sync::should_full_reembed(
            self.db.conn(),
            &self.queries,
            &reembed_config,
        );

        // ====== Phase 1: Detect Changes ======
        // Determine which files changed using the appropriate detection strategy

        let use_git_diff = is_hook_mode && self.config.data_dir.exists();
        let mut git_diff_changes: Option<Vec<codegraph_sync::FileChange>> = None;
        let mut fallback_to_hash_scan = false;

        if use_git_diff {
            // Hook mode: use git diff against checkpoint
            let last_head = checkpoint::read_last_head(self.db.conn(), &self.queries);
            let current_head = checkpoint::get_git_head(&self.config.root);

            match (last_head, current_head.as_ref()) {
                (Ok(Some(ref checkpoint_sha)), Some(_current_sha)) => {
                    let detector = GitDiffDetector::new(
                        &self.config.root,
                        &self.config.exclude_patterns,
                    );
                    match detector.detect_changes(checkpoint_sha) {
                        Ok(changes) => {
                            if changes.is_empty() {
                                // No changes detected by git diff — early exit
                                // Still update checkpoint if HEAD moved
                                if let Some(head) = &current_head {
                                    let _ = checkpoint::write_last_head(
                                        self.db.conn(),
                                        &self.queries,
                                        head,
                                    );
                                    let _ = checkpoint::write_last_timestamp(
                                        self.db.conn(),
                                        &self.queries,
                                    );
                                }
                                let sync_result = codegraph_sync::SyncResult {
                                    stats: codegraph_sync::SyncStats::default(),
                                    had_changes: false,
                                    duration_ms: 0,
                                    enrichment_scope: codegraph_sync::SelectiveScope::default(),
                                    deleted_node_ids: Vec::new(),
                                    changed_file_paths: Vec::new(),
                                    pre_delete_impact: codegraph_sync::ImpactCapture::new(),
                                };
                                return Ok(FullSyncResult {
                                    sync: sync_result,
                                    embeddings: EmbeddingSyncResult::default(),
                                });
                            }
                            git_diff_changes = Some(changes);
                        }
                        Err(e) => {
                            log::warn!("Git diff detection failed, falling back to hash scan: {}", e);
                            fallback_to_hash_scan = true;
                        }
                    }
                }
                _ => {
                    // No checkpoint or no git HEAD — fall back to hash scan
                    log::info!("No sync checkpoint found, falling back to hash scan");
                    fallback_to_hash_scan = true;
                }
            }
        }

        // ====== Phase 2: Acquire Lock ======
        // In hook mode, use try_acquire_or_pending to avoid blocking git.
        // In normal mode, acquire the lock (blocking/failing on collision).

        let _lock = if self.config.data_dir.exists() {
            if is_hook_mode {
                let hook_name = options.hook_name.as_deref().unwrap();
                match IndexLock::try_acquire_or_pending(&self.config.data_dir, hook_name) {
                    Ok(Some(lock)) => Some(lock),
                    Ok(None) => {
                        // Lock held, sync.pending written — return early
                        log::info!(
                            "Lock held by another sync, wrote sync.pending for hook '{}'",
                            hook_name
                        );
                        let sync_result = codegraph_sync::SyncResult {
                            stats: codegraph_sync::SyncStats::default(),
                            had_changes: false,
                            duration_ms: 0,
                            enrichment_scope: codegraph_sync::SelectiveScope::default(),
                            deleted_node_ids: Vec::new(),
                            changed_file_paths: Vec::new(),
                            pre_delete_impact: codegraph_sync::ImpactCapture::new(),
                        };
                        return Ok(FullSyncResult {
                            sync: sync_result,
                            embeddings: EmbeddingSyncResult::default(),
                        });
                    }
                    Err(e) => {
                        if is_hook_mode {
                            // Hook mode errors should not crash — write sync.failed and return Ok
                            log::error!("Hook sync lock error: {}", e);
                            self.write_sync_failed(
                                options.hook_name.as_deref().unwrap_or("unknown"),
                                &format!("lock error: {}", e),
                            );
                            let sync_result = codegraph_sync::SyncResult {
                                stats: codegraph_sync::SyncStats::default(),
                                had_changes: false,
                                duration_ms: 0,
                                enrichment_scope: codegraph_sync::SelectiveScope::default(),
                                deleted_node_ids: Vec::new(),
                                changed_file_paths: Vec::new(),
                                pre_delete_impact: codegraph_sync::ImpactCapture::new(),
                            };
                            return Ok(FullSyncResult {
                                sync: sync_result,
                                embeddings: EmbeddingSyncResult::default(),
                            });
                        }
                        return Err(CodeGraphError::Sync(e));
                    }
                }
            } else {
                match IndexLock::acquire(&self.config.data_dir) {
                    Ok(lock) => Some(lock),
                    Err(codegraph_sync::SyncError::LockHeld) => {
                        return Err(CodeGraphError::Sync(codegraph_sync::SyncError::LockHeld));
                    }
                    Err(e) => return Err(CodeGraphError::Sync(e)),
                }
            }
        } else {
            None
        };

        // ====== Verify-sync: pre-snapshot ======
        // Capture a full EdgeSnapshot before extraction so we can compare afterwards.
        let pre_verify_snapshot = if options.verify_sync {
            log::info!("verify-sync: capturing pre-sync EdgeSnapshot");
            Some(codegraph_sync::EdgeSnapshot::capture(self.db.conn()))
        } else {
            None
        };

        // ====== Phase 3: Extract (process changes) ======
        // Use SyncManager to detect (or process pre-detected) changes and update the DB.
        // SyncManager includes ImpactCapture in its process_modify/process_delete.

        let sync_result = if let Some(ref file_list) = options.file_list {
            // External file list mode
            let file_refs: Vec<&str> = file_list.iter().map(|s| s.as_str()).collect();
            manager.sync_files(self.db.conn(), &mut self.queries, &file_refs)?
        } else if git_diff_changes.is_some() && !fallback_to_hash_scan {
            // Hook mode with git diff changes: use SyncManager with the diff-detected files.
            // Convert FileChange paths to file_refs for sync_files (which re-detects changes
            // against the DB under lock, providing revalidation).
            let change_paths: Vec<String> = git_diff_changes
                .as_ref()
                .unwrap()
                .iter()
                .map(|c| c.path.clone())
                .collect();
            let file_refs: Vec<&str> = change_paths.iter().map(|s| s.as_str()).collect();
            manager.sync_files(self.db.conn(), &mut self.queries, &file_refs)?
        } else {
            // Fallback: full hash scan via SyncManager
            manager.sync_with_codegraph_dir(self.db.conn(), &mut self.queries, None)?
        };

        // Refresh lock after Phase 3 extraction
        if let Some(ref lock) = _lock { let _ = lock.refresh(); }

        // Full-reindex fallback: if >30% of tracked files changed, do a full reindex
        if sync_result.had_changes {
            let graph_stats = self.queries.get_stats(self.db.conn())?;
            let total_files = graph_stats.file_count as usize;
            let changed_files = sync_result.changed_file_paths.len();
            if total_files > 0 && changed_files * 100 / total_files > 30 {
                log::warn!(
                    "Large changeset detected: {}/{} files changed ({}%). Running full reindex.",
                    changed_files,
                    total_files,
                    changed_files * 100 / total_files,
                );
                // Drop the lock guard early — index_all will acquire its own lock
                drop(_lock);
                let index_result = self.index_all()?;

                // Update checkpoint after full reindex
                if let Some(head) = checkpoint::get_git_head(&self.config.root) {
                    let _ = checkpoint::write_last_head(self.db.conn(), &self.queries, &head);
                    let _ = checkpoint::write_last_timestamp(self.db.conn(), &self.queries);
                }

                // Build a FullSyncResult from the index result
                let full_sync_result = codegraph_sync::SyncResult {
                    stats: codegraph_sync::SyncStats {
                        files_added: index_result.files_indexed,
                        ..codegraph_sync::SyncStats::default()
                    },
                    had_changes: true,
                    duration_ms: 0,
                    enrichment_scope: codegraph_sync::SelectiveScope::default(),
                    deleted_node_ids: Vec::new(),
                    changed_file_paths: Vec::new(),
                    pre_delete_impact: codegraph_sync::ImpactCapture::new(),
                };
                return Ok(FullSyncResult {
                    sync: full_sync_result,
                    embeddings: EmbeddingSyncResult {
                        vectors_created: index_result.embeddings_generated,
                        full_reembed: true,
                        ..Default::default()
                    },
                });
            }
        }

        // ====== Phase 4: Resolve (scoped reference resolution) ======
        let scoped_resolution = if self.config.resolve_references && sync_result.had_changes {
            let changed_file_refs: Vec<&str> = sync_result
                .changed_file_paths
                .iter()
                .map(|s| s.as_str())
                .collect();
            let mut resolver = ReferenceResolver::new(self.db.conn(), &mut self.queries);
            match resolver.resolve_for_files(&changed_file_refs) {
                Ok(result) => {
                    log::info!(
                        "Scoped resolution: {} resolved ({} source nodes, {} target nodes)",
                        result.stats.resolved,
                        result.source_node_ids.len(),
                        result.target_node_ids.len(),
                    );
                    Some(result)
                }
                Err(e) => {
                    log::warn!("Scoped reference resolution failed: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Refresh lock after Phase 4 resolution
        if let Some(ref lock) = _lock { let _ = lock.refresh(); }

        // ====== Phase 5: Embed (compute candidates, sync embeddings) ======
        // Refresh lock at start of Phase 5 embedding
        if let Some(ref lock) = _lock { let _ = lock.refresh(); }

        let mut embed_result = EmbeddingSyncResult::default();
        let mut embed_candidate_ids: Option<HashSet<String>> = None;
        if sync_result.had_changes || needs_full_reembed {
            if needs_full_reembed {
                // Full re-embed: clear and regenerate all embeddings
                log::info!("Full re-embed triggered, regenerating all embeddings");
                match self.generate_embeddings() {
                    Ok(count) => {
                        embed_result.vectors_created = count;
                        let model_available = {
                            let mut embedder = TextEmbedder::new(EmbedderConfig::default());
                            embedder.load().is_ok()
                        };
                        if model_available {
                            embed_result.full_reembed = true;
                            let _ = codegraph_sync::reembed::record_embed_metadata(
                                self.db.conn(),
                                &self.queries,
                                &reembed_config,
                            );
                        } else {
                            embed_result.skipped_no_model = true;
                        }
                        log::info!("Full re-embed complete: {} vectors generated", count);
                    }
                    Err(e) => {
                        log::warn!("Full re-embed failed: {}", e);
                    }
                }
            } else {
                // Incremental: compute embed candidates from ImpactCapture + ScopedResolutionResult
                let embed_candidates = self.compute_embed_candidates(
                    &sync_result,
                    scoped_resolution.as_ref(),
                )?;
                // Store candidates for --verify-sync comparison
                if options.verify_sync {
                    embed_candidate_ids = Some(embed_candidates.clone());
                }
                match self.sync_embeddings_for_candidates(&sync_result, embed_candidates) {
                    Ok(result) if !result.skipped_no_model => {
                        embed_result = result;
                        let _ = codegraph_sync::reembed::record_embed_metadata(
                            self.db.conn(),
                            &self.queries,
                            &reembed_config,
                        );
                    }
                    Ok(result) => {
                        embed_result = result; // Model unavailable — skip metadata
                    }
                    Err(e) => {
                        log::warn!("Incremental embedding sync failed: {}", e);
                    }
                }
            }
        }

        // ====== Phase 6: Checkpoint ======
        // Write sync.last_head and sync.last_timestamp
        if let Some(head) = checkpoint::get_git_head(&self.config.root) {
            let _ = checkpoint::write_last_head(self.db.conn(), &self.queries, &head);
            let _ = checkpoint::write_last_timestamp(self.db.conn(), &self.queries);
        }

        // Clear sync.failed on success
        if self.config.data_dir.exists() {
            let failed_path = self.config.data_dir.join("sync.failed");
            if failed_path.exists() {
                let _ = fs::remove_file(&failed_path);
            }
        }

        // ====== Verify-sync: post-snapshot comparison ======
        // Compare EdgeSnapshot-based affected nodes against ImpactCapture embed candidates.
        if options.verify_sync {
            if let Some(ref pre_snapshot) = pre_verify_snapshot {
                let post_snapshot = codegraph_sync::EdgeSnapshot::capture(self.db.conn());
                let diff = codegraph_sync::EdgeDiff::compute(pre_snapshot, &post_snapshot);

                if diff.has_changes() {
                    // Nodes affected per EdgeDiff (the "ground truth" set)
                    let diff_affected: &HashSet<String> = &diff.affected_nodes;

                    if let Some(ref candidates) = embed_candidate_ids {
                        // Nodes that EdgeDiff says changed but ImpactCapture didn't flag
                        let missed: Vec<&String> = diff_affected
                            .iter()
                            .filter(|id| !candidates.contains(*id))
                            .collect();

                        // Nodes that ImpactCapture flagged but EdgeDiff didn't see
                        let extra: Vec<&String> = candidates
                            .iter()
                            .filter(|id| !diff_affected.contains(*id))
                            .collect();

                        if missed.is_empty() {
                            log::info!(
                                "verify-sync: ImpactCapture covered all {} EdgeDiff-affected nodes \
                                 ({} extra candidates beyond EdgeDiff)",
                                diff_affected.len(),
                                extra.len(),
                            );
                        } else {
                            log::warn!(
                                "verify-sync: ImpactCapture missed {} of {} EdgeDiff-affected nodes: {:?}",
                                missed.len(),
                                diff_affected.len(),
                                &missed[..missed.len().min(20)],
                            );
                        }
                    } else {
                        log::info!(
                            "verify-sync: EdgeDiff found {} affected nodes but no incremental \
                             candidates were computed (full re-embed or no changes)",
                            diff_affected.len(),
                        );
                    }
                } else {
                    log::info!("verify-sync: no edge changes detected between snapshots");
                }
            }
        }

        let result = FullSyncResult {
            sync: sync_result,
            embeddings: embed_result,
        };

        // ====== Phase 7: Release + Drain Pending ======
        // Drop lock (will happen via _lock going out of scope after this block).
        // In hook mode, check for pending events and re-run sync.
        if is_hook_mode && self.config.data_dir.exists() {
            drop(_lock);
            let pending = PendingSync::new(&self.config.data_dir);
            let max_drain_iterations = 3;
            for drain_iter in 0..max_drain_iterations {
                match pending.claim() {
                    Ok(Some(hook)) => {
                        log::info!(
                            "Draining pending sync event (iteration {}/{}): hook={}",
                            drain_iter + 1,
                            max_drain_iterations,
                            hook,
                        );
                        // Re-run sync with the claimed hook name
                        let drain_options = SyncOptions {
                            hook_name: Some(hook),
                            file_list: None,
                            verify_sync: false,
                        };
                        match self.sync_with_options(drain_options) {
                            Ok(_drain_result) => {
                                log::info!("Drain sync iteration {} completed", drain_iter + 1);
                            }
                            Err(e) => {
                                log::warn!("Drain sync iteration {} failed: {}", drain_iter + 1, e);
                                // Don't propagate drain errors — the primary sync succeeded
                            }
                        }
                        pending.complete_processing().unwrap_or_else(|e| {
                            log::warn!("Failed to complete processing marker: {}", e);
                        });
                    }
                    Ok(None) => {
                        // No more pending events
                        break;
                    }
                    Err(e) => {
                        log::warn!("Failed to claim pending sync: {}", e);
                        break;
                    }
                }
            }
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
        NodeKind::Component,
    ];

    /// Compute the set of node IDs that need re-embedding after a sync.
    ///
    /// Uses ImpactCapture (pre-delete neighbors + siblings) and ScopedResolutionResult
    /// instead of the old EdgeSnapshot/EdgeDiff approach. The formula is:
    ///
    /// ```text
    /// candidates = changed_nodes ∪ pre_delete_affected ∪ resolver_source
    ///            ∪ resolver_target ∪ siblings - deleted
    /// ```
    fn compute_embed_candidates(
        &self,
        sync_result: &codegraph_sync::SyncResult,
        scoped_resolution: Option<&codegraph_resolution::ScopedResolutionResult>,
    ) -> Result<HashSet<String>, CodeGraphError> {
        // 1. changed_nodes: current nodes in changed files (after sync)
        let mut changed_nodes: HashSet<String> = HashSet::new();
        for file_path in &sync_result.changed_file_paths {
            let nodes = self.queries.get_nodes_by_file(self.db.conn(), file_path)?;
            for node in nodes {
                changed_nodes.insert(node.id.0.clone());
            }
        }

        // 2. Truly deleted: old IDs that were NOT recreated
        let truly_deleted: HashSet<&str> = sync_result
            .deleted_node_ids
            .iter()
            .map(|s| s.as_str())
            .filter(|id| !changed_nodes.contains(*id))
            .collect();

        // 3. Build candidate set
        let mut candidates = changed_nodes;

        // pre_delete_affected: neighbors of modified/deleted nodes (captured before delete)
        candidates.extend(
            sync_result
                .pre_delete_impact
                .affected_ids
                .iter()
                .cloned(),
        );

        // siblings: nodes sharing Contains parent with modified/deleted nodes
        candidates.extend(
            sync_result
                .pre_delete_impact
                .sibling_ids
                .iter()
                .cloned(),
        );

        // resolver sources and targets from scoped resolution
        if let Some(resolution) = scoped_resolution {
            candidates.extend(resolution.source_node_ids.iter().cloned());
            candidates.extend(resolution.target_node_ids.iter().cloned());
        }

        // Remove truly deleted nodes from candidates
        candidates.retain(|id| !truly_deleted.contains(id.as_str()));

        log::info!(
            "Embed candidates: {} total ({} from impact, {} siblings, {} from resolver)",
            candidates.len(),
            sync_result.pre_delete_impact.affected_ids.len(),
            sync_result.pre_delete_impact.sibling_ids.len(),
            scoped_resolution
                .map(|r| r.source_node_ids.len() + r.target_node_ids.len())
                .unwrap_or(0),
        );

        Ok(candidates)
    }

    /// Incrementally update embeddings for a pre-computed set of candidate node IDs.
    ///
    /// This is the new incremental path that replaces the EdgeDiff-based approach.
    /// Candidate computation happens in `compute_embed_candidates()`.
    ///
    /// Gracefully skips if the embedding model is unavailable.
    fn sync_embeddings_for_candidates(
        &mut self,
        sync_result: &codegraph_sync::SyncResult,
        embed_candidates: HashSet<String>,
    ) -> Result<EmbeddingSyncResult, CodeGraphError> {
        // Load embedder — gracefully skip if unavailable
        let mut embedder = TextEmbedder::new(EmbedderConfig::default());
        match embedder.load() {
            Ok(()) => {}
            Err(VectorError::ModelNotFound { .. }) => {
                log::info!("Embedding model not found, skipping incremental embedding sync");
                return Ok(EmbeddingSyncResult { skipped_no_model: true, ..Default::default() });
            }
            Err(VectorError::FeatureNotEnabled { .. }) => {
                log::debug!("ONNX feature not enabled, skipping incremental embedding sync");
                return Ok(EmbeddingSyncResult { skipped_no_model: true, ..Default::default() });
            }
            Err(e) => return Err(CodeGraphError::Vector(e)),
        }

        let dimension = embedder.dimension();
        let storage = VectorStorage::new(dimension);
        storage.init(self.db.conn())?;
        let model_name = "nomic-embed-text-v1.5";

        // 1. Compute truly deleted node IDs (old IDs not recreated)
        let new_node_ids: HashSet<&str> = embed_candidates.iter().map(|s| s.as_str()).collect();
        let truly_deleted: Vec<&str> = sync_result
            .deleted_node_ids
            .iter()
            .map(|s| s.as_str())
            .filter(|id| !new_node_ids.contains(*id))
            .collect();

        // 2. Delete stale vectors
        let vectors_deleted = truly_deleted.len();
        if !truly_deleted.is_empty() {
            storage.delete_batch(self.db.conn(), &truly_deleted)?;
        }

        // 3. Filter to embeddable kinds and generate embeddings
        let embeddable_kinds: HashSet<NodeKind> = Self::EMBEDDABLE_KINDS.iter().copied().collect();
        let mut vectors_created = 0;
        let mut vectors_updated = 0;

        for node_id in &embed_candidates {
            let node = match self.queries.get_node_by_id(self.db.conn(), node_id)? {
                Some(n) => n,
                None => continue, // Node may have been deleted
            };

            if !embeddable_kinds.contains(&node.kind) {
                continue;
            }

            // Check if this is a new embed or an update
            let is_update = storage.get(self.db.conn(), node_id)?.is_some();

            let text = match crate::embedding::build_embedding_text(
                self.db.conn(),
                &mut self.queries,
                &node,
                &self.config.embedding,
            ) {
                Ok(text) => text,
                Err(e) => {
                    log::warn!("Failed to build embedding text for {}: {}", node_id, e);
                    continue;
                }
            };

            let embedding = match embedder.embed_document(&text) {
                Ok(e) => e,
                Err(e) => {
                    log::warn!("Failed to embed {}: {}", node_id, e);
                    continue;
                }
            };

            storage.store(self.db.conn(), node_id, &embedding, model_name)?;

            if is_update {
                vectors_updated += 1;
            } else {
                vectors_created += 1;
            }
        }

        log::info!(
            "Incremental embedding sync: {} deleted, {} created, {} updated",
            vectors_deleted, vectors_created, vectors_updated
        );

        Ok(EmbeddingSyncResult {
            vectors_deleted,
            vectors_created,
            vectors_updated,
            skipped_no_model: false,
            full_reembed: false,
        })
    }

    /// Incrementally update embeddings after a sync operation (legacy EdgeDiff interface).
    ///
    /// This computes the minimal set of nodes that need re-embedding based on:
    /// - New nodes from added/modified files
    /// - Deleted nodes whose vectors need cleanup
    /// - Ripple nodes whose embedding text references changed nodes (via EdgeDiff)
    /// - Sibling nodes sharing a Contains parent with changed nodes
    ///
    /// Retained for --verify-sync mode (M4). The primary sync path now uses
    /// `compute_embed_candidates()` + `sync_embeddings_for_candidates()`.
    ///
    /// Gracefully skips if the embedding model is unavailable.
    pub fn sync_embeddings(
        &mut self,
        sync_result: &codegraph_sync::SyncResult,
        edge_diff: &codegraph_sync::edge_diff::EdgeDiff,
    ) -> Result<EmbeddingSyncResult, CodeGraphError> {
        // Load embedder — gracefully skip if unavailable
        let mut embedder = TextEmbedder::new(EmbedderConfig::default());
        match embedder.load() {
            Ok(()) => {}
            Err(VectorError::ModelNotFound { .. }) => {
                log::info!("Embedding model not found, skipping incremental embedding sync");
                return Ok(EmbeddingSyncResult { skipped_no_model: true, ..Default::default() });
            }
            Err(VectorError::FeatureNotEnabled { .. }) => {
                log::debug!("ONNX feature not enabled, skipping incremental embedding sync");
                return Ok(EmbeddingSyncResult { skipped_no_model: true, ..Default::default() });
            }
            Err(e) => return Err(CodeGraphError::Vector(e)),
        }

        let dimension = embedder.dimension();
        let storage = VectorStorage::new(dimension);
        storage.init(self.db.conn())?;
        let model_name = "nomic-embed-text-v1.5";

        // 1. Get new node IDs from changed files
        let mut new_node_ids: HashSet<String> = HashSet::new();
        for file_path in &sync_result.changed_file_paths {
            let nodes = self.queries.get_nodes_by_file(self.db.conn(), file_path)?;
            for node in nodes {
                new_node_ids.insert(node.id.0.clone());
            }
        }

        // 2. Compute truly deleted node IDs (old IDs not recreated)
        let old_node_id_set: HashSet<&str> = sync_result.deleted_node_ids
            .iter()
            .map(|s| s.as_str())
            .collect();
        let truly_deleted: Vec<&str> = old_node_id_set
            .iter()
            .filter(|id| !new_node_ids.contains(**id))
            .copied()
            .collect();

        // 3. Delete stale vectors
        let vectors_deleted = truly_deleted.len();
        if !truly_deleted.is_empty() {
            storage.delete_batch(self.db.conn(), &truly_deleted)?;
        }

        // 4. Compute ripple nodes from EdgeDiff (affected by edge changes)
        let ripple_node_ids: HashSet<String> = edge_diff.affected_nodes
            .iter()
            .filter(|id| !old_node_id_set.contains(id.as_str()) && !new_node_ids.contains(*id))
            .cloned()
            .collect();

        // 5. Compute sibling nodes (share Contains parent with changed nodes)
        let all_changed_ids: Vec<&str> = new_node_ids.iter().map(|s| s.as_str())
            .chain(truly_deleted.iter().copied())
            .collect();
        let sibling_node_ids = {
            let mut traverser = GraphTraverser::new(self.db.conn(), &mut self.queries);
            traverser.get_embedding_siblings(&all_changed_ids)?
        };

        // 6. Combine into embed candidates (exclude truly deleted)
        let mut embed_candidates: HashSet<String> = new_node_ids;
        embed_candidates.extend(ripple_node_ids);
        for id in sibling_node_ids {
            if !old_node_id_set.contains(id.as_str()) {
                embed_candidates.insert(id);
            }
        }

        // 7. Filter to embeddable kinds and generate embeddings
        let embeddable_kinds: HashSet<NodeKind> = Self::EMBEDDABLE_KINDS.iter().copied().collect();
        let mut vectors_created = 0;
        let mut vectors_updated = 0;

        for node_id in &embed_candidates {
            let node = match self.queries.get_node_by_id(self.db.conn(), node_id)? {
                Some(n) => n,
                None => continue, // Node may have been deleted
            };

            if !embeddable_kinds.contains(&node.kind) {
                continue;
            }

            // Check if this is a new embed or an update
            let is_update = storage.get(self.db.conn(), node_id)?.is_some();

            let text = match crate::embedding::build_embedding_text(
                self.db.conn(),
                &mut self.queries,
                &node,
                &self.config.embedding,
            ) {
                Ok(text) => text,
                Err(e) => {
                    log::warn!("Failed to build embedding text for {}: {}", node_id, e);
                    continue;
                }
            };

            let embedding = match embedder.embed_document(&text) {
                Ok(e) => e,
                Err(e) => {
                    log::warn!("Failed to embed {}: {}", node_id, e);
                    continue;
                }
            };

            storage.store(self.db.conn(), node_id, &embedding, model_name)?;

            if is_update {
                vectors_updated += 1;
            } else {
                vectors_created += 1;
            }
        }

        log::info!(
            "Incremental embedding sync: {} deleted, {} created, {} updated",
            vectors_deleted, vectors_created, vectors_updated
        );

        Ok(EmbeddingSyncResult {
            vectors_deleted,
            vectors_created,
            vectors_updated,
            skipped_no_model: false,
            full_reembed: false,
        })
    }

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

    // ========== Sync Helpers ==========

    /// Write a sync.failed marker file for hook-mode error visibility.
    fn write_sync_failed(&self, hook_name: &str, error: &str) {
        if !self.config.data_dir.exists() {
            return;
        }
        let failed_path = self.config.data_dir.join("sync.failed");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let content = format!("{} hook={} error={}", now, hook_name, error);
        let _ = fs::write(failed_path, content);
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

/// Combined result from sync + embedding operations
#[derive(Debug)]
pub struct FullSyncResult {
    /// File sync statistics
    pub sync: codegraph_sync::SyncResult,
    /// Embedding sync statistics
    pub embeddings: EmbeddingSyncResult,
}

/// Result of incremental embedding sync
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmbeddingSyncResult {
    /// Vectors deleted (from truly removed nodes)
    pub vectors_deleted: usize,
    /// Vectors created (new nodes)
    pub vectors_created: usize,
    /// Vectors updated (ripple re-embeds)
    pub vectors_updated: usize,
    /// Whether embedding was skipped due to missing model
    pub skipped_no_model: bool,
    /// Whether a full re-embed was performed (vs incremental)
    pub full_reembed: bool,
}

/// Ensure `.codegraph/` is listed in `.gitignore` (create or append).
///
/// This prevents the codegraph data directory from being committed to version control.
/// If the `.gitignore` file doesn't exist, it is created with the entry.
/// If it already contains the entry, this is a no-op.
fn ensure_codegraph_in_gitignore(root: &Path) -> Result<(), std::io::Error> {
    let gitignore_path = root.join(".gitignore");
    let entry = ".codegraph/";

    if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path)?;
        if content.lines().any(|line| line.trim() == entry) {
            return Ok(()); // already present
        }
        // Append with newline
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&gitignore_path)?;
        use std::io::Write;
        if !content.ends_with('\n') {
            writeln!(file)?;
        }
        writeln!(file, "{}", entry)?;
    } else {
        std::fs::write(&gitignore_path, format!("{}\n", entry))?;
    }

    Ok(())
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

    #[test]
    fn test_sync_returns_full_sync_result() {
        let temp = tempfile::tempdir().unwrap();
        let project_path = temp.path().join("sync_test");
        fs::create_dir(&project_path).unwrap();

        // Create a Rust source file
        fs::write(
            project_path.join("lib.rs"),
            "fn hello() { println!(\"hello\"); }\nfn world() {}",
        )
        .unwrap();

        let mut cg = CodeGraph::init(&project_path).unwrap();

        // First: index to establish baseline
        let index_result = cg.index_all().unwrap();
        assert!(index_result.files_indexed > 0);

        // Sync with no changes — should be up to date
        let result = cg.sync().unwrap();
        assert!(!result.sync.had_changes);
        // Embeddings skipped (no ONNX model in test env)
        assert!(
            result.embeddings.skipped_no_model
                || result.embeddings == EmbeddingSyncResult::default()
        );
    }

    #[test]
    fn test_sync_detects_modified_file() {
        let temp = tempfile::tempdir().unwrap();
        let project_path = temp.path().join("sync_modify");
        fs::create_dir(&project_path).unwrap();

        fs::write(
            project_path.join("main.rs"),
            "fn main() { println!(\"v1\"); }",
        )
        .unwrap();

        let mut cg = CodeGraph::init(&project_path).unwrap();
        cg.index_all().unwrap();

        // Modify the file
        fs::write(
            project_path.join("main.rs"),
            "fn main() { println!(\"v2\"); }\nfn helper() {}",
        )
        .unwrap();

        let result = cg.sync().unwrap();
        assert!(result.sync.had_changes);
        assert_eq!(result.sync.stats.files_modified, 1);
        assert!(!result.sync.changed_file_paths.is_empty());
    }

    #[test]
    fn test_sync_detects_deleted_file() {
        let temp = tempfile::tempdir().unwrap();
        let project_path = temp.path().join("sync_delete");
        fs::create_dir(&project_path).unwrap();

        fs::write(
            project_path.join("a.rs"),
            "fn func_a() {}",
        )
        .unwrap();
        fs::write(
            project_path.join("b.rs"),
            "fn func_b() {}",
        )
        .unwrap();

        let mut cg = CodeGraph::init(&project_path).unwrap();
        cg.index_all().unwrap();

        // Verify both files indexed
        let nodes_a = cg.get_nodes_in_file("a.rs").unwrap();
        assert!(!nodes_a.is_empty());

        // Delete file a.rs
        fs::remove_file(project_path.join("a.rs")).unwrap();

        let result = cg.sync().unwrap();
        assert!(result.sync.had_changes);
        assert_eq!(result.sync.stats.files_deleted, 1);
        assert!(!result.sync.deleted_node_ids.is_empty());
    }

    #[test]
    fn test_sync_adds_new_file() {
        let temp = tempfile::tempdir().unwrap();
        let project_path = temp.path().join("sync_add");
        fs::create_dir(&project_path).unwrap();

        fs::write(
            project_path.join("existing.rs"),
            "fn existing() {}",
        )
        .unwrap();

        let mut cg = CodeGraph::init(&project_path).unwrap();
        cg.index_all().unwrap();

        // Add a new file
        fs::write(
            project_path.join("new_file.rs"),
            "fn new_func() {}\nfn another() {}",
        )
        .unwrap();

        let result = cg.sync().unwrap();
        assert!(result.sync.had_changes);
        assert_eq!(result.sync.stats.files_added, 1);
    }
}
