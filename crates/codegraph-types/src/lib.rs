//! CodeGraph Type Definitions
//!
//! Core types for the semantic knowledge graph system.
//! This crate contains shared types used across all CodeGraph crates.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// =============================================================================
// Node Identifier
// =============================================================================

/// Unique identifier for a node (32-char hex hash of file path + qualified name)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub String);

impl NodeId {
    /// Create a new NodeId from a string
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the inner string reference
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for NodeId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for NodeId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

// =============================================================================
// Node Kind
// =============================================================================

/// Types of nodes in the knowledge graph
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// Source file
    File,
    /// Module or package
    Module,
    /// Class definition
    Class,
    /// Struct definition
    Struct,
    /// Interface definition
    Interface,
    /// Trait definition (Rust)
    Trait,
    /// Protocol definition (Swift)
    Protocol,
    /// Function definition
    Function,
    /// Method definition
    Method,
    /// Property (getter/setter)
    Property,
    /// Field (class/struct member)
    Field,
    /// Variable
    Variable,
    /// Constant
    Constant,
    /// Enum definition
    Enum,
    /// Enum member/variant
    EnumMember,
    /// Type alias
    TypeAlias,
    /// Namespace
    Namespace,
    /// Function/method parameter
    Parameter,
    /// Import statement
    Import,
    /// Export statement
    Export,
    /// Route definition (web frameworks)
    Route,
    /// UI Component (React, Flutter, SwiftUI)
    Component,
}

impl NodeKind {
    /// Get all node kinds
    pub const fn all() -> &'static [NodeKind] {
        &[
            NodeKind::File,
            NodeKind::Module,
            NodeKind::Class,
            NodeKind::Struct,
            NodeKind::Interface,
            NodeKind::Trait,
            NodeKind::Protocol,
            NodeKind::Function,
            NodeKind::Method,
            NodeKind::Property,
            NodeKind::Field,
            NodeKind::Variable,
            NodeKind::Constant,
            NodeKind::Enum,
            NodeKind::EnumMember,
            NodeKind::TypeAlias,
            NodeKind::Namespace,
            NodeKind::Parameter,
            NodeKind::Import,
            NodeKind::Export,
            NodeKind::Route,
            NodeKind::Component,
        ]
    }

    /// Get string representation
    pub const fn as_str(&self) -> &'static str {
        match self {
            NodeKind::File => "file",
            NodeKind::Module => "module",
            NodeKind::Class => "class",
            NodeKind::Struct => "struct",
            NodeKind::Interface => "interface",
            NodeKind::Trait => "trait",
            NodeKind::Protocol => "protocol",
            NodeKind::Function => "function",
            NodeKind::Method => "method",
            NodeKind::Property => "property",
            NodeKind::Field => "field",
            NodeKind::Variable => "variable",
            NodeKind::Constant => "constant",
            NodeKind::Enum => "enum",
            NodeKind::EnumMember => "enum_member",
            NodeKind::TypeAlias => "type_alias",
            NodeKind::Namespace => "namespace",
            NodeKind::Parameter => "parameter",
            NodeKind::Import => "import",
            NodeKind::Export => "export",
            NodeKind::Route => "route",
            NodeKind::Component => "component",
        }
    }
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for NodeKind {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "file" => Ok(NodeKind::File),
            "module" => Ok(NodeKind::Module),
            "class" => Ok(NodeKind::Class),
            "struct" => Ok(NodeKind::Struct),
            "interface" => Ok(NodeKind::Interface),
            "trait" => Ok(NodeKind::Trait),
            "protocol" => Ok(NodeKind::Protocol),
            "function" => Ok(NodeKind::Function),
            "method" => Ok(NodeKind::Method),
            "property" => Ok(NodeKind::Property),
            "field" => Ok(NodeKind::Field),
            "variable" => Ok(NodeKind::Variable),
            "constant" => Ok(NodeKind::Constant),
            "enum" => Ok(NodeKind::Enum),
            "enum_member" => Ok(NodeKind::EnumMember),
            "type_alias" => Ok(NodeKind::TypeAlias),
            "namespace" => Ok(NodeKind::Namespace),
            "parameter" => Ok(NodeKind::Parameter),
            "import" => Ok(NodeKind::Import),
            "export" => Ok(NodeKind::Export),
            "route" => Ok(NodeKind::Route),
            "component" => Ok(NodeKind::Component),
            _ => Err(ParseError::InvalidNodeKind(s.to_string())),
        }
    }
}

