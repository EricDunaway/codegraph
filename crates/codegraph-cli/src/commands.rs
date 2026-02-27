//! CLI command implementations

use codegraph_core::{CodeGraph, CodeGraphConfig, CodeGraphError, SyncOptions};
use codegraph_core::sync::SyncError;
use codegraph_mcp::McpServer;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use thiserror::Error;

/// CLI errors
#[derive(Debug, Error)]
pub enum CliError {
    #[error("{0}")]
    CodeGraph(#[from] CodeGraphError),

    #[error("MCP server error: {0}")]
    Mcp(#[from] codegraph_mcp::McpError),

    #[error("Sync error: {0}")]
    Sync(#[from] SyncError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

/// Initialize a CodeGraph project
pub fn init(path: &Path) -> Result<(), CliError> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    println!(
        "{} Initializing CodeGraph in {}",
        style("→").cyan().bold(),
        style(path.display()).blue()
    );

    let cg = CodeGraph::init(&path)?;

    println!(
        "{} Created .codegraph directory",
        style("✓").green().bold()
    );
    println!(
        "{} Database initialized at {}",
        style("✓").green().bold(),
        style(cg.config().db_path.display()).dim()
    );
    println!();
    println!(
        "Run {} to build the index.",
        style("codegraph index").cyan()
    );

    Ok(())
}

/// Index all files in a project
pub fn index(path: &Path) -> Result<(), CliError> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    println!(
        "{} Indexing {}",
        style("→").cyan().bold(),
        style(path.display()).blue()
    );

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    pb.set_message("Scanning files...");

    let mut cg = match CodeGraph::open(&path) {
        Ok(cg) => cg,
        Err(e) => {
            pb.finish_and_clear();
            return Err(e.into());
        }
    };
    pb.set_message("Extracting code...");

    let result = match cg.index_all() {
        Ok(r) => r,
        Err(e) => {
            pb.finish_and_clear();
            return Err(e.into());
        }
    };

    pb.finish_and_clear();

    println!(
        "{} Indexed {} files",
        style("✓").green().bold(),
        style(result.files_indexed).cyan()
    );
    println!(
        "   {} nodes, {} edges",
        style(result.nodes_created).cyan(),
        style(result.edges_created).cyan()
    );

    if result.references_resolved > 0 {
        println!(
            "   {} references resolved",
            style(result.references_resolved).cyan()
        );
    }

    if result.embeddings_generated > 0 {
        println!(
            "   {} embeddings generated",
            style(result.embeddings_generated).cyan()
        );
    }

    Ok(())
}

/// Sync changes incrementally
pub fn sync(path: &Path, hook: Option<String>, verify_sync: bool) -> Result<(), CliError> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let is_hook_mode = hook.is_some();

    if !is_hook_mode {
        println!(
            "{} Syncing {}",
            style("→").cyan().bold(),
            style(path.display()).blue()
        );
    }

    let pb = if !is_hook_mode {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap(),
        );
        pb.set_message("Checking for changes...");
        Some(pb)
    } else {
        None
    };

    let mut cg = match CodeGraph::open(&path) {
        Ok(cg) => cg,
        Err(e) => {
            if let Some(pb) = &pb { pb.finish_and_clear(); }
            if is_hook_mode {
                // Hook mode: never fail the git operation — sync_with_options
                // would have written sync.failed, but we couldn't even open.
                // Write sync.failed directly since no CodeGraph instance exists.
                let codegraph_dir = path.join(".codegraph");
                if codegraph_dir.exists() {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let hook_name = hook.as_deref().unwrap_or("unknown");
                    let _ = std::fs::write(
                        codegraph_dir.join("sync.failed"),
                        format!("{} hook={} error={}", now, hook_name, e),
                    );
                }
                return Ok(());
            }
            return Err(e.into());
        }
    };

    let opts = SyncOptions {
        hook_name: hook.clone(),
        file_list: None,
        verify_sync,
    };

