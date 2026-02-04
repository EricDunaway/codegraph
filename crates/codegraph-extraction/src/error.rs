//! Extraction error types

use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during extraction
#[derive(Debug, Error)]
pub enum ExtractionError {
    /// File not found
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    /// File read error
    #[error("Failed to read file {path}: {message}")]
    FileRead { path: PathBuf, message: String },

    /// Parse error
    #[error("Parse error in {path} at {line}:{column}: {message}")]
    Parse {
        path: PathBuf,
        line: u32,
        column: u32,
        message: String,
    },

    /// Unsupported language
    #[error("Unsupported language: {0}")]
    UnsupportedLanguage(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Glob pattern error
    #[error("Invalid glob pattern: {0}")]
    GlobPattern(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),
}

impl ExtractionError {
    /// Create a parse error
    pub fn parse(path: impl Into<PathBuf>, line: u32, column: u32, message: impl Into<String>) -> Self {
        Self::Parse {
            path: path.into(),
            line,
            column,
            message: message.into(),
        }
    }

    /// Create a file read error
    pub fn file_read(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::FileRead {
            path: path.into(),
            message: message.into(),
        }
    }
}
