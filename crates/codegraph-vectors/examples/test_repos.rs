//! Test embeddings on real code from test repositories
//!
//! Run with: cargo run --example test_repos -p codegraph-vectors --features onnx

use codegraph_vectors::embedder::{EmbedderConfig, TextEmbedder};
use std::fs;
use std::path::Path;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    println!("=== CodeGraph Repo Embedding Test ===\n");

    // Initialize embedder
    let config = EmbedderConfig::default();
    let mut embedder = TextEmbedder::new(config);

    println!("Loading model...");
    if let Err(e) = embedder.load() {
        eprintln!("Failed to load model: {}", e);
        std::process::exit(1);
    }
    println!("Model loaded!\n");

    // Test files from different repos
    let test_files = vec![
        // TypeScript backend
        (
            "/Users/edunaway/Estimations/Apps/core-platform-backend/projects/flex-connects/src/flex-connects/payment-gateways/adyen-payment-gateway.flex-connect.ts",
            "TypeScript Payment Gateway",
        ),
        // Rust
        (
            "/Users/edunaway/Repos/atlantis-canary/battery-updater/src/mysql.rs",
            "Rust MySQL Module",
        ),
        (
            "/Users/edunaway/Repos/atlantis-canary/battery-updater/src/config.rs",
            "Rust Config Module",
        ),
    ];

    let mut embeddings: Vec<(&str, Vec<f32>)> = Vec::new();

    println!("=== Generating File Embeddings ===\n");

    for (path, label) in &test_files {
        if !Path::new(path).exists() {
            println!("Skipping {} (file not found)", label);
            continue;
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                println!("Failed to read {}: {}", label, e);
                continue;
            }
        };

        // Truncate content for embedding (models have token limits)
        let truncated = if content.len() > 2000 {
            &content[..2000]
        } else {
            &content
        };

        match embedder.embed(truncated) {
            Ok(emb) => {
                let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
                println!("{}", label);
                println!("  File: {}", path.split('/').next_back().unwrap_or(path));
                println!("  Content length: {} chars", content.len());
                println!("  Embedding norm: {:.4}", norm);
                embeddings.push((*label, emb));
            }
            Err(e) => {
                println!("Failed to embed {}: {}", label, e);
            }
        }
        println!();
    }

    // Test function-level embeddings
    println!("=== Function-Level Similarity Test ===\n");

    let functions = vec![
        ("async function processPayment(amount: number, currency: string): Promise<PaymentResult>", "TS payment fn"),
        ("pub async fn process_payment(amount: f64, currency: &str) -> Result<PaymentResult, Error>", "Rust payment fn"),
        ("fn calculate_battery_percentage(voltage: f32, temp: f32) -> f32", "Rust battery fn"),
        ("function formatCurrency(amount: number, locale: string): string", "TS format fn"),
    ];

    let mut fn_embeddings: Vec<(&str, Vec<f32>)> = Vec::new();

    for (code, label) in &functions {
        match embedder.embed(code) {
            Ok(emb) => {
                fn_embeddings.push((*label, emb));
                println!("Embedded: {}", label);
            }
            Err(e) => println!("Failed: {} - {}", label, e),
        }
    }

    println!("\n=== Cross-Language Similarity Matrix ===\n");

    // Print header
    print!("{:20}", "");
    for (label, _) in &fn_embeddings {
        print!("{:18}", label);
    }
    println!();

    // Print similarity matrix
    for (label1, emb1) in &fn_embeddings {
        print!("{:20}", label1);
        for (_, emb2) in &fn_embeddings {
            let sim: f32 = emb1.iter().zip(emb2.iter()).map(|(a, b)| a * b).sum();
            // Normalize by norms for cosine similarity
            let norm1: f32 = emb1.iter().map(|x| x * x).sum::<f32>().sqrt();
            let norm2: f32 = emb2.iter().map(|x| x * x).sum::<f32>().sqrt();
            let cosine = sim / (norm1 * norm2);
            print!("{:18.4}", cosine);
        }
        println!();
    }

    println!("\n=== Analysis ===\n");

    // Check if similar functions (payment) have high similarity
    if fn_embeddings.len() >= 2 {
        let ts_payment = &fn_embeddings[0].1;
        let rust_payment = &fn_embeddings[1].1;
        let rust_battery = &fn_embeddings[2].1;

        let sim_payment: f32 = ts_payment.iter().zip(rust_payment.iter()).map(|(a, b)| a * b).sum();
        let sim_unrelated: f32 = ts_payment.iter().zip(rust_battery.iter()).map(|(a, b)| a * b).sum();

        let norm_ts: f32 = ts_payment.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_rust_pay: f32 = rust_payment.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_rust_bat: f32 = rust_battery.iter().map(|x| x * x).sum::<f32>().sqrt();

        let cosine_similar = sim_payment / (norm_ts * norm_rust_pay);
        let cosine_unrelated = sim_unrelated / (norm_ts * norm_rust_bat);

        println!("TS payment vs Rust payment (similar concept): {:.4}", cosine_similar);
        println!("TS payment vs Rust battery (unrelated): {:.4}", cosine_unrelated);

        if cosine_similar > cosine_unrelated {
            println!("\n✓ Cross-language semantic similarity working correctly!");
        } else {
            println!("\n✗ Unexpected: unrelated code has higher similarity");
        }
    }

    println!("\n=== Test Complete ===");
}