    let full_result = match cg.sync_with_options(opts) {
        Ok(r) => r,
        Err(e) => {
            if let Some(pb) = &pb { pb.finish_and_clear(); }
            if is_hook_mode {
                // Hook mode: write sync.failed for visibility, then exit OK.
                let codegraph_dir = path.join(".codegraph");
                if codegraph_dir.exists() {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let hook_name = hook.as_deref().unwrap_or("unknown");
                    let _ = std::fs::write(
                        codegraph_dir.join("sync.failed"),
                        format!("{} hook={} error={}", now, hook_name, e),
                    );
                }
                return Ok(());
            }
            return Err(e.into());
        }
    };

    if let Some(pb) = &pb { pb.finish_and_clear(); }

    // In hook mode, stay silent on success
    if is_hook_mode {
        return Ok(());
    }

    let result = &full_result.sync;
    let embed = &full_result.embeddings;

    if !result.had_changes && !embed.full_reembed {
        println!(
            "{} Already up to date",
            style("✓").green().bold()
        );
    } else {
        let stats = &result.stats;
        println!(
            "{} Sync complete in {}ms",
            style("✓").green().bold(),
            result.duration_ms
        );
        if stats.files_added > 0 {
            println!("   {} files added", style(stats.files_added).green());
        }
        if stats.files_modified > 0 {
            println!("   {} files modified", style(stats.files_modified).yellow());
        }
        if stats.files_deleted > 0 {
            println!("   {} files deleted", style(stats.files_deleted).red());
        }
        println!(
            "   {} total nodes",
            style(stats.total_nodes).cyan()
        );

        // Embedding stats
        if embed.skipped_no_model {
            println!("   {} embeddings skipped (model unavailable)", style("—").dim());
        } else {
            let embed_total = embed.vectors_created + embed.vectors_updated + embed.vectors_deleted;
            if embed_total > 0 || embed.full_reembed {
                if embed.full_reembed {
                    println!("   {} embeddings regenerated (full)", style(embed.vectors_created).cyan());
                } else {
                    if embed.vectors_created > 0 {
                        println!("   {} embeddings created", style(embed.vectors_created).green());
                    }
                    if embed.vectors_updated > 0 {
                        println!("   {} embeddings updated", style(embed.vectors_updated).yellow());
                    }
                    if embed.vectors_deleted > 0 {
                        println!("   {} embeddings deleted", style(embed.vectors_deleted).red());
                    }
                }
            }
        }
    }

    Ok(())
}

/// Show project status
pub fn status(path: &Path) -> Result<(), CliError> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    let cg = CodeGraph::open(&path)?;
    let stats = cg.get_stats()?;

    println!(
        "{} CodeGraph Status for {}",
        style("📊").cyan(),
        style(path.display()).blue()
    );
    println!();
    println!(
        "   {} files indexed",
        style(stats.file_count).cyan().bold()
    );
    println!(
        "   {} symbols (nodes)",
        style(stats.node_count).cyan().bold()
    );
    println!(
        "   {} relationships (edges)",
        style(stats.edge_count).cyan().bold()
    );
    println!();
    println!(
        "   Database: {}",
        style(cg.config().db_path.display()).dim()
    );

    // Surface sync.failed if present
    let failed_path = cg.config().data_dir.join("sync.failed");
    if failed_path.exists() {
        println!();
        match std::fs::read_to_string(&failed_path) {
            Ok(contents) => {
                println!(
                    "{} Last background sync failed:",
                    style("!").red().bold()
                );
                for line in contents.lines() {
                    println!("   {}", style(line).red());
                }
                println!(
                    "   {}",
                    style("Run 'codegraph sync' to retry.").dim()
                );
            }
            Err(_) => {
                println!(
                    "{} sync.failed marker exists but could not be read",
                    style("!").yellow().bold()
                );
            }
        }
    }

    Ok(())
}

