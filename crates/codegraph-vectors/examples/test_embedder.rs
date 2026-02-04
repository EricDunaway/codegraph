//! Test the embedder with the ONNX model
//!
//! Run with: cargo run --example test_embedder -p codegraph-vectors --features onnx

use codegraph_vectors::embedder::{EmbedderConfig, TextEmbedder};

fn main() {
    // Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    println!("=== CodeGraph Embedder Test ===\n");

    // Create embedder with default config (will search standard paths)
    println!("Creating embedder...");
    let config = EmbedderConfig::default();
    println!("  Model search paths:");
    for path in TextEmbedder::model_search_paths() {
        println!("    - {}", path.display());
    }

    let mut embedder = TextEmbedder::new(config);

    // Load the model
    println!("\nLoading model...");
    match embedder.load() {
        Ok(_) => println!("  Model loaded successfully!"),
        Err(e) => {
            eprintln!("  Failed to load model: {}", e);
            std::process::exit(1);
        }
    }

    // Test some embeddings
    let test_texts = [
        "fn get_user_by_id(id: u64) -> Option<User>",
        "async function fetchUserData(userId: string): Promise<User>",
        "def calculate_total_price(items: List[Item]) -> float:",
        "class DatabaseConnection { constructor(config: Config) {} }",
    ];

    println!("\nGenerating embeddings...\n");

    for text in &test_texts {
        match embedder.embed(text) {
            Ok(embedding) => {
                let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
                println!("Text: \"{}\"", &text[..text.len().min(50)]);
                println!("  Dimension: {}", embedding.len());
                println!("  Norm: {:.4}", norm);
                println!("  First 3: [{:.4}, {:.4}, {:.4}]", embedding[0], embedding[1], embedding[2]);
                println!();
            }
            Err(e) => {
                eprintln!("Failed to embed '{}': {}", text, e);
            }
        }
    }

    // Test similarity between similar functions
    println!("=== Similarity Test ===\n");
    let rust_fn = embedder.embed("fn get_user(id: i64) -> User").unwrap();
    let ts_fn = embedder.embed("function getUser(id: number): User").unwrap();
    let unrelated = embedder.embed("class HttpServer { listen(port: number) {} }").unwrap();

    let sim_rust_ts: f32 = rust_fn.iter().zip(ts_fn.iter()).map(|(a, b)| a * b).sum();
    let sim_rust_unrelated: f32 = rust_fn.iter().zip(unrelated.iter()).map(|(a, b)| a * b).sum();

    println!("Rust fn vs TypeScript fn (similar): {:.4}", sim_rust_ts);
    println!("Rust fn vs HttpServer (unrelated): {:.4}", sim_rust_unrelated);
    println!();

    if sim_rust_ts > sim_rust_unrelated {
        println!("✓ Similar functions have higher similarity (as expected)");
    } else {
        println!("✗ Unexpected: unrelated code has higher similarity");
    }

    println!("\n=== Test Complete ===");
}