// =============================================================================
// Edge Kind
// =============================================================================

/// Types of edges (relationships) between nodes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Parent contains child (file→class, class→method)
    Contains,
    /// Function/method calls another
    Calls,
    /// File imports from another
    Imports,
    /// File exports a symbol
    Exports,
    /// Class/interface extends another
    Extends,
    /// Class implements interface
    Implements,
    /// Generic reference to another symbol
    References,
    /// Variable/parameter has type
    TypeOf,
    /// Function returns type
    Returns,
    /// Creates instance of class
    Instantiates,
    /// Method overrides parent method
    Overrides,
    /// Decorator applied to symbol
    Decorates,
}

impl EdgeKind {
    /// Get all edge kinds
    pub const fn all() -> &'static [EdgeKind] {
        &[
            EdgeKind::Contains,
            EdgeKind::Calls,
            EdgeKind::Imports,
            EdgeKind::Exports,
            EdgeKind::Extends,
            EdgeKind::Implements,
            EdgeKind::References,
            EdgeKind::TypeOf,
            EdgeKind::Returns,
            EdgeKind::Instantiates,
            EdgeKind::Overrides,
            EdgeKind::Decorates,
        ]
    }

    /// Get string representation
    pub const fn as_str(&self) -> &'static str {
        match self {
            EdgeKind::Contains => "contains",
            EdgeKind::Calls => "calls",
            EdgeKind::Imports => "imports",
            EdgeKind::Exports => "exports",
            EdgeKind::Extends => "extends",
            EdgeKind::Implements => "implements",
            EdgeKind::References => "references",
            EdgeKind::TypeOf => "type_of",
            EdgeKind::Returns => "returns",
            EdgeKind::Instantiates => "instantiates",
            EdgeKind::Overrides => "overrides",
            EdgeKind::Decorates => "decorates",
        }
    }

    /// Check if this is a structural relationship (not semantic)
    pub const fn is_structural(&self) -> bool {
        matches!(self, EdgeKind::Contains | EdgeKind::Imports | EdgeKind::Exports)
    }
}

impl std::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for EdgeKind {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "contains" => Ok(EdgeKind::Contains),
            "calls" => Ok(EdgeKind::Calls),
            "imports" => Ok(EdgeKind::Imports),
            "exports" => Ok(EdgeKind::Exports),
            "extends" => Ok(EdgeKind::Extends),
            "implements" => Ok(EdgeKind::Implements),
            "references" => Ok(EdgeKind::References),
            "type_of" => Ok(EdgeKind::TypeOf),
            "returns" => Ok(EdgeKind::Returns),
            "instantiates" => Ok(EdgeKind::Instantiates),
            "overrides" => Ok(EdgeKind::Overrides),
            "decorates" => Ok(EdgeKind::Decorates),
            _ => Err(ParseError::InvalidEdgeKind(s.to_string())),
        }
    }
}

// =============================================================================
// Language
// =============================================================================

/// Supported programming languages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    // === Primary Targets (first-class support with framework patterns) ===
    /// TypeScript
    TypeScript,
    /// JavaScript
    JavaScript,
    /// TypeScript with JSX
    Tsx,
    /// JavaScript with JSX
    Jsx,
    /// Rust
    Rust,
    /// PHP
    Php,
    /// Dart (Flutter, AngularDart)
    Dart,
    /// GraphQL (standalone + embedded extraction)
    GraphQL,
    /// Shell scripts (Bash)
    Bash,
    /// HashiCorp Configuration Language (Terraform, Vault, Nomad, Packer)
    Hcl,

    // === Secondary (extraction only, no framework patterns) ===
    /// Python
    Python,
    /// Go
    Go,
    /// Java
    Java,
    /// C
    C,
    /// C++
    Cpp,
    /// C#
    CSharp,
    /// Ruby
    Ruby,
    /// Swift
    Swift,
    /// Kotlin
    Kotlin,

    /// Unknown language
    Unknown,
}

