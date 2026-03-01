//! Embedding text builder for constructing text representations of nodes
//!
//! This module creates text for embedding generation with:
//! - Decorator-first ordering (E1)
//! - Token budget enforcement (B1-B8)
//! - Graph context inclusion (G1-G6)
//! - Tiered truncation (B3, B6)

use crate::error::VectorError;
use codegraph_types::{EmbeddingTextConfig, Node, NodeKind};
use tiktoken_rs::cl100k_base;

/// Graph context for a node (callees, callers, siblings, inheritance)
#[derive(Debug, Clone, Default)]
pub struct GraphContext {
    /// Functions/methods this node calls
    pub callees: Vec<String>,
    /// Functions/methods that call this node
    pub callers: Vec<String>,
    /// Sibling nodes in the same container (class methods, module functions)
    pub siblings: Vec<String>,
    /// Interfaces/traits this node implements
    pub implements: Vec<String>,
    /// Parent class this node extends
    pub extends: Option<String>,
}

/// LSP and extraction enrichment data for a node
#[derive(Debug, Clone, Default)]
pub struct NodeEnrichment {
    /// Inferred type from LSP hover
    pub inferred_type: Option<String>,
    /// Resolved import path
    pub resolved_import_path: Option<String>,
    /// Package name
    pub package_name: Option<String>,
    /// Thrown error types
    pub thrown_errors: Vec<String>,
    /// Associated test names
    pub test_names: Vec<String>,
    /// Code snippet
    pub code_snippet: Option<String>,
}

impl NodeEnrichment {
    /// Create enrichment data from a Node's enrichment fields
    pub fn from_node(node: &Node) -> Self {
        Self {
            inferred_type: node.inferred_type.clone(),
            resolved_import_path: node.resolved_import_path.clone(),
            package_name: node.package_name.clone(),
            thrown_errors: node.thrown_errors.clone(),
            test_names: node.test_names.clone(),
            code_snippet: node.code_snippet.clone(),
        }
    }
}

/// Token counter using tiktoken cl100k_base as proxy for embedding model (B5)
pub struct TokenCounter {
    bpe: tiktoken_rs::CoreBPE,
}

impl TokenCounter {
    /// Create a new token counter
    pub fn new() -> Result<Self, VectorError> {
        let bpe = cl100k_base()
            .map_err(|e| VectorError::TokenCounterFailed(e.to_string()))?;
        Ok(Self { bpe })
    }

    /// Count tokens in text
    pub fn count_tokens(&self, text: &str) -> usize {
        self.bpe.encode_with_special_tokens(text).len()
    }
}

/// Builder for constructing embedding text from nodes
pub struct EmbeddingTextBuilder {
    config: EmbeddingTextConfig,
}

impl EmbeddingTextBuilder {
    /// Create a new builder with the given configuration
    pub fn new(config: EmbeddingTextConfig) -> Self {
        Self { config }
    }

