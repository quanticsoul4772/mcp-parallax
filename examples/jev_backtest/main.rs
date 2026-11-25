//! 064 T031 — the Jev back-test harness (manual-run).
//!
//! Offline subcommands read the database only. Live subcommands call Jev
//! (`TYPESAFE_API_KEY`) or the configured big model, and spend.
//!
//! ```text
//! cargo run --release --example jev_backtest -- base-rate
//! cargo run --release --example jev_backtest -- report
//! cargo run --release --example jev_backtest -- noise [rows]            (live, Jev)
//! cargo run --release --example jev_backtest -- order [rows]            (live, Jev)
//! cargo run --release --example jev_backtest -- self-agreement [rows]   (live, big model)
//! cargo run --release --example jev_backtest -- injection <folder> <question-file> [threshold]
//! cargo run --release --example jev_backtest -- reproduce [file]
//! ```
//!
//! The database is `DATABASE_PATH` (default `./data/parallax.db`). Noise,
//! order and self-agreement read tune-split rows only, rebuild stored turns
//! from their transcripts, and save what they measured to
//! `./data/jev-measurements.json`; `reproduce` prints the same tables from
//! that file without calls (SC-006). The injection folder holds `.jsonl`
//! lines `{"text": ..., "label": true|false, "url": ...}` that you built
//! yourself in a fresh folder; the harness never downloads anything. Each
//! page is sent as `{"page_url", "page_text"}`, as exa-mcp-server's
//! back-test sent it, and lines that are not JSON or lack a field are
//! counted as skipped. The question file is JSON,
//! `{"instructions": ..., "criteria": {"true": ..., "false": ...}}`.

// Manual tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod live;
mod offline;
mod rebuild;

use mcp_parallax::config::Config;
use mcp_parallax::storage::SqliteStorage;

fn usage() {
    println!(
        "usage: jev_backtest <base-rate | report | noise [rows] | order [rows] | \
         self-agreement [rows] | injection <folder> <question-file> [threshold] | \
         reproduce [file]>"
    );
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        usage();
        std::process::exit(2);
    };
    let config = Config::from_env().expect("configuration");
    let rows = |i: usize| -> usize { args.get(i).map_or(100, |v| v.parse().expect("a row count")) };
    match command.as_str() {
        "base-rate" | "report" | "noise" | "order" | "self-agreement" => {
            let storage = SqliteStorage::connect(&config.database_path)
                .await
                .expect("database");
            match command.as_str() {
                "base-rate" => offline::base_rate(&storage).await,
                "report" => offline::report(&storage).await,
                "noise" => live::noise(&config, &storage, rows(1)).await,
                "order" => live::order(&config, &storage, rows(1)).await,
                _ => live::self_agreement(&config, &storage, rows(1)).await,
            }
        }
        "reproduce" => {
            let path = args.get(1).map_or(rebuild::RESULTS_FILE, String::as_str);
            match mcp_parallax::jev::measure::read_results(std::path::Path::new(path), true) {
                Ok(results) => print!("{}", mcp_parallax::jev::measure::tables(&results)),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        "injection" => {
            let folder = args.get(1).expect("the dataset folder");
            let question = args.get(2).expect("the question file");
            let threshold = args
                .get(3)
                .map_or(0.35, |t| t.parse().expect("a threshold"));
            live::injection(&config, folder, question, threshold).await;
        }
        _ => {
            usage();
            std::process::exit(2);
        }
    }
}