impl Language {
    /// Get the file extensions for this language
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            Language::TypeScript => &["ts", "mts", "cts"],
            Language::JavaScript => &["js", "mjs", "cjs"],
            Language::Tsx => &["tsx"],
            Language::Jsx => &["jsx"],
            Language::Rust => &["rs"],
            Language::Php => &["php"],
            Language::Dart => &["dart"],
            Language::GraphQL => &["graphql", "gql"],
            Language::Bash => &["sh", "bash"],
            Language::Hcl => &["tf", "tfvars", "hcl"],
            Language::Python => &["py", "pyi"],
            Language::Go => &["go"],
            Language::Java => &["java"],
            Language::C => &["c", "h"],
            Language::Cpp => &["cpp", "hpp", "cc", "cxx", "hxx"],
            Language::CSharp => &["cs"],
            Language::Ruby => &["rb"],
            Language::Swift => &["swift"],
            Language::Kotlin => &["kt", "kts"],
            Language::Unknown => &[],
        }
    }

    /// Detect language from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "ts" | "mts" | "cts" => Language::TypeScript,
            "js" | "mjs" | "cjs" => Language::JavaScript,
            "tsx" => Language::Tsx,
            "jsx" => Language::Jsx,
            "rs" => Language::Rust,
            "php" => Language::Php,
            "dart" => Language::Dart,
            "graphql" | "gql" => Language::GraphQL,
            "sh" | "bash" => Language::Bash,
            "tf" | "tfvars" | "hcl" => Language::Hcl,
            "py" | "pyi" => Language::Python,
            "go" => Language::Go,
            "java" => Language::Java,
            "c" | "h" => Language::C,
            "cpp" | "hpp" | "cc" | "cxx" | "hxx" => Language::Cpp,
            "cs" => Language::CSharp,
            "rb" => Language::Ruby,
            "swift" => Language::Swift,
            "kt" | "kts" => Language::Kotlin,
            _ => Language::Unknown,
        }
    }

    /// Get string representation
    pub const fn as_str(&self) -> &'static str {
        match self {
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Tsx => "tsx",
            Language::Jsx => "jsx",
            Language::Rust => "rust",
            Language::Php => "php",
            Language::Dart => "dart",
            Language::GraphQL => "graphql",
            Language::Bash => "bash",
            Language::Hcl => "hcl",
            Language::Python => "python",
            Language::Go => "go",
            Language::Java => "java",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::CSharp => "csharp",
            Language::Ruby => "ruby",
            Language::Swift => "swift",
            Language::Kotlin => "kotlin",
            Language::Unknown => "unknown",
        }
    }

    /// Check if this is a primary target language (with framework support)
    pub const fn is_primary(&self) -> bool {
        matches!(
            self,
            Language::TypeScript
                | Language::JavaScript
                | Language::Tsx
                | Language::Jsx
                | Language::Rust
                | Language::Php
                | Language::Dart
                | Language::GraphQL
                | Language::Bash
                | Language::Hcl
        )
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for Language {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "typescript" | "ts" => Ok(Language::TypeScript),
            "javascript" | "js" => Ok(Language::JavaScript),
            "tsx" => Ok(Language::Tsx),
            "jsx" => Ok(Language::Jsx),
            "rust" | "rs" => Ok(Language::Rust),
            "php" => Ok(Language::Php),
            "dart" => Ok(Language::Dart),
            "graphql" | "gql" => Ok(Language::GraphQL),
            "bash" | "sh" | "shell" => Ok(Language::Bash),
            "hcl" | "terraform" | "tf" => Ok(Language::Hcl),
            "python" | "py" => Ok(Language::Python),
            "go" | "golang" => Ok(Language::Go),
            "java" => Ok(Language::Java),
            "c" => Ok(Language::C),
            "cpp" | "c++" => Ok(Language::Cpp),
            "csharp" | "c#" | "cs" => Ok(Language::CSharp),
            "ruby" | "rb" => Ok(Language::Ruby),
            "swift" => Ok(Language::Swift),
            "kotlin" | "kt" => Ok(Language::Kotlin),
            "unknown" => Ok(Language::Unknown),
            _ => Err(ParseError::InvalidLanguage(s.to_string())),
        }
    }
}

