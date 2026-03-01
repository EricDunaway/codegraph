//! Prepared statement query builder for the knowledge graph database

use crate::error::DbError;
use codegraph_types::{
    Edge, EdgeKind, FileRecord, GraphStats, Language, Node, NodeId, NodeKind,
    SearchResult, UnresolvedReference,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

/// Pre-compiled prepared statements and query utilities for common operations
pub struct QueryBuilder {
    /// LRU-style node cache (max 1000 entries)
    node_cache: HashMap<String, Node>,
    max_cache_size: usize,
    /// Cache insertion order for LRU eviction
    cache_order: Vec<String>,
}

impl QueryBuilder {
    /// Create a new query builder
    pub fn new(_conn: &Connection) -> Result<Self, DbError> {
        Ok(Self {
            node_cache: HashMap::new(),
            max_cache_size: 1000,
            cache_order: Vec::new(),
        })
    }

    // =========================================================================
    // Node Operations
    // =========================================================================

    /// Insert or update a node (upsert semantics)
    pub fn insert_node(&self, conn: &Connection, node: &Node) -> Result<(), DbError> {
        conn.execute(
            r#"
            INSERT OR REPLACE INTO nodes (
                id, kind, name, qualified_name, file_path, language,
                start_line, end_line, start_column, end_column,
                docstring, signature, visibility,
                is_exported, is_async, is_static, is_abstract,
                decorators, type_parameters, updated_at,
                inferred_type, resolved_import_path, code_snippet,
                thrown_errors, test_names, package_name
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10,
                ?11, ?12, ?13,
                ?14, ?15, ?16, ?17,
                ?18, ?19, ?20,
                ?21, ?22, ?23,
                ?24, ?25, ?26
            )
            "#,
            params![
                node.id.as_str(),
                node.kind.as_str(),
                node.name,
                node.qualified_name,
                node.file_path,
                node.language.as_str(),
                node.start_line,
                node.end_line,
                node.start_column,
                node.end_column,
                node.docstring,
                node.signature,
                node.visibility.map(|v| v.as_str()),
                node.is_exported as i32,
                node.is_async as i32,
                node.is_static as i32,
                node.is_abstract as i32,
                if node.decorators.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&node.decorators).unwrap())
                },
                if node.type_parameters.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&node.type_parameters).unwrap())
                },
                node.updated_at,
                // Enrichment fields (v2)
                node.inferred_type,
                node.resolved_import_path,
                node.code_snippet,
                if node.thrown_errors.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&node.thrown_errors).unwrap())
                },
                if node.test_names.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&node.test_names).unwrap())
                },
                node.package_name,
            ],
        )?;
        Ok(())
    }

    /// Insert multiple nodes in a transaction
    pub fn insert_nodes(&self, conn: &Connection, nodes: &[Node]) -> Result<(), DbError> {
        for node in nodes {
            self.insert_node(conn, node)?;
        }
        Ok(())
    }

    /// Update an existing node
    pub fn update_node(&mut self, conn: &Connection, node: &Node) -> Result<(), DbError> {
        // Invalidate cache
        self.invalidate_cache(&node.id.0);

        conn.execute(
            r#"
            UPDATE nodes SET
                kind = ?2,
                name = ?3,
                qualified_name = ?4,
                file_path = ?5,
                language = ?6,
                start_line = ?7,
                end_line = ?8,
                start_column = ?9,
                end_column = ?10,
                docstring = ?11,
                signature = ?12,
                visibility = ?13,
                is_exported = ?14,
                is_async = ?15,
                is_static = ?16,
                is_abstract = ?17,
                decorators = ?18,
                type_parameters = ?19,
                updated_at = ?20,
                inferred_type = ?21,
                resolved_import_path = ?22,
                code_snippet = ?23,
                thrown_errors = ?24,
                test_names = ?25,
                package_name = ?26
            WHERE id = ?1
            "#,
            params![
                node.id.as_str(),
                node.kind.as_str(),
                node.name,
                node.qualified_name,
                node.file_path,
                node.language.as_str(),
                node.start_line,
                node.end_line,
                node.start_column,
                node.end_column,
                node.docstring,
                node.signature,
                node.visibility.map(|v| v.as_str()),
                node.is_exported as i32,
                node.is_async as i32,
                node.is_static as i32,
                node.is_abstract as i32,
                if node.decorators.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&node.decorators).unwrap())
                },
                if node.type_parameters.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&node.type_parameters).unwrap())
                },
                node.updated_at,
                // Enrichment fields (v2)
                node.inferred_type,
                node.resolved_import_path,
                node.code_snippet,
                if node.thrown_errors.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&node.thrown_errors).unwrap())
                },
                if node.test_names.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&node.test_names).unwrap())
                },
                node.package_name,
            ],
        )?;
        Ok(())
    }

    /// Delete a node by ID
    pub fn delete_node(&mut self, conn: &Connection, id: &str) -> Result<(), DbError> {
        self.invalidate_cache(id);
        conn.execute("DELETE FROM nodes WHERE id = ?", params![id])?;
        Ok(())
    }

    /// Delete all nodes for a file
    pub fn delete_nodes_by_file(&mut self, conn: &Connection, file_path: &str) -> Result<(), DbError> {
        // Invalidate cache for nodes in this file
        let to_remove: Vec<String> = self
            .node_cache
            .iter()
            .filter(|(_, node)| node.file_path == file_path)
            .map(|(id, _)| id.clone())
            .collect();

        for id in to_remove {
            self.invalidate_cache(&id);
        }

        conn.execute("DELETE FROM nodes WHERE file_path = ?", params![file_path])?;
        Ok(())
    }

    /// Get a node by ID
    pub fn get_node_by_id(&mut self, conn: &Connection, id: &str) -> Result<Option<Node>, DbError> {
        // Check cache first
        if let Some(node) = self.node_cache.get(id).cloned() {
            // Move to end for LRU
            self.touch_cache(id);
            return Ok(Some(node));
        }

        let node = conn
            .query_row(
                "SELECT * FROM nodes WHERE id = ?",
                params![id],
                Self::row_to_node,
            )
            .optional()?;

        if let Some(ref n) = node {
            self.cache_node(n.clone());
        }

        Ok(node)
    }

    /// Get all nodes in a file
    pub fn get_nodes_by_file(&self, conn: &Connection, file_path: &str) -> Result<Vec<Node>, DbError> {
        let mut stmt = conn.prepare(
            "SELECT * FROM nodes WHERE file_path = ? ORDER BY start_line"
        )?;

        let nodes = stmt
            .query_map(params![file_path], Self::row_to_node)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(nodes)
    }

    /// Get all nodes of a specific kind
    pub fn get_nodes_by_kind(&self, conn: &Connection, kind: NodeKind) -> Result<Vec<Node>, DbError> {
        let mut stmt = conn.prepare("SELECT * FROM nodes WHERE kind = ?")?;

        let nodes = stmt
            .query_map(params![kind.as_str()], Self::row_to_node)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(nodes)
    }

    /// Search nodes by name using FTS5 with LIKE fallback
    pub fn search_nodes(
        &self,
        conn: &Connection,
        query: &str,
        kinds: Option<&[NodeKind]>,
        languages: Option<&[Language]>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SearchResult>, DbError> {
        // Try FTS5 first
        let results = self.search_nodes_fts(conn, query, kinds, languages, limit, offset)?;

        // Fall back to LIKE if no results
        if results.is_empty() && query.len() >= 2 {
            return self.search_nodes_like(conn, query, kinds, languages, limit, offset);
        }

        Ok(results)
    }

    /// FTS5 search
    fn search_nodes_fts(
        &self,
        conn: &Connection,
        query: &str,
        kinds: Option<&[NodeKind]>,
        languages: Option<&[Language]>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SearchResult>, DbError> {
        // Build FTS query with prefix matching
        let fts_query: String = query
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .map(|t| format!("\"{}\"*", t.replace(['\'', '"', '*', '(', ')'], "")))
            .collect::<Vec<_>>()
            .join(" OR ");

        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let mut sql = String::from(
            r#"
            SELECT nodes.*, bm25(nodes_fts) as score
            FROM nodes_fts
            JOIN nodes ON nodes_fts.id = nodes.id
            WHERE nodes_fts MATCH ?1
            "#
        );

        let mut param_idx = 2;

        if let Some(k) = kinds {
            if !k.is_empty() {
                let placeholders: String = k.iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", param_idx + i))
                    .collect::<Vec<_>>()
                    .join(",");
                sql.push_str(&format!(" AND nodes.kind IN ({placeholders})"));
                param_idx += k.len();
            }
        }

        if let Some(l) = languages {
            if !l.is_empty() {
                let placeholders: String = l.iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", param_idx + i))
                    .collect::<Vec<_>>()
                    .join(",");
                sql.push_str(&format!(" AND nodes.language IN ({placeholders})"));
                param_idx += l.len();
            }
        }

        sql.push_str(&format!(" ORDER BY score LIMIT ?{} OFFSET ?{}", param_idx, param_idx + 1));

        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Ok(Vec::new()), // FTS query failed
        };

        // Build params dynamically
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        params_vec.push(Box::new(fts_query));

        if let Some(k) = kinds {
            for kind in k {
                params_vec.push(Box::new(kind.as_str().to_string()));
            }
        }

        if let Some(l) = languages {
            for lang in l {
                params_vec.push(Box::new(lang.as_str().to_string()));
            }
        }

        params_vec.push(Box::new(limit as i64));
        params_vec.push(Box::new(offset as i64));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let results = stmt
            .query_map(params_refs.as_slice(), |row| {
                let node = Self::row_to_node(row)?;
                let score: f64 = row.get("score")?;
                Ok(SearchResult {
                    node,
                    score: score.abs() as f32,
                    highlights: Vec::new(),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// LIKE-based substring search fallback
    fn search_nodes_like(
        &self,
        conn: &Connection,
        query: &str,
        kinds: Option<&[NodeKind]>,
        languages: Option<&[Language]>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SearchResult>, DbError> {
        let contains = format!("%{query}%");
        let starts_with = format!("{query}%");

        let mut sql = String::from(
            r#"
            SELECT *,
                CASE
                    WHEN name = ?1 THEN 1.0
                    WHEN name LIKE ?2 THEN 0.9
                    WHEN name LIKE ?3 THEN 0.8
                    WHEN qualified_name LIKE ?3 THEN 0.7
                    ELSE 0.5
                END as score
            FROM nodes
            WHERE (name LIKE ?3 OR qualified_name LIKE ?3)
            "#
        );

        let mut param_idx = 4;

        if let Some(k) = kinds {
            if !k.is_empty() {
                let placeholders: String = k.iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", param_idx + i))
                    .collect::<Vec<_>>()
                    .join(",");
                sql.push_str(&format!(" AND kind IN ({placeholders})"));
                param_idx += k.len();
            }
        }

        if let Some(l) = languages {
            if !l.is_empty() {
                let placeholders: String = l.iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", param_idx + i))
                    .collect::<Vec<_>>()
                    .join(",");
                sql.push_str(&format!(" AND language IN ({placeholders})"));
                param_idx += l.len();
            }
        }

        sql.push_str(&format!(
            " ORDER BY score DESC, length(name) ASC LIMIT ?{} OFFSET ?{}",
            param_idx, param_idx + 1
        ));

        let mut stmt = conn.prepare(&sql)?;

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        params_vec.push(Box::new(query.to_string()));
        params_vec.push(Box::new(starts_with));
        params_vec.push(Box::new(contains));

        if let Some(k) = kinds {
            for kind in k {
                params_vec.push(Box::new(kind.as_str().to_string()));
            }
        }

        if let Some(l) = languages {
            for lang in l {
                params_vec.push(Box::new(lang.as_str().to_string()));
            }
        }

        params_vec.push(Box::new(limit as i64));
        params_vec.push(Box::new(offset as i64));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let results = stmt
            .query_map(params_refs.as_slice(), |row| {
                let node = Self::row_to_node(row)?;
                let score: f64 = row.get("score")?;
                Ok(SearchResult {
                    node,
                    score: score as f32,
                    highlights: Vec::new(),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    // =========================================================================
    // Edge Operations
    // =========================================================================

    /// Insert a new edge
    pub fn insert_edge(&self, conn: &Connection, edge: &Edge) -> Result<(), DbError> {
        conn.execute(
            r#"
            INSERT OR IGNORE INTO edges (source, target, kind, metadata, line, col)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                edge.source.as_str(),
                edge.target.as_str(),
                edge.kind.as_str(),
                edge.metadata.as_ref().map(|m| serde_json::to_string(m).unwrap()),
                edge.line,
                edge.column,
            ],
        )?;
        Ok(())
    }

    /// Insert multiple edges
    pub fn insert_edges(&self, conn: &Connection, edges: &[Edge]) -> Result<(), DbError> {
        for edge in edges {
            self.insert_edge(conn, edge)?;
        }
        Ok(())
    }

    /// Delete all edges from a source node
    pub fn delete_edges_by_source(&self, conn: &Connection, source_id: &str) -> Result<(), DbError> {
        conn.execute("DELETE FROM edges WHERE source = ?", params![source_id])?;
        Ok(())
    }

    /// Get outgoing edges from a node
    pub fn get_outgoing_edges(
        &self,
        conn: &Connection,
        source_id: &str,
        kinds: Option<&[EdgeKind]>,
    ) -> Result<Vec<Edge>, DbError> {
        if let Some(k) = kinds {
            if !k.is_empty() {
                let placeholders: String = (0..k.len()).map(|i| format!("?{}", i + 2)).collect::<Vec<_>>().join(",");
                let sql = format!(
                    "SELECT * FROM edges WHERE source = ?1 AND kind IN ({placeholders})"
                );
                let mut stmt = conn.prepare(&sql)?;

                let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
                params_vec.push(Box::new(source_id.to_string()));
                for kind in k {
                    params_vec.push(Box::new(kind.as_str().to_string()));
                }
                let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

                let edges = stmt
                    .query_map(params_refs.as_slice(), Self::row_to_edge)?
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(edges);
            }
        }

        let mut stmt = conn.prepare("SELECT * FROM edges WHERE source = ?")?;
        let edges = stmt
            .query_map(params![source_id], Self::row_to_edge)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(edges)
    }

    /// Get incoming edges to a node
    pub fn get_incoming_edges(
        &self,
        conn: &Connection,
        target_id: &str,
        kinds: Option<&[EdgeKind]>,
    ) -> Result<Vec<Edge>, DbError> {
        if let Some(k) = kinds {
            if !k.is_empty() {
                let placeholders: String = (0..k.len()).map(|i| format!("?{}", i + 2)).collect::<Vec<_>>().join(",");
                let sql = format!(
                    "SELECT * FROM edges WHERE target = ?1 AND kind IN ({placeholders})"
                );
                let mut stmt = conn.prepare(&sql)?;

                let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
                params_vec.push(Box::new(target_id.to_string()));
                for kind in k {
                    params_vec.push(Box::new(kind.as_str().to_string()));
                }
                let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

                let edges = stmt
                    .query_map(params_refs.as_slice(), Self::row_to_edge)?
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(edges);
            }
        }

        let mut stmt = conn.prepare("SELECT * FROM edges WHERE target = ?")?;
        let edges = stmt
            .query_map(params![target_id], Self::row_to_edge)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(edges)
    }

    // =========================================================================
    // File Operations
    // =========================================================================

    /// Insert or update a file record
    pub fn upsert_file(&self, conn: &Connection, file: &FileRecord) -> Result<(), DbError> {
        conn.execute(
            r#"
            INSERT INTO files (path, content_hash, language, size, modified_at, indexed_at, node_count, errors)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(path) DO UPDATE SET
                content_hash = ?2,
                language = ?3,
                size = ?4,
                modified_at = ?5,
                indexed_at = ?6,
                node_count = ?7,
                errors = ?8
            "#,
            params![
                file.path,
                file.content_hash,
                file.language.as_str(),
                file.size as i64,
                file.modified_at,
                file.indexed_at,
                file.node_count,
                if file.errors.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&file.errors).unwrap())
                },
            ],
        )?;
        Ok(())
    }

    /// Delete a file record and its nodes
    pub fn delete_file(&mut self, conn: &Connection, file_path: &str) -> Result<(), DbError> {
        self.delete_nodes_by_file(conn, file_path)?;
        conn.execute("DELETE FROM files WHERE path = ?", params![file_path])?;
        Ok(())
    }

    /// Get a file record by path
    pub fn get_file_by_path(&self, conn: &Connection, file_path: &str) -> Result<Option<FileRecord>, DbError> {
        conn.query_row(
            "SELECT * FROM files WHERE path = ?",
            params![file_path],
            Self::row_to_file_record,
        )
        .optional()
        .map_err(DbError::from)
    }

    /// Get all tracked files
    pub fn get_all_files(&self, conn: &Connection) -> Result<Vec<FileRecord>, DbError> {
        let mut stmt = conn.prepare("SELECT * FROM files ORDER BY path")?;
        let files = stmt
            .query_map([], Self::row_to_file_record)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(files)
    }

    // =========================================================================
    // Unresolved References
    // =========================================================================

    /// Insert an unresolved reference
    pub fn insert_unresolved_ref(&self, conn: &Connection, ref_: &UnresolvedReference) -> Result<(), DbError> {
        conn.execute(
            r#"
            INSERT INTO unresolved_refs (from_node_id, reference_name, reference_kind, line, col, candidates)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                ref_.from_node_id.as_str(),
                ref_.reference_name,
                ref_.reference_kind.as_str(),
                ref_.line,
                ref_.column,
                if ref_.candidates.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&ref_.candidates).unwrap())
                },
            ],
        )?;
        Ok(())
    }

    /// Get unresolved references by name
    pub fn get_unresolved_by_name(&self, conn: &Connection, name: &str) -> Result<Vec<UnresolvedReference>, DbError> {
        let mut stmt = conn.prepare(
            "SELECT * FROM unresolved_refs WHERE reference_name = ?"
        )?;
        let refs = stmt
            .query_map(params![name], Self::row_to_unresolved_ref)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(refs)
    }

    /// Get all unresolved references (only those not yet resolved)
    pub fn get_all_unresolved_refs(&self, conn: &Connection) -> Result<Vec<UnresolvedReference>, DbError> {
        let mut stmt = conn.prepare("SELECT * FROM unresolved_refs WHERE resolved = 0")?;
        let refs = stmt
            .query_map([], Self::row_to_unresolved_ref)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(refs)
    }

    /// Get unresolved refs from nodes in the given files (source-scoped resolution).
    /// Only returns refs where resolved = 0.
    pub fn get_unresolved_refs_by_files(
        &self,
        conn: &Connection,
        file_paths: &[&str],
    ) -> Result<Vec<UnresolvedReference>, DbError> {
        if file_paths.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders: Vec<String> = (1..=file_paths.len()).map(|i| format!("?{}", i)).collect();
        let sql = format!(
            r#"SELECT ur.* FROM unresolved_refs ur
               JOIN nodes n ON ur.from_node_id = n.id
               WHERE n.file_path IN ({}) AND ur.resolved = 0"#,
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = file_paths.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let refs = stmt.query_map(params.as_slice(), Self::row_to_unresolved_ref)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(refs)
    }

    /// Get symbol names defined in the given files (for target-scoped resolution).
    pub fn get_symbol_names_in_files(
        &self,
        conn: &Connection,
        file_paths: &[&str],
    ) -> Result<Vec<String>, DbError> {
        if file_paths.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders: Vec<String> = (1..=file_paths.len()).map(|i| format!("?{}", i)).collect();
        let sql = format!(
            "SELECT DISTINCT name FROM nodes WHERE file_path IN ({})",
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = file_paths.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let names = stmt.query_map(params.as_slice(), |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(names)
    }

    /// Get unresolved refs matching given names, with frequency cap.
    /// Skips names with > cap matching unresolved refs.
    pub fn get_unresolved_refs_by_names_capped(
        &self,
        conn: &Connection,
        names: &[&str],
        cap: usize,
    ) -> Result<Vec<UnresolvedReference>, DbError> {
        let mut result = Vec::new();
        for name in names {
            let count: usize = conn.query_row(
                "SELECT COUNT(*) FROM unresolved_refs WHERE reference_name = ?1 AND resolved = 0",
                params![name],
                |row| row.get(0),
            )?;
            if count > cap {
                log::debug!("Skipping target-scoped resolution for '{}': {} refs > cap {}", name, count, cap);
                continue;
            }
            let mut stmt = conn.prepare(
                "SELECT * FROM unresolved_refs WHERE reference_name = ?1 AND resolved = 0"
            )?;
            let refs = stmt.query_map(params![name], Self::row_to_unresolved_ref)?
                .collect::<Result<Vec<_>, _>>()?;
            result.extend(refs);
        }
        Ok(result)
    }

    /// Mark an unresolved ref as resolved (set resolved = 1 instead of deleting).
    pub fn mark_unresolved_ref_resolved(
        &self,
        conn: &Connection,
        from_node_id: &str,
        reference_name: &str,
    ) -> Result<(), DbError> {
        conn.execute(
            "UPDATE unresolved_refs SET resolved = 1 WHERE from_node_id = ?1 AND reference_name = ?2 AND resolved = 0",
            params![from_node_id, reference_name],
        )?;
        Ok(())
    }

    /// Clear all unresolved references
    pub fn clear_unresolved_refs(&self, conn: &Connection) -> Result<(), DbError> {
        conn.execute("DELETE FROM unresolved_refs", [])?;
        Ok(())
    }

    /// Get unresolved references as tuples for resolution
    pub fn get_unresolved_references(
        &self,
        conn: &Connection,
    ) -> Result<Vec<(String, String, String, i32)>, DbError> {
        let mut stmt = conn.prepare(
            r#"
            SELECT ur.from_node_id, ur.reference_name, n.file_path, ur.line
            FROM unresolved_refs ur
            JOIN nodes n ON ur.from_node_id = n.id
            "#
        )?;

        let refs = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i32>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(refs)
    }

    /// Insert an unresolved reference (simple version)
    pub fn insert_unresolved_reference(
        &self,
        conn: &Connection,
        source_id: &str,
        ref_name: &str,
        file_path: &str,
        line: i32,
    ) -> Result<(), DbError> {
        conn.execute(
            r#"
            INSERT INTO unresolved_refs (from_node_id, reference_name, reference_kind, line, col)
            VALUES (?1, ?2, 'references', ?3, 0)
            "#,
            params![source_id, ref_name, line],
        )?;
        Ok(())
    }

    /// Delete a specific unresolved reference
    pub fn delete_unresolved_reference(
        &self,
        conn: &Connection,
        source_id: &str,
        ref_name: &str,
    ) -> Result<(), DbError> {
        conn.execute(
            "DELETE FROM unresolved_refs WHERE from_node_id = ? AND reference_name = ?",
            params![source_id, ref_name],
        )?;
        Ok(())
    }

    /// Get all nodes in a file (alias for get_nodes_by_file)
    pub fn get_nodes_in_file(&self, conn: &Connection, file_path: &str) -> Result<Vec<Node>, DbError> {
        self.get_nodes_by_file(conn, file_path)
    }

    /// Search symbols by name (returns nodes directly)
    pub fn search_symbols(&self, conn: &Connection, query: &str) -> Result<Vec<Node>, DbError> {
        let results = self.search_nodes(conn, query, None, None, 100, 0)?;
        Ok(results.into_iter().map(|r| r.node).collect())
    }

    // =========================================================================
    // Statistics
    // =========================================================================

    /// Get graph statistics
    pub fn get_stats(&self, conn: &Connection) -> Result<GraphStats, DbError> {
        let node_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM nodes",
            [],
            |row| row.get(0),
        )?;

        let edge_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM edges",
            [],
            |row| row.get(0),
        )?;

        let file_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM files",
            [],
            |row| row.get(0),
        )?;

        let mut nodes_by_kind = HashMap::new();
        let mut stmt = conn.prepare("SELECT kind, COUNT(*) FROM nodes GROUP BY kind")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let kind: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            if let Ok(k) = kind.parse::<NodeKind>() {
                nodes_by_kind.insert(k, count as u64);
            }
        }

        let mut edges_by_kind = HashMap::new();
        let mut stmt = conn.prepare("SELECT kind, COUNT(*) FROM edges GROUP BY kind")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let kind: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            if let Ok(k) = kind.parse::<EdgeKind>() {
                edges_by_kind.insert(k, count as u64);
            }
        }

        let mut files_by_language = HashMap::new();
        let mut stmt = conn.prepare("SELECT language, COUNT(*) FROM files GROUP BY language")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let lang: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            if let Ok(l) = lang.parse::<Language>() {
                files_by_language.insert(l, count as u64);
            }
        }

        Ok(GraphStats {
            node_count: node_count as u64,
            edge_count: edge_count as u64,
            file_count: file_count as u64,
            nodes_by_kind,
            edges_by_kind,
            files_by_language,
            db_size_bytes: 0, // Set by caller
            last_updated: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
        })
    }

    /// Clear all data
    pub fn clear(&mut self, conn: &Connection) -> Result<(), DbError> {
        self.node_cache.clear();
        self.cache_order.clear();
        conn.execute_batch(
            r#"
            DELETE FROM unresolved_refs;
            DELETE FROM edges;
            DELETE FROM nodes;
            DELETE FROM files;
            "#
        )?;
        Ok(())
    }

    // =========================================================================
    // Metadata Operations
    // =========================================================================

    /// Set a metadata key-value pair (upsert)
    pub fn set_metadata(&self, conn: &Connection, key: &str, value: &str) -> Result<(), DbError> {
        conn.execute(
            "INSERT INTO metadata (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    /// Get a metadata value by key
    pub fn get_metadata(&self, conn: &Connection, key: &str) -> Result<Option<String>, DbError> {
        conn.query_row(
            "SELECT value FROM metadata WHERE key = ?",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(DbError::from)
    }

    /// Delete a metadata key
    pub fn delete_metadata(&self, conn: &Connection, key: &str) -> Result<(), DbError> {
        conn.execute("DELETE FROM metadata WHERE key = ?", params![key])?;
        Ok(())
    }

    /// Get all metadata as a key-value map
    pub fn get_all_metadata(&self, conn: &Connection) -> Result<std::collections::HashMap<String, String>, DbError> {
        let mut stmt = conn.prepare("SELECT key, value FROM metadata")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (k, v) = row?;
            map.insert(k, v);
        }
        Ok(map)
    }

    // =========================================================================
    // Bulk Data Operations
    // =========================================================================

    /// Clear all graph data (nodes, edges, files, unresolved_refs).
    /// Used by index_all() to do a clean rebuild.
    pub fn clear_all_graph_data(&mut self, conn: &Connection) -> Result<(), DbError> {
        self.node_cache.clear();
        self.cache_order.clear();
        // Delete in dependency order (also handled by FK CASCADE, but explicit for clarity)
        conn.execute_batch(
            "DELETE FROM edges;
             DELETE FROM unresolved_refs;
             DELETE FROM nodes;
             DELETE FROM files;"
        )?;
        Ok(())
    }

    // =========================================================================
    // Cache Management
    // =========================================================================

    fn cache_node(&mut self, node: Node) {
        if self.node_cache.len() >= self.max_cache_size {
            // Evict oldest
            if let Some(oldest) = self.cache_order.first().cloned() {
                self.node_cache.remove(&oldest);
                self.cache_order.remove(0);
            }
        }
        let id = node.id.0.clone();
        self.node_cache.insert(id.clone(), node);
        self.cache_order.push(id);
    }

    fn invalidate_cache(&mut self, id: &str) {
        self.node_cache.remove(id);
        self.cache_order.retain(|i| i != id);
    }

    fn touch_cache(&mut self, id: &str) {
        if let Some(pos) = self.cache_order.iter().position(|i| i == id) {
            let id = self.cache_order.remove(pos);
            self.cache_order.push(id);
        }
    }

    /// Clear the node cache
    pub fn clear_cache(&mut self) {
        self.node_cache.clear();
        self.cache_order.clear();
    }

    // =========================================================================
    // Row Conversion Helpers
    // =========================================================================

    fn row_to_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
        let decorators: Option<String> = row.get("decorators")?;
        let type_params: Option<String> = row.get("type_parameters")?;
        let visibility: Option<String> = row.get("visibility")?;
        let kind: String = row.get("kind")?;
        let language: String = row.get("language")?;

        // Enrichment fields (optional - may not exist in v1 schema)
        let thrown_errors: Option<String> = row.get("thrown_errors").ok().flatten();
        let test_names: Option<String> = row.get("test_names").ok().flatten();

        Ok(Node {
            id: NodeId::new(row.get::<_, String>("id")?),
            kind: kind.parse().unwrap_or(NodeKind::Function),
            name: row.get("name")?,
            qualified_name: row.get("qualified_name")?,
            file_path: row.get("file_path")?,
            language: language.parse().unwrap_or(Language::Unknown),
            start_line: row.get("start_line")?,
            end_line: row.get("end_line")?,
            start_column: row.get("start_column")?,
            end_column: row.get("end_column")?,
            docstring: row.get("docstring")?,
            signature: row.get("signature")?,
            visibility: visibility.and_then(|v| v.parse().ok()),
            is_exported: row.get::<_, i32>("is_exported")? != 0,
            is_async: row.get::<_, i32>("is_async")? != 0,
            is_static: row.get::<_, i32>("is_static")? != 0,
            is_abstract: row.get::<_, i32>("is_abstract")? != 0,
            decorators: decorators
                .map(|d| serde_json::from_str(&d).unwrap_or_default())
                .unwrap_or_default(),
            type_parameters: type_params
                .map(|t| serde_json::from_str(&t).unwrap_or_default())
                .unwrap_or_default(),
            updated_at: row.get("updated_at")?,
            // Enrichment fields (v2)
            inferred_type: row.get("inferred_type").ok().flatten(),
            resolved_import_path: row.get("resolved_import_path").ok().flatten(),
            code_snippet: row.get("code_snippet").ok().flatten(),
            thrown_errors: thrown_errors
                .map(|s| serde_json::from_str(&s).unwrap_or_default())
                .unwrap_or_default(),
            test_names: test_names
                .map(|s| serde_json::from_str(&s).unwrap_or_default())
                .unwrap_or_default(),
            package_name: row.get("package_name").ok().flatten(),
        })
    }

    fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
        let kind: String = row.get("kind")?;
        let metadata: Option<String> = row.get("metadata")?;

        Ok(Edge {
            id: Some(row.get("id")?),
            source: NodeId::new(row.get::<_, String>("source")?),
            target: NodeId::new(row.get::<_, String>("target")?),
            kind: kind.parse().unwrap_or(EdgeKind::References),
            metadata: metadata.and_then(|m| serde_json::from_str(&m).ok()),
            line: row.get("line")?,
            column: row.get("col")?,
        })
    }

    fn row_to_file_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileRecord> {
        let language: String = row.get("language")?;
        let errors: Option<String> = row.get("errors")?;

        Ok(FileRecord {
            path: row.get("path")?,
            content_hash: row.get("content_hash")?,
            language: language.parse().unwrap_or(Language::Unknown),
            size: row.get::<_, i64>("size")? as u64,
            modified_at: row.get("modified_at")?,
            indexed_at: row.get("indexed_at")?,
            node_count: row.get("node_count")?,
            errors: errors
                .map(|e| serde_json::from_str(&e).unwrap_or_default())
                .unwrap_or_default(),
        })
    }

    fn row_to_unresolved_ref(row: &rusqlite::Row<'_>) -> rusqlite::Result<UnresolvedReference> {
        let kind: String = row.get("reference_kind")?;
        let candidates: Option<String> = row.get("candidates")?;

        Ok(UnresolvedReference {
            from_node_id: NodeId::new(row.get::<_, String>("from_node_id")?),
            reference_name: row.get("reference_name")?,
            reference_kind: kind.parse().unwrap_or(EdgeKind::References),
            line: row.get("line")?,
            column: row.get("col")?,
            candidates: candidates
                .map(|c| serde_json::from_str(&c).unwrap_or_default())
                .unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DatabaseConnection;

    fn create_test_node(id: &str, name: &str) -> Node {
        Node::new(
            id,
            NodeKind::Function,
            name,
            format!("test.rs::{name}"),
            "test.rs",
            Language::Rust,
            1,
            10,
        )
    }

    #[test]
    fn test_insert_and_get_node() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let mut queries = QueryBuilder::new(db.conn()).unwrap();

        let node = create_test_node("node1", "myFunction");
        queries.insert_node(db.conn(), &node).unwrap();

        let retrieved = queries.get_node_by_id(db.conn(), "node1").unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.name, "myFunction");
        assert_eq!(retrieved.kind, NodeKind::Function);
    }

    #[test]
    fn test_insert_and_get_edge() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        // Insert nodes first
        let node1 = create_test_node("node1", "caller");
        let node2 = create_test_node("node2", "callee");
        queries.insert_node(db.conn(), &node1).unwrap();
        queries.insert_node(db.conn(), &node2).unwrap();

        // Insert edge
        let edge = Edge::new("node1", "node2", EdgeKind::Calls);
        queries.insert_edge(db.conn(), &edge).unwrap();

        // Get outgoing edges
        let edges = queries.get_outgoing_edges(db.conn(), "node1", None).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target.as_str(), "node2");
        assert_eq!(edges[0].kind, EdgeKind::Calls);
    }

    #[test]
    fn test_upsert_file() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        let file = FileRecord {
            path: "test.rs".to_string(),
            content_hash: "abc123".to_string(),
            language: Language::Rust,
            size: 100,
            modified_at: 12345,
            indexed_at: 12346,
            node_count: 5,
            errors: Vec::new(),
        };

        queries.upsert_file(db.conn(), &file).unwrap();

        let retrieved = queries.get_file_by_path(db.conn(), "test.rs").unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.content_hash, "abc123");
        assert_eq!(retrieved.node_count, 5);
    }

    #[test]
    fn test_get_stats() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        // Insert some data
        let node1 = create_test_node("node1", "func1");
        let node2 = create_test_node("node2", "func2");
        queries.insert_node(db.conn(), &node1).unwrap();
        queries.insert_node(db.conn(), &node2).unwrap();

        let edge = Edge::new("node1", "node2", EdgeKind::Calls);
        queries.insert_edge(db.conn(), &edge).unwrap();

        let stats = queries.get_stats(db.conn()).unwrap();
        assert_eq!(stats.node_count, 2);
        assert_eq!(stats.edge_count, 1);
    }

    #[test]
    fn test_delete_nodes_by_file() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let mut queries = QueryBuilder::new(db.conn()).unwrap();

        let node1 = create_test_node("node1", "func1");
        let mut node2 = create_test_node("node2", "func2");
        node2.file_path = "other.rs".to_string();

        queries.insert_node(db.conn(), &node1).unwrap();
        queries.insert_node(db.conn(), &node2).unwrap();

        queries.delete_nodes_by_file(db.conn(), "test.rs").unwrap();

        let remaining = queries.get_node_by_id(db.conn(), "node1").unwrap();
        assert!(remaining.is_none());

        let still_exists = queries.get_node_by_id(db.conn(), "node2").unwrap();
        assert!(still_exists.is_some());
    }

    #[test]
    fn test_search_nodes_like() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        let node1 = create_test_node("node1", "signInWithGoogle");
        let node2 = create_test_node("node2", "signOut");
        let node3 = create_test_node("node3", "unrelated");

        queries.insert_node(db.conn(), &node1).unwrap();
        queries.insert_node(db.conn(), &node2).unwrap();
        queries.insert_node(db.conn(), &node3).unwrap();

        let results = queries.search_nodes(db.conn(), "sign", None, None, 10, 0).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_insert_and_read_enriched_node() {
        use crate::migrations;

        let db = DatabaseConnection::open_in_memory().unwrap();
        migrations::run_migrations(db.conn()).unwrap();
        let mut queries = QueryBuilder::new(db.conn()).unwrap();

        let mut node = Node::new(
            "enriched-node",
            NodeKind::Function,
            "processPayment",
            "PaymentService.processPayment",
            "src/payment.ts",
            Language::TypeScript,
            10,
            25,
        );
        node.inferred_type = Some("Promise<Receipt>".to_string());
        node.thrown_errors = vec!["PaymentError".to_string(), "ValidationError".to_string()];
        node.package_name = Some("@myapp/payments".to_string());
        node.code_snippet = Some("async function processPayment() {\n  // ...\n}".to_string());

        // Insert
        queries.insert_node(db.conn(), &node).unwrap();

        // Read back
        let retrieved = queries.get_node_by_id(db.conn(), "enriched-node").unwrap().unwrap();

        assert_eq!(retrieved.inferred_type.as_deref(), Some("Promise<Receipt>"));
        assert_eq!(retrieved.thrown_errors.len(), 2);
        assert!(retrieved.thrown_errors.contains(&"PaymentError".to_string()));
        assert_eq!(retrieved.package_name.as_deref(), Some("@myapp/payments"));
        assert!(retrieved.code_snippet.is_some());
    }

    #[test]
    fn test_update_enriched_node() {
        use crate::migrations;

        let db = DatabaseConnection::open_in_memory().unwrap();
        migrations::run_migrations(db.conn()).unwrap();
        let mut queries = QueryBuilder::new(db.conn()).unwrap();

        // Insert initial node
        let mut node = Node::new(
            "update-test",
            NodeKind::Function,
            "myFunc",
            "module::myFunc",
            "src/lib.rs",
            Language::Rust,
            1,
            10,
        );
        queries.insert_node(db.conn(), &node).unwrap();

        // Update with enrichment data
        node.inferred_type = Some("Result<(), Error>".to_string());
        node.test_names = vec!["test_myFunc".to_string()];
        queries.update_node(db.conn(), &node).unwrap();

        // Read back
        let retrieved = queries.get_node_by_id(db.conn(), "update-test").unwrap().unwrap();

        assert_eq!(retrieved.inferred_type.as_deref(), Some("Result<(), Error>"));
        assert_eq!(retrieved.test_names.len(), 1);
        assert_eq!(retrieved.test_names[0], "test_myFunc");
    }

    #[test]
    fn test_metadata_operations() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        // Set metadata
        queries.set_metadata(db.conn(), "embedding_version", "1").unwrap();
        queries.set_metadata(db.conn(), "config_hash", "abc123").unwrap();

        // Get metadata
        let version = queries.get_metadata(db.conn(), "embedding_version").unwrap();
        assert_eq!(version, Some("1".to_string()));

        // Update existing
        queries.set_metadata(db.conn(), "embedding_version", "2").unwrap();
        let version = queries.get_metadata(db.conn(), "embedding_version").unwrap();
        assert_eq!(version, Some("2".to_string()));

        // Non-existent key
        let missing = queries.get_metadata(db.conn(), "nonexistent").unwrap();
        assert!(missing.is_none());

        // Delete
        queries.delete_metadata(db.conn(), "config_hash").unwrap();
        let deleted = queries.get_metadata(db.conn(), "config_hash").unwrap();
        assert!(deleted.is_none());
    }

    #[test]
    fn test_get_all_metadata() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        // Set multiple values
        queries.set_metadata(db.conn(), "key1", "value1").unwrap();
        queries.set_metadata(db.conn(), "key2", "value2").unwrap();

        // Get all
        let all = queries.get_all_metadata(db.conn()).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all.get("key1"), Some(&"value1".to_string()));
        assert_eq!(all.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_clear_all_graph_data() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let mut queries = QueryBuilder::new(db.conn()).unwrap();

        // Insert a node
        let node = create_test_node("n1", "foo");
        queries.insert_node(db.conn(), &node).unwrap();

        // Insert an edge (self-referential to keep it simple)
        let edge = Edge::new("n1", "n1", EdgeKind::Contains);
        queries.insert_edge(db.conn(), &edge).unwrap();

        // Insert a file record
        let file = FileRecord {
            path: "test.rs".to_string(),
            content_hash: "abc123".to_string(),
            language: Language::Rust,
            size: 100,
            modified_at: 12345,
            indexed_at: 12346,
            node_count: 1,
            errors: Vec::new(),
        };
        queries.upsert_file(db.conn(), &file).unwrap();

        // Insert an unresolved reference
        queries
            .insert_unresolved_reference(db.conn(), "n1", "SomeType", "test.rs", 5)
            .unwrap();

        // Verify data exists before clearing
        let stats = queries.get_stats(db.conn()).unwrap();
        assert!(stats.node_count > 0);
        assert!(stats.edge_count > 0);
        assert!(stats.file_count > 0);
        let unresolved_before: i32 = db.conn()
            .query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r| r.get(0))
            .unwrap();
        assert!(unresolved_before > 0);

        // Clear
        queries.clear_all_graph_data(db.conn()).unwrap();

        // Verify all tables empty
        let stats = queries.get_stats(db.conn()).unwrap();
        assert_eq!(stats.node_count, 0);
        assert_eq!(stats.edge_count, 0);
        assert_eq!(stats.file_count, 0);

        // Verify unresolved_refs also cleared
        let unresolved_count: i32 = db.conn()
            .query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(unresolved_count, 0);

        // Verify cache is not stale after clear
        let cached = queries.get_node_by_id(db.conn(), "n1").unwrap();
        assert!(cached.is_none(), "node should not be served from stale cache after clear");
    }
}
