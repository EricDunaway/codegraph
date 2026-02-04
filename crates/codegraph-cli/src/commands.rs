//! CLI command implementations

use codegraph_core::{CodeGraph, CodeGraphConfig, CodeGraphError};
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

    let mut cg = CodeGraph::open(&path)?;
    pb.set_message("Extracting code...");

    let result = cg.index_all()?;

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

    Ok(())
}

/// Sync changes incrementally
pub fn sync(path: &Path) -> Result<(), CliError> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    println!(
        "{} Syncing {}",
        style("→").cyan().bold(),
        style(path.display()).blue()
    );

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    pb.set_message("Checking for changes...");

    // For now, just do a full re-index
    // TODO: Implement proper incremental sync
    let mut cg = CodeGraph::open(&path)?;
    let result = cg.index_all()?;

    pb.finish_and_clear();

    println!(
        "{} Sync complete",
        style("✓").green().bold()
    );
    println!(
        "   {} files, {} nodes",
        style(result.files_indexed).cyan(),
        style(result.nodes_created).cyan()
    );

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
pub fn hooks_install(path: &Path) -> Result<(), CliError> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // Check if git repo
    let git_dir = path.join(".git");
    if !git_dir.exists() {
        return Err(CliError::Other(
            "Not a git repository. Git hooks require a .git directory.".to_string(),
        ));
    }

    let manager = codegraph_core::sync::GitHooksManager::new(&path)?;
    manager.install_all()?;

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

    let manager = codegraph_core::sync::GitHooksManager::new(&path)?;
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

    let manager = codegraph_core::sync::GitHooksManager::new(&path)?;

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