// =============================================================================
// Visibility
// =============================================================================

/// Visibility modifier for symbols
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// Public (accessible from anywhere)
    Public,
    /// Private (accessible only within the defining scope)
    Private,
    /// Protected (accessible within class and subclasses)
    Protected,
    /// Internal (accessible within module/package)
    Internal,
}

impl Visibility {
    /// Get string representation
    pub const fn as_str(&self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Private => "private",
            Visibility::Protected => "protected",
            Visibility::Internal => "internal",
        }
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for Visibility {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "public" | "pub" => Ok(Visibility::Public),
            "private" | "priv" => Ok(Visibility::Private),
            "protected" => Ok(Visibility::Protected),
            "internal" => Ok(Visibility::Internal),
            _ => Err(ParseError::InvalidVisibility(s.to_string())),
        }
    }
}

// =============================================================================
// Node
// =============================================================================

/// A node in the knowledge graph representing a code symbol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Unique identifier (hash of file path + qualified name)
    pub id: NodeId,

    /// Type of code element
    pub kind: NodeKind,

    /// Simple name (e.g., "calculateTotal")
    pub name: String,

    /// Fully qualified name (e.g., "src/utils.ts::MathHelper.calculateTotal")
    pub qualified_name: String,

    /// File path relative to project root
    pub file_path: String,

    /// Programming language
    pub language: Language,

    /// Starting line number (1-indexed)
    pub start_line: u32,

    /// Ending line number (1-indexed)
    pub end_line: u32,

    /// Starting column (0-indexed)
    pub start_column: u32,

    /// Ending column (0-indexed)
    pub end_column: u32,

    /// Documentation string if present
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,

    /// Function/method signature
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,

    /// Visibility modifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,

    /// Whether symbol is exported
    #[serde(default)]
    pub is_exported: bool,

    /// Whether symbol is async
    #[serde(default)]
    pub is_async: bool,

    /// Whether symbol is static
    #[serde(default)]
    pub is_static: bool,

    /// Whether symbol is abstract
    #[serde(default)]
    pub is_abstract: bool,

    /// Decorators/annotations applied
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decorators: Vec<String>,

    /// Generic type parameters
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_parameters: Vec<String>,

    /// When the node was last updated (Unix timestamp)
    pub updated_at: i64,
}

impl Node {
    /// Create a new node with required fields
    pub fn new(
        id: impl Into<NodeId>,
        kind: NodeKind,
        name: impl Into<String>,
        qualified_name: impl Into<String>,
        file_path: impl Into<String>,
        language: Language,
        start_line: u32,
        end_line: u32,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            name: name.into(),
            qualified_name: qualified_name.into(),
            file_path: file_path.into(),
            language,
            start_line,
            end_line,
            start_column: 0,
            end_column: 0,
            docstring: None,
            signature: None,
            visibility: None,
            is_exported: false,
            is_async: false,
            is_static: false,
            is_abstract: false,
            decorators: Vec::new(),
            type_parameters: Vec::new(),
            updated_at: 0,
        }
    }

    /// Get the line count for this node
    pub fn line_count(&self) -> u32 {
        self.end_line.saturating_sub(self.start_line) + 1
    }
}

// =============================================================================
// Edge
// =============================================================================

/// An edge representing a relationship between two nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// Database row ID (set after insertion)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,

    /// Source node ID
    pub source: NodeId,

    /// Target node ID
    pub target: NodeId,

    /// Type of relationship
    pub kind: EdgeKind,

    /// Additional context about the relationship
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// Line number where relationship occurs (e.g., call site)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,

    /// Column number where relationship occurs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

impl Edge {
    /// Create a new edge
    pub fn new(source: impl Into<NodeId>, target: impl Into<NodeId>, kind: EdgeKind) -> Self {
        Self {
            id: None,
            source: source.into(),
            target: target.into(),
            kind,
            metadata: None,
            line: None,
            column: None,
        }
    }

    /// Create an edge with location information
    pub fn with_location(
        source: impl Into<NodeId>,
        target: impl Into<NodeId>,
        kind: EdgeKind,
        line: u32,
        column: u32,
    ) -> Self {
        Self {
            id: None,
            source: source.into(),
            target: target.into(),
            kind,
            metadata: None,
            line: Some(line),
            column: Some(column),
        }
    }
}

