//! CodeGraph CLI
//!
//! Command-line interface for the CodeGraph code intelligence system.

mod commands;

use clap::{Parser, Subcommand};
use console::style;
use std::path::PathBuf;

/// CodeGraph - Local-first code intelligence
#[derive(Parser)]
#[command(name = "codegraph")]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize CodeGraph in a project
    Init {
        /// Path to the project (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Index all files in the project
    Index {
        /// Path to the project (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Incrementally sync changes
    Sync {
        /// Path to the project (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Show project statistics
    Status {
        /// Path to the project (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Search for symbols
    Query {
        /// Search query
        query: String,

        /// Maximum results
        #[arg(short, long, default_value = "20")]
        limit: usize,

        /// Path to the project (default: current directory)
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },

    /// Build context for a task
    Context {
        /// Task or query to build context for
        task: String,

        /// Maximum tokens in context
        #[arg(short, long, default_value = "8000")]
        max_tokens: usize,

        /// Path to the project (default: current directory)
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },

    /// Manage git hooks
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },

    /// Start MCP server
    Serve {
        /// Enable MCP mode (JSON-RPC over stdio)
        #[arg(long)]
        mcp: bool,

        /// Path to the project (default: current directory)
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Subcommand)]
enum HooksAction {
    /// Install git hooks for auto-sync
    Install {
        /// Path to the project (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Uninstall git hooks
    Uninstall {
        /// Path to the project (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Check if hooks are installed
    Status {
        /// Path to the project (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    // Set up logging
    if cli.verbose {
        tracing_subscriber::fmt()
            .with_env_filter("codegraph=debug")
            .init();
    }

    let result = match cli.command {
        Commands::Init { path } => commands::init(&path),
        Commands::Index { path } => commands::index(&path),
        Commands::Sync { path } => commands::sync(&path),
        Commands::Status { path } => commands::status(&path),
        Commands::Query { query, limit, path } => commands::query(&path, &query, limit),
        Commands::Context { task, max_tokens, path } => commands::context(&path, &task, max_tokens),
        Commands::Hooks { action } => match action {
            HooksAction::Install { path } => commands::hooks_install(&path),
            HooksAction::Uninstall { path } => commands::hooks_uninstall(&path),
            HooksAction::Status { path } => commands::hooks_status(&path),
        },
        Commands::Serve { mcp, path } => {
            if mcp {
                commands::serve_mcp(&path)
            } else {
                eprintln!("{} Use --mcp flag to start MCP server", style("Error:").red().bold());
                std::process::exit(1);
            }
        }
    };

    if let Err(e) = result {
        eprintln!("{} {}", style("Error:").red().bold(), e);
        std::process::exit(1);
    }
}
