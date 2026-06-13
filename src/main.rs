//! Parallax MCP server — binary entry point.
//!
//! All logs go to stderr; stdout is reserved for MCP JSON-RPC. Construction
//! order is config → storage (migration at boot) → client → server →
//! serve(stdio): every misconfiguration fails here, named, before the first
//! tool call.

// The binary entry point is a production path too — no panics via unwrap/expect.
#![deny(clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::client::AnthropicClient;
use mcp_parallax::config::Config;
use mcp_parallax::server::Parallax;
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::clock::SystemClock;
use rmcp::transport::stdio;
use rmcp::ServiceExt;
use std::sync::Arc;

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
    // OTel's internal diagnostics flow through `tracing` (internal-logs) —
    // default them to warn so a misconfigured collector is visible without
    // drowning the log (007 D8); LOG_LEVEL directives can still override.
    // Defaults come FIRST: EnvFilter replaces duplicate-target directives
    // with the later one, so user LOG_LEVEL directives genuinely override
    // these (review finding 1).
    let filter = tracing_subscriber::EnvFilter::new(format!(
        "opentelemetry=warn,opentelemetry_sdk=warn,opentelemetry-otlp=warn,{}",
        std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string())
    ));
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

    // The default DATABASE_PATH lives under ./data/ — create the parent
    // directory so a fresh checkout boots without manual setup.
    if let Some(parent) = std::path::Path::new(&config.database_path).parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::error!("cannot create database directory {parent:?}: {e}");
                std::process::exit(1);
            }
        }
    }

    let storage = match SqliteStorage::connect(&config.database_path).await {
        Ok(storage) => Arc::new(storage),
        Err(e) => {
            tracing::error!("storage error at startup: {e}");
            std::process::exit(1);
        }
    };

    let client = Arc::new(AnthropicClient::new(&config));
    let server = match Parallax::new(client, storage, Arc::new(SystemClock), &config) {
        Ok(server) => server,
        Err(e) => {
            tracing::error!("server construction failed: {e}");
            std::process::exit(1);
        }
    };

    // Telemetry (007): off unless a standard OTLP endpoint variable is set
    // (and OTEL_SDK_DISABLED is not true); a malformed variable fails boot,
    // named, like every other config error.
    let telemetry = match mcp_parallax::observability::init(server.session_id()) {
        Ok(guard) => guard,
        Err(e) => {
            tracing::error!("telemetry configuration error: {e}");
            std::process::exit(1);
        }
    };

    tracing::info!(
        database = %config.database_path,
        model = %config.anthropic_model,
        ensemble_k = config.verify_ensemble_k,
        timeout_ms = config.request_timeout_ms,
        max_retries = config.max_retries,
        telemetry = telemetry.is_some(),
        "parallax: serving MCP over stdio"
    );

    let service = match server.serve(stdio()).await {
        Ok(service) => service,
        Err(e) => {
            tracing::error!("transport initialization failed: {e}");
            std::process::exit(1);
        }
    };
    let result = service.waiting().await;
    // Flush buffered telemetry within the bounded window before exit
    // (007 FR-010) — a dead collector never hangs shutdown.
    if let Some(guard) = telemetry {
        guard.shutdown();
    }
    if let Err(e) = result {
        tracing::error!("server terminated with error: {e}");
        std::process::exit(1);
    }
}

// --version/--help run before the MCP transport exists, so stdout is still a
// terminal here — the one place printing to it is correct.
#[allow(clippy::print_stdout)]
fn print_version() {
    println!("mcp-parallax {}", env!("CARGO_PKG_VERSION"));
}

#[allow(clippy::print_stdout)]
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
    println!("    (no arguments)   Start the MCP server on stdio");
    println!();
    println!("ENVIRONMENT VARIABLES:");
    println!("    ANTHROPIC_API_KEY       Anthropic API key (required)");
    println!("    ANTHROPIC_MODEL         Model id (default: claude-opus-4-8)");
    println!("    VERIFY_ENSEMBLE_K       Verification passes, >= 1 (default: 3)");
    println!("    VERIFY_MAX_CLAIM_CHARS  Max claim length (default: 50000)");
    println!("    DATABASE_PATH           SQLite database path (default: ./data/parallax.db)");
    println!("    LOG_LEVEL               error|warn|info|debug|trace (default: info)");
    println!("    REQUEST_TIMEOUT_MS      Per-request timeout in ms (default: 30000)");
    println!("    MAX_RETRIES             Maximum API retry attempts (default: 3)");
}