// =============================================================================
// File Record
// =============================================================================

/// Metadata about a tracked file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    /// File path relative to project root
    pub path: String,

    /// Content hash for change detection (SHA-256)
    pub content_hash: String,

    /// Detected language
    pub language: Language,

    /// File size in bytes
    pub size: u64,

    /// Last modification timestamp (Unix)
    pub modified_at: i64,

    /// When last indexed (Unix)
    pub indexed_at: i64,

    /// Number of nodes extracted
    pub node_count: u32,

    /// Any extraction errors
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ExtractionError>,
}

// =============================================================================
// Extraction Types
// =============================================================================

/// Result from parsing a source file
#[derive(Debug, Clone, Default)]
pub struct ExtractionResult {
    /// Extracted nodes
    pub nodes: Vec<Node>,

    /// Extracted edges
    pub edges: Vec<Edge>,

    /// References that couldn't be resolved yet
    pub unresolved_references: Vec<UnresolvedReference>,

    /// Any errors during extraction
    pub errors: Vec<ExtractionError>,

    /// Extraction duration in milliseconds
    pub duration_ms: u64,
}

/// Error during code extraction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionError {
    /// Error message
    pub message: String,

    /// Line number if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,

    /// Column number if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,

    /// Error severity
    pub severity: ErrorSeverity,

    /// Error code for categorization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// Severity level for extraction errors
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorSeverity {
    /// Non-critical issue
    Warning,
    /// Critical issue that prevented extraction
    Error,
}

/// A reference that couldn't be resolved during extraction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnresolvedReference {
    /// ID of the node containing the reference
    pub from_node_id: NodeId,

    /// Name being referenced
    pub reference_name: String,

    /// Type of reference (call, type, import, etc.)
    pub reference_kind: EdgeKind,

    /// Line number of the reference
    pub line: u32,

    /// Column number of the reference
    pub column: u32,

    /// Possible qualified names it might resolve to
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<String>,
}

// =============================================================================
// Query Types
// =============================================================================

/// A subgraph containing a subset of the knowledge graph
#[derive(Debug, Clone, Default)]
pub struct Subgraph {
    /// Nodes in this subgraph (keyed by node ID)
    pub nodes: HashMap<NodeId, Node>,

    /// Edges in this subgraph
    pub edges: Vec<Edge>,

    /// Root node IDs (entry points)
    pub roots: Vec<NodeId>,
}

impl Subgraph {
    /// Create an empty subgraph
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node to the subgraph
    pub fn add_node(&mut self, node: Node) {
        self.nodes.insert(node.id.clone(), node);
    }

    /// Add an edge to the subgraph
    pub fn add_edge(&mut self, edge: Edge) {
        self.edges.push(edge);
    }

    /// Get the node count
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Get the edge count
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

/// Options for graph traversal
#[derive(Debug, Clone, Default)]
pub struct TraversalOptions {
    /// Maximum depth to traverse (default: no limit)
    pub max_depth: Option<u32>,

    /// Edge types to follow (default: all)
    pub edge_kinds: Option<Vec<EdgeKind>>,

    /// Node types to include (default: all)
    pub node_kinds: Option<Vec<NodeKind>>,

    /// Direction of traversal
    pub direction: TraversalDirection,

    /// Maximum nodes to return
    pub limit: Option<usize>,

    /// Whether to include the starting node
    pub include_start: bool,
}

/// Direction for graph traversal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TraversalDirection {
    /// Follow outgoing edges (source → target)
    #[default]
    Outgoing,
    /// Follow incoming edges (target → source)
    Incoming,
    /// Follow both directions
    Both,
}

/// A search result with relevance scoring
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Matching node
    pub node: Node,

    /// Relevance score (0.0-1.0)
    pub score: f32,

    /// Matched text snippets for highlighting
    pub highlights: Vec<String>,
}

// =============================================================================
// Configuration Types
// =============================================================================