/// Search for symbols
pub fn query(path: &Path, query: &str, limit: usize) -> Result<(), CliError> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    let cg = CodeGraph::open(&path)?;
    let results = cg.search(query, limit)?;

    if results.is_empty() {
        println!(
            "{} No results found for '{}'",
            style("!").yellow().bold(),
            style(query).cyan()
        );
        return Ok(());
    }

    println!(
        "{} Found {} results for '{}':",
        style("🔍").cyan(),
        style(results.len()).cyan().bold(),
        style(query).cyan()
    );
    println!();

    for result in results {
        let kind = format!("{:?}", result.node.kind).to_lowercase();
        println!(
            "  {} {} {}",
            style(&kind).dim(),
            style(&result.node.qualified_name).cyan().bold(),
            style(format!("({}:{})", result.node.file_path, result.node.start_line)).dim()
        );
    }

    Ok(())
}

/// Build context for a task
pub fn context(path: &Path, task: &str, max_tokens: usize) -> Result<(), CliError> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    let mut cg = CodeGraph::open(&path)?;

    let options = codegraph_core::context::ContextOptions {
        max_tokens,
        ..Default::default()
    };

    let result = cg.build_context_with_options(task, options)?;

    // Print the formatted output
    println!("{}", result.output);

    // Print summary
    let node_count = result.context.nodes.len();
    let estimated_tokens = result.output.len() / 4; // rough estimate
    eprintln!();
    eprintln!(
        "{} ~{} tokens, {} nodes included",
        style("📋").cyan(),
        style(estimated_tokens).cyan().bold(),
        style(node_count).cyan()
    );
    if result.truncated {
        eprintln!(
            "   {} {} nodes excluded due to limits",
            style("!").yellow(),
            style(result.excluded_count).yellow()
        );
    }

    Ok(())
}

/// Install git hooks
pub fn hooks_install(path: &Path, force: bool) -> Result<(), CliError> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // Check if git repo
    let git_dir = path.join(".git");
    if !git_dir.exists() {
        return Err(CliError::Other(
            "Not a git repository. Git hooks require a .git directory.".to_string(),
        ));
    }

    let manager = codegraph_core::sync::GitHooksManager::new(&path, force)?;
    manager.install_all()?;

    // Ensure .codegraph/ is in .gitignore
    {
        let gitignore = path.join(".gitignore");
        let entry = ".codegraph/";
        if !gitignore.exists() || !std::fs::read_to_string(&gitignore).map(|c| c.lines().any(|l| l.trim() == entry)).unwrap_or(false) {
            let _ = std::fs::OpenOptions::new().create(true).append(true).open(&gitignore)
                .and_then(|mut f| { use std::io::Write; writeln!(f, "{}", entry) });
        }
    }

    println!(
        "{} Git hooks installed",
        style("✓").green().bold()
    );
    println!("   CodeGraph will auto-sync on commits.");

    Ok(())
}

/// Uninstall git hooks
pub fn hooks_uninstall(path: &Path) -> Result<(), CliError> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    let manager = codegraph_core::sync::GitHooksManager::new(&path, false)?;
    manager.uninstall_all()?;

    println!(
        "{} Git hooks removed",
        style("✓").green().bold()
    );

    Ok(())
}

/// Check git hooks status
pub fn hooks_status(path: &Path) -> Result<(), CliError> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    let manager = codegraph_core::sync::GitHooksManager::new(&path, false)?;

    println!(
        "{} Git Hooks Status",
        style("🔗").cyan()
    );
    println!();

    let installed = manager.is_installed();
    let status = if installed {
        style("installed").green()
    } else {
        style("not installed").dim()
    };
    println!("   CodeGraph hooks: {}", status);

    Ok(())
}

/// Start MCP server
pub fn serve_mcp(path: &Path) -> Result<(), CliError> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // Verify project is initialized
    let config = CodeGraphConfig::new(path);
    if !config.data_dir.exists() {
        return Err(CliError::Other(
            "Project not initialized. Run 'codegraph init' first.".to_string(),
        ));
    }

    let db_path = config.db_path.to_string_lossy().to_string();
    let mut server = McpServer::new(&db_path)?;

    // Run the server (blocks until stdin closes)
    server.run()?;

    Ok(())
}
