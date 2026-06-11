//! Parallax MCP server — binary entry point.
//!
//! All logs go to stderr; stdout is reserved for MCP JSON-RPC. The transport and
//! tool surface are not yet wired — this entry point initializes logging and
//! validates configuration, the foundation the server is built on.

// The binary entry point is a production path too — no panics via unwrap/expect.
#![deny(clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::config::Config;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "--version" | "-v" => {
                print_version();
                return;
            }
            "--help" | "-h" => {
                print_help();
                return;
            }
            other => {
                eprintln!("Unknown argument: {other}");
                eprintln!();
                print_help();
                std::process::exit(1);
            }
        }
    }

    // Initialize logging to stderr only (stdout is for MCP JSON-RPC).
    let filter = tracing_subscriber::EnvFilter::new(
        std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
    );
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            tracing::error!("configuration error: {e}");
            std::process::exit(1);
        }
    };

    tracing::info!(
        database = %config.database_path,
        timeout_ms = config.request_timeout_ms,
        max_retries = config.max_retries,
        "parallax: configuration loaded; transport not yet wired"
    );
}

fn print_version() {
    println!("mcp-parallax {}", env!("CARGO_PKG_VERSION"));
}

fn print_help() {
    println!("Parallax MCP server v{}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("USAGE:");
    println!("    mcp-parallax [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    --version, -v    Print version information and exit");
    println!("    --help, -h       Print this help message and exit");
    println!();
    println!("    (no arguments)   Initialize and start the server");
    println!();
    println!("ENVIRONMENT VARIABLES:");
    println!("    ANTHROPIC_API_KEY     Anthropic API key (required)");
    println!("    DATABASE_PATH         SQLite database path (default: ./data/parallax.db)");
    println!("    LOG_LEVEL             error|warn|info|debug|trace (default: info)");
    println!("    REQUEST_TIMEOUT_MS    Per-request timeout in ms (default: 30000)");
    println!("    MAX_RETRIES           Maximum API retry attempts (default: 3)");
}