/// Configuration for a CodeGraph project
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Schema version for migrations
    pub version: u32,

    /// Root directory of the project
    pub root_dir: String,

    /// Glob patterns for files to include
    pub include: Vec<String>,

    /// Glob patterns for files to exclude
    pub exclude: Vec<String>,

    /// Languages to process (auto-detected if empty)
    pub languages: Vec<Language>,

    /// Framework hints for better extraction
    pub frameworks: Vec<FrameworkHint>,

    /// Maximum file size to process (in bytes)
    pub max_file_size: u64,

    /// Whether to extract docstrings
    pub extract_docstrings: bool,

    /// Whether to track call sites
    pub track_call_sites: bool,

    /// Whether to compute embeddings for semantic search
    pub enable_embeddings: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            root_dir: ".".to_string(),
            include: DEFAULT_INCLUDE.iter().map(|s| (*s).to_string()).collect(),
            exclude: DEFAULT_EXCLUDE.iter().map(|s| (*s).to_string()).collect(),
            languages: Vec::new(),
            frameworks: Vec::new(),
            max_file_size: 1024 * 1024, // 1MB
            extract_docstrings: true,
            track_call_sites: true,
            enable_embeddings: false,
        }
    }
}

/// Framework-specific hints for better extraction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameworkHint {
    /// Framework name (react, express, django, etc.)
    pub name: String,

    /// Version constraint if relevant
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Custom patterns for this framework
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patterns: Option<FrameworkPatterns>,
}

/// Custom patterns for framework detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameworkPatterns {
    /// Component detection patterns
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<String>>,

    /// Route detection patterns
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routes: Option<Vec<String>>,

    /// Model detection patterns
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
}

/// Statistics about the knowledge graph
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphStats {
    /// Total number of nodes
    pub node_count: u64,

    /// Total number of edges
    pub edge_count: u64,

    /// Number of tracked files
    pub file_count: u64,

    /// Node counts by kind
    pub nodes_by_kind: HashMap<NodeKind, u64>,

    /// Edge counts by kind
    pub edges_by_kind: HashMap<EdgeKind, u64>,

    /// File counts by language
    pub files_by_language: HashMap<Language, u64>,

    /// Database size in bytes
    pub db_size_bytes: u64,

    /// Last update timestamp
    pub last_updated: i64,
}

// =============================================================================
// Error Types
// =============================================================================

/// Parse error for string-to-enum conversions
#[derive(Debug, Clone)]
pub enum ParseError {
    /// Invalid node kind string
    InvalidNodeKind(String),
    /// Invalid edge kind string
    InvalidEdgeKind(String),
    /// Invalid language string
    InvalidLanguage(String),
    /// Invalid visibility string
    InvalidVisibility(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::InvalidNodeKind(s) => write!(f, "invalid node kind: {s}"),
            ParseError::InvalidEdgeKind(s) => write!(f, "invalid edge kind: {s}"),
            ParseError::InvalidLanguage(s) => write!(f, "invalid language: {s}"),
            ParseError::InvalidVisibility(s) => write!(f, "invalid visibility: {s}"),
        }
    }
}

impl std::error::Error for ParseError {}

// =============================================================================
// Default Patterns
// =============================================================================

/// Default file include patterns
pub const DEFAULT_INCLUDE: &[&str] = &[
    // TypeScript/JavaScript
    "**/*.ts",
    "**/*.tsx",
    "**/*.js",
    "**/*.jsx",
    "**/*.mts",
    "**/*.cts",
    "**/*.mjs",
    "**/*.cjs",
    // Rust
    "**/*.rs",
    // PHP
    "**/*.php",
    // Dart
    "**/*.dart",
    // GraphQL
    "**/*.graphql",
    "**/*.gql",
    // Bash
    "**/*.sh",
    "**/*.bash",
    // Terraform/HCL
    "**/*.tf",
    "**/*.tfvars",
    "**/*.hcl",
    // Python
    "**/*.py",
    "**/*.pyi",
    // Go
    "**/*.go",
    // Java
    "**/*.java",
    // C/C++
    "**/*.c",
    "**/*.h",
    "**/*.cpp",
    "**/*.hpp",
    "**/*.cc",
    "**/*.cxx",
    // C#
    "**/*.cs",
    // Ruby
    "**/*.rb",
    // Swift
    "**/*.swift",
    // Kotlin
    "**/*.kt",
    "**/*.kts",
];

