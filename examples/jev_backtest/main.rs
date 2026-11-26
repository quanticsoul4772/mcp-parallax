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
//! cargo run --release --example jev_backtest -- injection <folder> [threshold]
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
//! page is sent in the shape the earlier back-test used,
//! `{"page_url", "page_text"}`, but redacted first, the URL included (any
//! run of 20 or more characters mixing letters and digits, which can cross
//! `/` and `-`, is replaced, so a URL such as
//! `https://github.com/anthropics/claude-code/issues/12345` arrives as
//! `https://github.[REDACTED]`). Lines that are not JSON or lack a field are
//! counted as skipped, and a dataset with no usable line stops the run before
//! any call, leaving the saved result as it was; so does a run in which no
//! call produced an answer. The question is the server's own `directs_ai`
//! (`jev::questions::injection_question`).

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
         self-agreement [rows] | injection <folder> [threshold] | \
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
            let threshold = args
                .get(2)
                .map_or(0.35, |t| t.parse().expect("a threshold"));
            live::injection(&config, folder, threshold).await;
        }
        _ => {
            usage();
            std::process::exit(2);
        }
    }
}
