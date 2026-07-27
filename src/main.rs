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
    println!("{}", help_text());
}

/// The `--help` body, built as a string so a test can check it against the
/// variables [`mcp_parallax::config`] actually reads.
///
/// This is the **only pre-runtime contract** a caller sees: someone who runs
/// `--help` and then starts the server should not end up with two different
/// mental models. It drifted badly before this was testable — the timeout
/// default said 30000 when the code had read 120000 since 018, and thirteen
/// variables plus both routing namespaces were missing entirely. 027 corrected
/// that same timeout in `README.md` and `CLAUDE.md` and did not touch this
/// block, because nothing connected them.
fn help_text() -> String {
    format!(
        "\
Parallax MCP server v{version}

USAGE:
    mcp-parallax [OPTIONS]

OPTIONS:
    --version, -v    Print version information and exit
    --help, -h       Print this help message and exit

    (no arguments)   Start the MCP server on stdio

CORE (always read):
    ANTHROPIC_API_KEY             Anthropic API key. REQUIRED; startup fails without it
    ANTHROPIC_MODEL               Default model id (default: claude-opus-4-8)
    ANTHROPIC_API_BASE            API endpoint (default: https://api.anthropic.com)
    INPUT_MAX_CHARS               Max input length (default: 50000)
                                  VERIFY_MAX_CLAIM_CHARS is honoured as a
                                  deprecated 002-era alias when this is unset
    VERIFY_ENSEMBLE_K             Verification passes, >= 1 (default: 3).
                                  A call may request fewer via `passes`, never more
    DATABASE_PATH                 SQLite path (default: ./data/parallax.db)
    LOG_LEVEL                     error|warn|info|debug|trace (default: info)
    REQUEST_TIMEOUT_MS            Per-request timeout in ms (default: 120000)
    MAX_RETRIES                   Maximum API retry attempts (default: 3)

CAPABILITY GATES (absent = the tools are not in the catalog at all):
    VOYAGE_API_KEY                Enables the memory tools (save/recall/forget/surface)
    VOYAGE_MODEL                  Embedding model (default: voyage-4). Stay within one
                                  family — changing it makes stored vectors incomparable
    MEMORY_RECALL_LIMIT           Default recall top-k, 1..=20 (default: 5).
                                  `recall` takes a per-call `limit`
    BRAVE_API_KEY                 Enables `research`
    GROUNDED_VERIFY_ROOT          Enables `grounded_verify`; the single root that
                                  locators resolve within, canonicalized at startup

RESEARCH:
    FETCH_TIMEOUT_MS              Per-source fetch timeout in ms (default: 10000)
    RESEARCH_CONCURRENCY          Concurrent fetch/extract/verify cap, 1..=32
                                  (default: 8). A call may lower it, never raise it
    FETCH_ALLOW_PRIVATE           SSRF guard (default: false). When false, fetches to
                                  loopback/private/link-local targets are blocked

GROUNDED VERIFY:
    GROUNDED_VERIFY_MAX_BYTES     Assembled-evidence byte ceiling (default: 262144)
    GROUNDED_VERIFY_MAX_LOCATORS  Max locators per call (default: 64)

CHECKPOINTS:
    CHECKPOINT_GATE_PATTERNS      Comma-separated substrings extending the pre-action
                                  gate's built-in risk patterns (default: empty).
                                  An empty entry (\"a,,b\") is an error, not a skip

PER-CALL-SITE ROUTING (both off by default; unset changes nothing):
    PARALLAX_MODEL_<SITE>         Route one call site to a model
    PARALLAX_MODEL_<TIER>         Route a whole tier
    PARALLAX_EFFORT_<SITE>        Reasoning effort for one call site
    PARALLAX_EFFORT_<TIER>        Reasoning effort for a whole tier

    SITES:  VERIFY UNSTICK DIVERGE DECIDE ELICIT GROUNDED_VERIFY CHECK_TRANSLATE
            RESEARCH_SCOPE RESEARCH_EXTRACT RESEARCH_VERIFY RESEARCH_SYNTHESIZE
            CHECKPOINT_REVIEW
    TIERS:  BULK (research extraction only) JUDGMENT (everything else)
    LEVELS: low medium high max xhigh

    Model and effort resolve independently, most-specific-first: site, then tier,
    then the default. An unknown suffix or level is a startup error naming the
    variable. Effort support varies by model family and is NOT checked at startup —
    claude-haiku-4-5 rejects it; when a provider does, the error names the model,
    the level, and the remedies.

TELEMETRY:
    OTEL_EXPORTER_OTLP_ENDPOINT   Presence enables OTLP export (traces + metrics).
                                  The standard OTEL_* family is honoured
    OTEL_SDK_DISABLED             true (case-insensitive) force-disables telemetry

A present-but-unparseable value is an error, never a silent fallback to the default.
",
        version = env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::help_text;

    /// Every environment variable `config.rs` reads must appear in `--help`.
    ///
    /// The drift this catches is not hypothetical: before this test the help
    /// text advertised a 30000 ms timeout against a 120000 ms default, omitted
    /// thirteen variables and both routing namespaces, and still named a
    /// deprecated alias as though it were canonical. 027 fixed that same
    /// timeout in two markdown files and missed this one, and a later
    /// loose-ends sweep re-checked those same two files — because both were
    /// looking for documentation where documentation *looks* like it lives.
    ///
    /// Comparing against the source that does the reading is what makes the
    /// next omission fail rather than ship.
    #[test]
    fn help_lists_every_variable_the_config_reads() {
        let config = include_str!("config.rs");
        let help = help_text();

        // Fixtures that exist only to prove absent/invalid handling, and the
        // alias the help mentions by name but does not advertise as canonical.
        let fixtures = [
            "PARALLAX_TEST_DEFINITELY_UNSET_KEY",
            "PARALLAX_MODEL_NOT_A_CALL_SITE",
        ];

        let mut missing = Vec::new();
        let mut rest = config;
        while let Some(open) = rest.find('"') {
            rest = &rest[open + 1..];
            let Some(close) = rest.find('"') else { break };
            let token = &rest[..close];
            rest = &rest[close + 1..];
            let looks_like_env = token.len() >= 4
                && token
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                && token.contains('_');
            if looks_like_env && !fixtures.contains(&token) && !help.contains(token) {
                missing.push(token.to_string());
            }
        }
        missing.sort_unstable();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "`--help` omits variables that config.rs reads: {missing:?}"
        );
    }

    /// The routing namespaces are prefixes rather than whole names, so the
    /// scan above cannot see them. 018 and 022 both shipped a namespace with
    /// no operator-facing documentation anywhere; 027 added it to the markdown
    /// and not here.
    #[test]
    fn help_documents_both_routing_namespaces_and_their_vocabulary() {
        let help = help_text();
        for prefix in ["PARALLAX_MODEL_", "PARALLAX_EFFORT_"] {
            assert!(help.contains(prefix), "`--help` omits {prefix}*");
        }
        for site in ["RESEARCH_EXTRACT", "CHECKPOINT_REVIEW", "GROUNDED_VERIFY"] {
            assert!(help.contains(site), "`--help` omits the {site} call site");
        }
        for tier in ["BULK", "JUDGMENT"] {
            assert!(help.contains(tier), "`--help` omits the {tier} tier");
        }
        for level in ["low", "medium", "high", "max", "xhigh"] {
            assert!(
                help.contains(level),
                "`--help` omits the `{level}` effort level"
            );
        }
    }

    /// Every numeric default `config.rs` applies must be the one `--help`
    /// states, **read out of the source rather than listed here**.
    ///
    /// 034 pinned five values by hand and called the loop closed. It was not:
    /// changing `MEMORY_RECALL_LIMIT` from 5 to 7 and `RESEARCH_CONCURRENCY`
    /// from 8 to 12 in `config.rs` left all three help tests green, because
    /// neither number was on the hand-written list. That is the same class as
    /// the defect 034 existed to fix — help saying 30000 while the code read
    /// 120000 — so half the class had been left open.
    ///
    /// Deriving the pairs means a default that changes without its help entry
    /// fails, including for variables nobody thought to pin.
    #[test]
    fn help_states_every_numeric_default_the_config_applies() {
        let config = include_str!("config.rs");
        let help = help_text();
        let lines: Vec<&str> = help.lines().collect();
        let marker = "parse_env(\"";

        let mut checked = 0;
        let mut wrong = Vec::new();
        let mut rest = config;
        while let Some(at) = rest.find(marker) {
            rest = &rest[at + marker.len()..];
            let Some(close) = rest.find('"') else { break };
            let name = &rest[..close];
            let after = &rest[close + 1..];
            let Some(comma) = after.find(',') else {
                continue;
            };
            let Some(end) = after[comma + 1..].find(')') else {
                continue;
            };
            let literal: String = after[comma + 1..comma + 1 + end]
                .trim()
                .chars()
                .filter(char::is_ascii_digit)
                .collect();
            if literal.is_empty() {
                continue;
            }

            // An entry is its own line plus its continuation lines. The
            // window reaches backwards too, because a name can appear ON a
            // continuation line while its default sits on the entry above —
            // VERIFY_MAX_CLAIM_CHARS is exactly that, an alias documented
            // under INPUT_MAX_CHARS and sharing its default.
            let Some(hit) = lines.iter().position(|l| l.contains(name)) else {
                continue;
            };
            let start = hit.saturating_sub(2);
            let block = lines[start..(hit + 4).min(lines.len())].join(" ");
            checked += 1;
            if !block.contains(&literal) {
                wrong.push(format!("{name} applies {literal}, help says: {block:?}"));
            }
        }

        assert!(
            checked >= 8,
            "expected to read most numeric defaults out of config.rs, saw {checked}"
        );
        assert!(
            wrong.is_empty(),
            "`--help` contradicts config.rs: {wrong:#?}"
        );
    }

    /// The README's configuration table must list every variable `config.rs`
    /// reads, with the defaults it applies.
    ///
    /// The same guard 034 and 036 put on `--help`, one file over. That pair
    /// fixed the binary's surface and left the README's identical 22-row table
    /// hand-written and unchecked — the §10 rule (a document may never restate
    /// a fact it could derive) broken in the file next to where it was applied.
    ///
    /// The table is kept hand-written rather than generated, because its
    /// *Purpose* column is reasons and reasons have no source to derive from.
    /// The rule is derive the facts, hand-write the reasons; this checks the
    /// facts and leaves the prose alone.
    #[test]
    fn the_readme_config_table_matches_the_config() {
        let config = include_str!("config.rs");
        // `include_str!` reads the file as it sits on disk, which is CRLF
        // here. Searching for a blank-line boundary without normalising first
        // matches nothing, the table comes out empty, and the test reports
        // every variable missing — which is what it did before this line.
        let readme = include_str!("../README.md").replace("\r\n", "\n");

        let start = readme
            .find("| Variable | Required | Default | Purpose |")
            .expect("README has no configuration table");
        let table_end = readme[start..]
            .find("\n\n")
            .expect("the configuration table is not followed by a blank line");
        let table = &readme[start..start + table_end];
        assert!(
            table.lines().count() > 15,
            "extracted {} table lines; the boundary search is wrong, not the table",
            table.lines().count()
        );

        let mut missing = Vec::new();
        let mut wrong = Vec::new();
        let mut rest = config;
        let marker = "parse_env(";
        while let Some(at) = rest.find(marker) {
            rest = &rest[at + marker.len()..];
            if !rest.starts_with('"') {
                continue;
            }
            rest = &rest[1..];
            let Some(close) = rest.find('"') else { break };
            let name = &rest[..close];
            let after = &rest[close + 1..];

            // A fixture that exists only to prove absent-variable handling.
            if name == "PARALLAX_TEST_DEFINITELY_UNSET_KEY" {
                continue;
            }
            if !table.contains(name) {
                missing.push(name.to_string());
                continue;
            }

            let Some(comma) = after.find(',') else {
                continue;
            };
            let Some(end) = after[comma + 1..].find(')') else {
                continue;
            };
            let literal: String = after[comma + 1..comma + 1 + end]
                .trim()
                .chars()
                .filter(char::is_ascii_digit)
                .collect();
            if literal.is_empty() {
                continue;
            }
            let Some(row) = table.lines().find(|l| l.contains(name)) else {
                continue;
            };
            if !row.contains(&literal) {
                wrong.push(format!("{name} applies {literal}; README row: {row}"));
            }
        }

        assert!(missing.is_empty(), "README config table omits: {missing:?}");
        assert!(wrong.is_empty(), "README contradicts config.rs: {wrong:#?}");

        // Variables read through `env::var` carry no numeric default for the
        // scan above to compare, but must still be listed.
        for name in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_MODEL",
            "ANTHROPIC_API_BASE",
            "VOYAGE_API_KEY",
            "VOYAGE_MODEL",
            "BRAVE_API_KEY",
            "GROUNDED_VERIFY_ROOT",
            "CHECKPOINT_GATE_PATTERNS",
            "DATABASE_PATH",
            "LOG_LEVEL",
        ] {
            assert!(table.contains(name), "README config table omits {name}");
        }
    }

    /// The changelog must carry a dated section for the version being built,
    /// and no document may restate that version as a literal.
    ///
    /// §10: a document may never restate a fact it could derive. The README
    /// said `v0.3.0` in two places, kept in step by whoever remembered — the
    /// same shape as `--help` claiming a 30 000 ms timeout the code had not
    /// used since 018. Those references now point at the changelog instead,
    /// and this pins the one place a version literal still has to be written
    /// by hand.
    #[test]
    fn the_changelog_documents_the_version_being_built() {
        let version = env!("CARGO_PKG_VERSION");
        let changelog = include_str!("../CHANGELOG.md");
        assert!(
            changelog.contains(&format!("## [{version}] - ")),
            "CHANGELOG.md has no dated section for {version}; either the cut              was not made or Cargo.toml moved without it"
        );

        // A released section must not sit above `[Unreleased]`, which would
        // mean the cut was written into the wrong place.
        let unreleased = changelog
            .find("## [Unreleased]")
            .expect("CHANGELOG.md has no [Unreleased] block");
        let released = changelog
            .find(&format!("## [{version}] - "))
            .expect("checked above");
        assert!(
            unreleased < released,
            "the {version} section is above [Unreleased]"
        );

        // The README should point at the changelog rather than name a version
        // it would then have to be reminded to update.
        let readme = include_str!("../README.md");
        assert!(
            !readme.contains(&format!("v{version}.")),
            "README.md restates the version; point it at CHANGELOG.md instead"
        );
    }

    /// Non-numeric defaults, which the scan above cannot read out of a
    /// `parse_env` literal. Small and explicit on purpose.
    #[test]
    fn help_states_the_string_defaults() {
        let help = help_text();
        for expected in ["claude-opus-4-8", "voyage-4", "https://api.anthropic.com"] {
            assert!(
                help.contains(expected),
                "`--help` omits the {expected} default"
            );
        }
        assert!(!help.contains("30000"), "the stale 30000 timeout is back");
    }
}