/// Default file exclude patterns (75+ patterns)
pub const DEFAULT_EXCLUDE: &[&str] = &[
    // Version control
    "**/.git/**",
    // Dependencies
    "**/node_modules/**",
    "**/vendor/**",
    "**/Pods/**",
    // Generic build outputs
    "**/dist/**",
    "**/build/**",
    "**/out/**",
    "**/bin/**",
    "**/obj/**",
    "**/target/**",
    // JavaScript/TypeScript frameworks
    "**/*.min.js",
    "**/*.bundle.js",
    "**/.next/**",
    "**/.nuxt/**",
    "**/.svelte-kit/**",
    "**/.output/**",
    "**/.turbo/**",
    "**/.cache/**",
    "**/.parcel-cache/**",
    "**/.vite/**",
    "**/.astro/**",
    "**/.docusaurus/**",
    "**/.gatsby/**",
    "**/.webpack/**",
    "**/.nx/**",
    "**/.yarn/cache/**",
    "**/.pnpm-store/**",
    "**/storybook-static/**",
    // React Native / Expo
    "**/.expo/**",
    "**/web-build/**",
    "**/ios/Pods/**",
    "**/ios/build/**",
    "**/android/build/**",
    "**/android/.gradle/**",
    // Python
    "**/__pycache__/**",
    "**/.venv/**",
    "**/venv/**",
    "**/.pytest_cache/**",
    "**/.mypy_cache/**",
    "**/.ruff_cache/**",
    "**/.tox/**",
    "**/.nox/**",
    "**/*.egg-info/**",
    "**/.eggs/**",
    // Go
    "**/go/pkg/mod/**",
    // Rust
    "**/target/debug/**",
    "**/target/release/**",
    // Java/Kotlin/Gradle
    "**/.gradle/**",
    "**/.m2/**",
    "**/generated-sources/**",
    "**/.kotlin/**",
    // C#/.NET
    "**/.vs/**",
    "**/.nuget/**",
    "**/artifacts/**",
    "**/publish/**",
    // C/C++
    "**/cmake-build-*/**",
    "**/CMakeFiles/**",
    "**/bazel-*/**",
    "**/vcpkg_installed/**",
    "**/.conan/**",
    "**/Debug/**",
    "**/Release/**",
    "**/x64/**",
    // Electron
    "**/release/**",
    "**/*.app/**",
    "**/*.asar",
    // Swift/iOS/Xcode
    "**/DerivedData/**",
    "**/.build/**",
    "**/.swiftpm/**",
    "**/xcuserdata/**",
    "**/Carthage/Build/**",
    "**/SourcePackages/**",
    // PHP
    "**/.composer/**",
    "**/storage/framework/**",
    "**/bootstrap/cache/**",
    // Ruby
    "**/.bundle/**",
    "**/tmp/cache/**",
    "**/public/assets/**",
    "**/public/packs/**",
    "**/.yardoc/**",
    // Testing/Coverage
    "**/coverage/**",
    "**/htmlcov/**",
    "**/.nyc_output/**",
    "**/test-results/**",
    "**/.coverage/**",
    // IDE/Editor
    "**/.idea/**",
    // Logs and temp
    "**/logs/**",
    "**/tmp/**",
    "**/temp/**",
    // Documentation build output
    "**/_build/**",
    "**/docs/_build/**",
    "**/site/**",
];