    /// Build embedding text for a node (without token budget enforcement)
    pub fn build_text(
        &self,
        node: &Node,
        context: &GraphContext,
        enrichment: &NodeEnrichment,
    ) -> String {
        let mut parts = Vec::new();

        // E1: Decorators first
        if !node.decorators.is_empty() {
            parts.push(node.decorators.join(" "));
        }

        // Kind + name
        parts.push(format!("{} {}", node.kind.as_str(), node.name));

        // File path
        parts.push(format!("in {}", node.file_path));

        // Signature (if present)
        if let Some(ref sig) = node.signature {
            parts.push(sig.clone());
        }

        // Docstring (if present)
        if let Some(ref doc) = node.docstring {
            parts.push(doc.clone());
        }

        // Inferred type from LSP
        if let Some(ref typ) = enrichment.inferred_type {
            parts.push(format!("type: {}", typ));
        }

        // Resolved import path (for import nodes)
        if matches!(node.kind, NodeKind::Import) {
            if let Some(ref path) = enrichment.resolved_import_path {
                parts.push(format!("resolves to: {}", path));
            }
        }

        // Package name (from enrichment or derived from directory)
        if let Some(ref pkg) = enrichment.package_name {
            parts.push(format!("package: {}", pkg));
        } else if let Some(pkg) = Self::derive_package_from_path(&node.file_path) {
            parts.push(format!("package: {}", pkg));
        }

        // Inheritance (for classes)
        if matches!(node.kind, NodeKind::Class | NodeKind::Struct) {
            if let Some(ref extends) = context.extends {
                parts.push(format!("extends: {}", extends));
            }
            if !context.implements.is_empty() {
                parts.push(format!("implements: {}", context.implements.join(", ")));
            }
        }

        // Graph context
        if !context.callees.is_empty() {
            let callees: Vec<_> = context.callees.iter().take(self.config.max_callees).cloned().collect();
            parts.push(format!("calls: {}", callees.join(", ")));
        }

        if !context.callers.is_empty() {
            let callers: Vec<_> = context.callers.iter().take(self.config.max_callers).cloned().collect();
            parts.push(format!("called by: {}", callers.join(", ")));
        }

        if !context.siblings.is_empty() {
            let siblings: Vec<_> = context.siblings.iter().take(self.config.max_siblings).cloned().collect();
            parts.push(format!("siblings: {}", siblings.join(", ")));
        }

        // Thrown errors
        if !enrichment.thrown_errors.is_empty() {
            parts.push(format!("throws: {}", enrichment.thrown_errors.join(", ")));
        }

        // Associated tests
        if !enrichment.test_names.is_empty() {
            parts.push(format!("tested by: {}", enrichment.test_names.join(", ")));
        }

        // Code snippet (last, most likely to be truncated)
        if let Some(ref snippet) = enrichment.code_snippet {
            let lines: Vec<_> = snippet.lines().take(self.config.max_snippet_lines).collect();
            if !lines.is_empty() {
                parts.push(format!("code:\n{}", lines.join("\n")));
            }
        }

        parts.join("\n")
    }

    /// Build embedding text with token budget enforcement
    pub fn build_text_with_budget(
        &self,
        node: &Node,
        context: &GraphContext,
        enrichment: &NodeEnrichment,
        counter: &TokenCounter,
    ) -> String {
        let mut text = self.build_text(node, context, enrichment);
        let mut tokens = counter.count_tokens(&text);

        if tokens <= self.config.max_tokens {
            return text;
        }

        // Tier 1 overflow protection (B6): truncate decorators, signature, code
        let mut modified_node = node.clone();
        let mut modified_enrichment = enrichment.clone();

        // Truncate decorators to max 10
        if modified_node.decorators.len() > 10 {
            modified_node.decorators.truncate(10);
        }

        // Truncate signature to ~200 chars
        if let Some(ref sig) = modified_node.signature {
            if sig.len() > 200 {
                modified_node.signature = Some(format!("{}...", &sig[..197]));
            }
        }

        // Remove code snippet first (most expendable)
        modified_enrichment.code_snippet = None;

        text = self.build_text(&modified_node, context, &modified_enrichment);
        tokens = counter.count_tokens(&text);

        if tokens <= self.config.max_tokens {
            return text;
        }

        // Tier 2: Remove graph context (callees, callers, siblings)
        let empty_context = GraphContext::default();
        text = self.build_text(&modified_node, &empty_context, &modified_enrichment);
        tokens = counter.count_tokens(&text);

        if tokens <= self.config.max_tokens {
            return text;
        }

        // Tier 3: Remove docstring and enrichment
        modified_node.docstring = None;
        modified_enrichment = NodeEnrichment::default();
        text = self.build_text(&modified_node, &empty_context, &modified_enrichment);
        tokens = counter.count_tokens(&text);

        if tokens <= self.config.max_tokens {
            return text;
        }

        // Final fallback: just kind + name + file
        format!(
            "{} {}\nin {}",
            modified_node.kind.as_str(),
            modified_node.name,
            modified_node.file_path
        )
    }

