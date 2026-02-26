//! Reference resolution for CodeGraph
//!
//! This crate handles resolving unresolved references after extraction,
//! using name matching, import resolution, and framework-specific patterns.

pub mod error;
pub mod resolver;
pub mod matcher;

pub use error::ResolutionError;
pub use resolver::{ReferenceResolver, ResolutionResult, ResolutionStats, ScopedResolutionResult};
pub use matcher::NameMatcher;