/// Built-in symbols to skip during reference resolution
/// These are language built-ins that don't exist in the codebase
pub const BUILTIN_SYMBOLS: &[&str] = &[
    // JavaScript/TypeScript globals
    "console",
    "window",
    "document",
    "global",
    "globalThis",
    "process",
    "Promise",
    "Array",
    "Object",
    "String",
    "Number",
    "Boolean",
    "Date",
    "Math",
    "JSON",
    "RegExp",
    "Error",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "Symbol",
    "Proxy",
    "Reflect",
    "fetch",
    "require",
    "module",
    "exports",
    "__dirname",
    "__filename",
    "Buffer",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    "setImmediate",
    "clearImmediate",
    "queueMicrotask",
    // React built-ins
    "React",
    "Component",
    "Fragment",
    "Suspense",
    "StrictMode",
    "createElement",
    "useState",
    "useEffect",
    "useContext",
    "useReducer",
    "useCallback",
    "useMemo",
    "useRef",
    "useImperativeHandle",
    "useLayoutEffect",
    "useDebugValue",
    // Python built-ins
    "print",
    "len",
    "range",
    "str",
    "int",
    "float",
    "list",
    "dict",
    "set",
    "tuple",
    "open",
    "input",
    "type",
    "isinstance",
    "hasattr",
    "getattr",
    "setattr",
    "super",
    "self",
    "cls",
    "None",
    "True",
    "False",
    "__init__",
    "__str__",
    "__repr__",
    // Node.js core modules
    "fs",
    "path",
    "os",
    "crypto",
    "http",
    "https",
    "url",
    "util",
    "events",
    "stream",
    "child_process",
    "buffer",
    "net",
    "dns",
    "tls",
    "zlib",
    "readline",
    "cluster",
    "worker_threads",
    "assert",
    "async_hooks",
    "perf_hooks",
    "v8",
    "vm",
    "wasi",
];

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_kind_roundtrip() {
        for kind in NodeKind::all() {
            let s = kind.as_str();
            let parsed: NodeKind = s.parse().unwrap();
            assert_eq!(*kind, parsed);
        }
    }

    #[test]
    fn test_edge_kind_roundtrip() {
        for kind in EdgeKind::all() {
            let s = kind.as_str();
            let parsed: EdgeKind = s.parse().unwrap();
            assert_eq!(*kind, parsed);
        }
    }

    #[test]
    fn test_language_from_extension() {
        assert_eq!(Language::from_extension("ts"), Language::TypeScript);
        assert_eq!(Language::from_extension("tsx"), Language::Tsx);
        assert_eq!(Language::from_extension("rs"), Language::Rust);
        assert_eq!(Language::from_extension("dart"), Language::Dart);
        assert_eq!(Language::from_extension("xyz"), Language::Unknown);
    }

    #[test]
    fn test_node_creation() {
        let node = Node::new(
            "abc123",
            NodeKind::Function,
            "myFunc",
            "src/lib.rs::myFunc",
            "src/lib.rs",
            Language::Rust,
            10,
            20,
        );

        assert_eq!(node.id.as_str(), "abc123");
        assert_eq!(node.kind, NodeKind::Function);
        assert_eq!(node.name, "myFunc");
        assert_eq!(node.line_count(), 11);
    }

    #[test]
    fn test_edge_creation() {
        let edge = Edge::new("node1", "node2", EdgeKind::Calls);
        assert_eq!(edge.source.as_str(), "node1");
        assert_eq!(edge.target.as_str(), "node2");
        assert_eq!(edge.kind, EdgeKind::Calls);
        assert!(edge.id.is_none());
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.version, 1);
        assert!(!config.include.is_empty());
        assert!(!config.exclude.is_empty());
        assert_eq!(config.max_file_size, 1024 * 1024);
    }

    #[test]
    fn test_subgraph() {
        let mut subgraph = Subgraph::new();
        assert_eq!(subgraph.node_count(), 0);

        let node = Node::new(
            "test",
            NodeKind::Function,
            "test",
            "test",
            "test.rs",
            Language::Rust,
            1,
            10,
        );
        subgraph.add_node(node);
        assert_eq!(subgraph.node_count(), 1);
    }

    #[test]
    fn test_builtin_symbols_not_empty() {
        assert!(!BUILTIN_SYMBOLS.is_empty());
        assert!(BUILTIN_SYMBOLS.contains(&"console"));
        assert!(BUILTIN_SYMBOLS.contains(&"React"));
        assert!(BUILTIN_SYMBOLS.contains(&"print"));
    }

    #[test]
    fn test_default_exclude_patterns() {
        assert!(!DEFAULT_EXCLUDE.is_empty());
        assert!(DEFAULT_EXCLUDE.contains(&"**/node_modules/**"));
        assert!(DEFAULT_EXCLUDE.contains(&"**/.git/**"));
        assert!(DEFAULT_EXCLUDE.contains(&"**/target/**"));
    }
}