    /// Derive a package name from the file path (M2: directory fallback)
    ///
    /// Takes the parent directory of the file as the package name.
    /// E.g., "src/services/payment/handler.ts" -> "src/services/payment"
    fn derive_package_from_path(file_path: &str) -> Option<String> {
        let path = std::path::Path::new(file_path);
        path.parent().map(|p| p.to_string_lossy().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_types::Language;

    fn make_test_node() -> Node {
        let mut node = Node::new(
            "test-id",
            NodeKind::Function,
            "processPayment",
            "PaymentService.processPayment",
            "src/services/payment.ts",
            Language::TypeScript,
            10,
            25,
        );
        node.decorators = vec!["@Controller".to_string(), "@Post('/pay')".to_string()];
        node.signature = Some("async processPayment(order: Order): Promise<Receipt>".to_string());
        node.docstring = Some("Process a payment for an order.".to_string());
        node
    }

    #[test]
    fn test_token_counting() {
        let counter = TokenCounter::new().expect("Failed to create token counter");

        // Short text
        let count = counter.count_tokens("Hello world");
        assert!(count > 0 && count < 10, "Expected 2-3 tokens, got {}", count);

        // Code-like text
        let code = "function processPayment(order: Order): Promise<Receipt> { return this.gateway.charge(order.total); }";
        let count = counter.count_tokens(code);
        assert!(
            count > 15 && count < 40,
            "Expected ~25 tokens, got {}",
            count
        );
    }

    #[test]
    fn test_basic_embedding_text() {
        let node = make_test_node();
        let config = EmbeddingTextConfig::default();
        let builder = EmbeddingTextBuilder::new(config);

        let text = builder.build_text(&node, &GraphContext::default(), &NodeEnrichment::default());

        // Decorators first (E1)
        assert!(
            text.starts_with("@Controller"),
            "Should start with decorators: {}",
            text
        );
        // Contains kind and name
        assert!(text.contains("function processPayment"));
        // Contains file path
        assert!(text.contains("src/services/payment.ts"));
        // Contains signature
        assert!(text.contains("async processPayment(order: Order)"));
    }

    #[test]
    fn test_embedding_without_decorators() {
        let mut node = make_test_node();
        node.decorators = vec![];

        let config = EmbeddingTextConfig::default();
        let builder = EmbeddingTextBuilder::new(config);

        let text = builder.build_text(&node, &GraphContext::default(), &NodeEnrichment::default());

        // Should start with kind + name
        assert!(text.starts_with("function processPayment"));
    }

    #[test]
    fn test_graph_context_in_embedding() {
        let config = EmbeddingTextConfig::default();
        let builder = EmbeddingTextBuilder::new(config);

        let node = make_test_node();
        let context = GraphContext {
            callees: vec!["validateOrder".to_string(), "chargeCard".to_string()],
            callers: vec!["handleCheckout".to_string()],
            siblings: vec![
                "refundPayment".to_string(),
                "getPaymentStatus".to_string(),
            ],
            implements: vec![],
            extends: None,
        };

        let text = builder.build_text(&node, &context, &NodeEnrichment::default());

        assert!(text.contains("calls: validateOrder, chargeCard"));
        assert!(text.contains("called by: handleCheckout"));
        assert!(text.contains("siblings: refundPayment, getPaymentStatus"));
    }

    #[test]
    fn test_graph_context_respects_limits() {
        let config = EmbeddingTextConfig {
            max_callees: 2,
            max_callers: 1,
            max_siblings: 2,
            ..Default::default()
        };
        let builder = EmbeddingTextBuilder::new(config);

        let node = make_test_node();
        let context = GraphContext {
            callees: vec![
                "callee_a".to_string(),
                "callee_b".to_string(),
                "callee_c".to_string(),
                "callee_d".to_string(),
            ],
            callers: vec!["caller_x".to_string(), "caller_y".to_string(), "caller_z".to_string()],
            siblings: vec!["sibling_1".to_string(), "sibling_2".to_string(), "sibling_3".to_string()],
            ..Default::default()
        };

        let text = builder.build_text(&node, &context, &NodeEnrichment::default());

        // Should only have first 2 callees
        assert!(text.contains("calls: callee_a, callee_b"));
        // c and d should not be in the calls line
        let calls_line = text.lines().find(|l| l.starts_with("calls:")).unwrap();
        assert!(!calls_line.contains("callee_c"));
        assert!(!calls_line.contains("callee_d"));

        // Should only have first 1 caller
        let callers_line = text.lines().find(|l| l.starts_with("called by:")).unwrap();
        assert!(callers_line.contains("caller_x"));
        assert!(!callers_line.contains("caller_y"));
    }

    #[test]
    fn test_class_inheritance_in_embedding() {
        let config = EmbeddingTextConfig::default();
        let builder = EmbeddingTextBuilder::new(config);

        let mut node = make_test_node();
        node.kind = NodeKind::Class;
        node.name = "PaymentService".to_string();

        let context = GraphContext {
            extends: Some("BaseService".to_string()),
            implements: vec!["IPayment".to_string(), "IRefundable".to_string()],
            ..Default::default()
        };

        let text = builder.build_text(&node, &context, &NodeEnrichment::default());

        assert!(text.contains("extends: BaseService"));
        assert!(text.contains("implements: IPayment, IRefundable"));
    }

    #[test]
    fn test_truncation_respects_budget() {
        let config = EmbeddingTextConfig {
            max_tokens: 50, // Very small budget
            ..Default::default()
        };
        let builder = EmbeddingTextBuilder::new(config);
        let counter = TokenCounter::new().unwrap();

        let mut node = make_test_node();
        node.docstring = Some("A".repeat(1000)); // Very long docstring

        let text = builder.build_text_with_budget(
            &node,
            &GraphContext::default(),
            &NodeEnrichment::default(),
            &counter,
        );

        let tokens = counter.count_tokens(&text);
        assert!(tokens <= 50, "Should respect budget: {} tokens", tokens);
    }

    #[test]
    fn test_tier1_overflow_truncates_decorators() {
        let config = EmbeddingTextConfig {
            max_tokens: 30, // Very small
            ..Default::default()
        };
        let builder = EmbeddingTextBuilder::new(config);
        let counter = TokenCounter::new().unwrap();

        let mut node = make_test_node();
        // 50 decorators should trigger overflow protection (B6)
        node.decorators = (0..50).map(|i| format!("@Decorator{}", i)).collect();

        let text = builder.build_text_with_budget(
            &node,
            &GraphContext::default(),
            &NodeEnrichment::default(),
            &counter,
        );

        // Should have max 10 decorators after truncation
        let decorator_count = text.matches("@Decorator").count();
        assert!(
            decorator_count <= 10,
            "Should limit decorators to 10, found {}",
            decorator_count
        );
    }

    #[test]
    fn test_tier1_overflow_truncates_signature() {
        let config = EmbeddingTextConfig {
            max_tokens: 30,
            ..Default::default()
        };
        let builder = EmbeddingTextBuilder::new(config);
        let counter = TokenCounter::new().unwrap();

        let mut node = make_test_node();
        node.decorators = vec![];
        // Very long signature (500+ chars)
        node.signature = Some(format!(
            "function veryLongName({}): void",
            "param: Type, ".repeat(50)
        ));

        let text = builder.build_text_with_budget(
            &node,
            &GraphContext::default(),
            &NodeEnrichment::default(),
            &counter,
        );

        // Signature should be truncated to ~200 chars or removed entirely
        // depending on tier reached
        let tokens = counter.count_tokens(&text);
        assert!(tokens <= 30, "Should respect budget: {} tokens", tokens);
    }

    #[test]
    fn test_enrichment_in_embedding() {
        let config = EmbeddingTextConfig::default();
        let builder = EmbeddingTextBuilder::new(config);

        let node = make_test_node();
        let enrichment = NodeEnrichment {
            inferred_type: Some("Promise<Receipt>".to_string()),
            package_name: Some("@myapp/payments".to_string()),
            thrown_errors: vec!["PaymentError".to_string()],
            test_names: vec!["test_processPayment".to_string()],
            ..Default::default()
        };

        let text = builder.build_text(&node, &GraphContext::default(), &enrichment);

        assert!(text.contains("type: Promise<Receipt>"));
        assert!(text.contains("package: @myapp/payments"));
        assert!(text.contains("throws: PaymentError"));
        assert!(text.contains("tested by: test_processPayment"));
    }

    #[test]
    fn test_code_snippet_in_embedding() {
        let config = EmbeddingTextConfig {
            max_snippet_lines: 3,
            ..Default::default()
        };
        let builder = EmbeddingTextBuilder::new(config);

        let node = make_test_node();
        let enrichment = NodeEnrichment {
            code_snippet: Some(
                "async function processPayment(order: Order) {\n  const result = await charge(order);\n  return result;\n  // more code\n  // even more\n}".to_string(),
            ),
            ..Default::default()
        };

        let text = builder.build_text(&node, &GraphContext::default(), &enrichment);

        assert!(text.contains("code:"));
        // Should only have 3 lines
        let code_section = text.split("code:\n").nth(1).unwrap_or("");
        let line_count = code_section.lines().count();
        assert!(line_count <= 3, "Should limit snippet to 3 lines, got {}", line_count);
    }

    #[test]
    fn test_import_resolved_path() {
        let config = EmbeddingTextConfig::default();
        let builder = EmbeddingTextBuilder::new(config);

        let mut node = make_test_node();
        node.kind = NodeKind::Import;
        node.name = "PaymentService".to_string();
        node.decorators = vec![];

        let enrichment = NodeEnrichment {
            resolved_import_path: Some("src/services/payment.ts".to_string()),
            ..Default::default()
        };

        let text = builder.build_text(&node, &GraphContext::default(), &enrichment);

        assert!(text.contains("resolves to: src/services/payment.ts"));
    }

    #[test]
    fn test_package_name_fallback_to_directory() {
        let config = EmbeddingTextConfig::default();
        let builder = EmbeddingTextBuilder::new(config);

        let mut node = make_test_node();
        node.file_path = "src/services/payment/handler.ts".to_string();
        node.decorators = vec![];

        // No package_name in enrichment - should derive from path
        let enrichment = NodeEnrichment::default();

        let text = builder.build_text(&node, &GraphContext::default(), &enrichment);

        assert!(text.contains("package: src/services/payment"));
    }

    #[test]
    fn test_package_name_from_enrichment_takes_precedence() {
        let config = EmbeddingTextConfig::default();
        let builder = EmbeddingTextBuilder::new(config);

        let mut node = make_test_node();
        node.file_path = "src/services/payment/handler.ts".to_string();
        node.decorators = vec![];

        let enrichment = NodeEnrichment {
            package_name: Some("@myapp/payments".to_string()),
            ..Default::default()
        };

        let text = builder.build_text(&node, &GraphContext::default(), &enrichment);

        // Should use enrichment value, not derived
        // Count package lines - should only have one with the enrichment value
        let package_lines: Vec<_> = text.lines().filter(|l| l.starts_with("package:")).collect();
        assert_eq!(package_lines.len(), 1, "Should have exactly one package line");
        assert!(
            package_lines[0].contains("@myapp/payments"),
            "Package line should use enrichment value: {}",
            package_lines[0]
        );
        // Derived value should NOT appear as a separate package line
        assert!(
            !package_lines[0].contains("src/services/payment"),
            "Should not use derived package: {}",
            package_lines[0]
        );
    }

    #[test]
    fn test_derive_package_from_path() {
        assert_eq!(
            EmbeddingTextBuilder::derive_package_from_path("src/services/payment/handler.ts"),
            Some("src/services/payment".to_string())
        );
        assert_eq!(
            EmbeddingTextBuilder::derive_package_from_path("main.ts"),
            Some("".to_string())
        );
    }
}
