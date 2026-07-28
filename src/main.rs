//! Parallax MCP server — binary entry point.
//!
//! All logs go to stderr; stdout is reserved for MCP JSON-RPC. Construction
//! order is config → storage (migration at boot) → client → server →
//! serve(stdio): every misconfiguration fails here, named, before the first
//! tool call.

// The binary entry point is a production path too — no panics via unwrap/expect.
#![deny(clippy::unwrap_used, clippy::expect_used)]

#[cfg(test)]
mod config_facts;

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
        std::env::var("LOG_LEVEL")
            .unwrap_or_else(|_| mcp_parallax::config::DEFAULT_LOG_LEVEL.to_string())
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
        // Stripped, like the other two scans. A variable name quoted inside
        // any comment — `// RETRY_BACKOFF_MS was removed in 041` — otherwise
        // becomes a variable `--help` is required to document, and the failure
        // blames a correct document for omitting something that does not exist.
        let config = crate::config_facts::strip_comments(include_str!("config.rs"));
        let help = help_text();

        // Fixtures that exist only to prove absent/invalid handling, and the
        // alias the help mentions by name but does not advertise as canonical.
        let fixtures = [
            "PARALLAX_TEST_DEFINITELY_UNSET_KEY",
            "PARALLAX_MODEL_NOT_A_CALL_SITE",
        ];

        let mut missing = Vec::new();
        let mut rest = config.as_str();
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
        use mcp_parallax::routing::{CallSite, Effort, Tier};

        let help = help_text();
        for prefix in ["PARALLAX_MODEL_", "PARALLAX_EFFORT_"] {
            assert!(help.contains(prefix), "`--help` omits {prefix}*");
        }

        // Whole tokens, never `contains`. `VERIFY` is a substring of both
        // `GROUNDED_VERIFY` and `RESEARCH_VERIFY`, and `high` of `xhigh`, so a
        // substring check passes on a help body that lists only the longer
        // name — the collision that made 040's window comparison useless.
        let tokens: std::collections::HashSet<&str> = help
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|s| !s.is_empty())
            .collect();

        // Derived from the enums, not listed here. The three vocabularies were
        // pinned by hand -- three of twelve call sites -- so a thirteenth site
        // would ship invisible to `--help`, which is 034's defect exactly.
        for site in CallSite::ALL {
            assert!(
                tokens.contains(site.suffix()),
                "`--help` omits the {} call site; PARALLAX_MODEL_{} and PARALLAX_EFFORT_{} are \
                 settable but undocumented",
                site.suffix(),
                site.suffix(),
                site.suffix()
            );
        }
        for tier in Tier::ALL {
            assert!(
                tokens.contains(tier.suffix()),
                "`--help` omits the {} tier",
                tier.suffix()
            );
        }
        for level in Effort::ALL {
            assert!(
                tokens.contains(level.as_str()),
                "`--help` omits the `{}` effort level",
                level.as_str()
            );
        }
    }

    /// 040 / FR-001, FR-006, FR-007: every default `config.rs` applies is
    /// resolved — not merely the ones written as a bare numeric literal — and
    /// every document states the resolved value.
    ///
    /// Replaces three tests: a numeric scan that skipped any default with no
    /// digits, a hand-written list of three strings, and a second copy of the
    /// numeric scan for the README that had already drifted from the first.
    /// Those two copies are how 039 inherited 036's blind spot.
    #[test]
    fn every_default_resolves_and_every_document_states_it() {
        let facts = crate::config_facts::resolve();

        crate::config_facts::assert_all_resolved(&facts);
        crate::config_facts::assert_exclusions_are_live(&facts);
        crate::config_facts::assert_coverage_balances(&facts);
        crate::config_facts::assert_extraction_is_complete(
            crate::config_facts::SOURCES[0].1,
            &facts,
        );

        let help = help_text();
        let readme = include_str!("../README.md").replace("\r\n", "\n");
        let table = readme_table(&readme);

        let help_defaults = crate::config_facts::stated_in_help(&help);
        let readme_defaults = crate::config_facts::stated_in_table(table);

        let mut wrong = Vec::new();
        for fact in &facts {
            let crate::config_facts::Resolution::Resolved(value) = &fact.resolution else {
                continue;
            };
            // Compared against each document's own structured default marker —
            // the README's Default column, `--help`'s `(default: X)` — and
            // never against a window of surrounding lines. A window search
            // asks only whether the value appears *somewhere nearby*, so a
            // single-digit default matches any neighbouring row containing
            // that digit. Setting `MAX_RETRIES` to a wrong `99` passed under
            // the window; `999999` failed only because six digits are rare
            // enough not to collide. That is a check whose sensitivity depends
            // on how unusual the value is, which is not a check.
            for (doc, stated) in [("--help", &help_defaults), ("README.md", &readme_defaults)] {
                let Some(claim) = stated.iter().find(|s| s.name == fact.name) else {
                    wrong.push(format!(
                        "DEFAULT_UNDOCUMENTED: `{}` carries a default that {doc} does not state",
                        fact.name
                    ));
                    continue;
                };
                if &claim.value != value {
                    wrong.push(format!(
                        "DEFAULT_MISMATCH: `{}` applies {value}; {doc} says {}",
                        fact.name, claim.value
                    ));
                }
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));

        // Reverse direction (FR-009): every default a document states must
        // belong to a variable that has one. The forward pass above iterates
        // variables the source still contains, so a row left behind after a
        // variable was removed is invisible to it.
        //
        // Both documents, not just the README. A `(default: ...)` in `--help`
        // for a removed variable, or one bound to the wrong name, was invisible
        // while only one side was checked. There is no count floor here: a
        // `stated.len() >= 10` threshold is the `checked >= 8` shape this
        // feature exists to abolish, reinstated one file over. Extraction
        // failing is already reported by EXTRACTION_EMPTY and by every variable
        // turning up undocumented at once.
        for (doc, stated) in [("--help", &help_defaults), ("README.md", &readme_defaults)] {
            crate::config_facts::assert_no_contradictory_defaults(doc, stated);
            crate::config_facts::assert_no_phantom_defaults(&facts, doc, stated);
        }
    }

    /// 040 / FR-008: variables with **no** default are invisible to the
    /// resolver by design, which makes them the silent casualty of deleting the
    /// old scans. They must still be required to appear in both documents.
    #[test]
    fn variables_without_a_default_are_still_required_to_be_listed() {
        let help = help_text();
        let readme = include_str!("../README.md").replace("\r\n", "\n");
        let table = readme_table(&readme);
        let facts = crate::config_facts::resolve();

        // Derived, not listed. A hand-written list here is 034's defect kept
        // alive inside the feature that exists to abolish hand-written lists:
        // a new capability gate would be checked against `--help` by the
        // generic scan and against the README by nothing at all.
        let names = crate::config_facts::variables_without_defaults(
            crate::config_facts::SOURCES[0].1,
            &facts,
        );
        assert!(
            names.len() >= 5,
            "expected the known capability gates and the required key; got {names:?}"
        );
        for name in &names {
            assert!(
                !facts.iter().any(|f| &f.name == name),
                "{name} has no default, so the resolver must not report one"
            );
            assert!(help.contains(name.as_str()), "--help omits {name}");
            assert!(
                table.contains(name.as_str()),
                "README config table omits {name}"
            );
        }
    }

    /// The README's configuration table, bounded and sanity-checked.
    ///
    /// 039 shipped a boundary search over a CRLF file that matched nothing,
    /// extracted an empty table, and reported every variable missing — blaming
    /// the document for its own parsing bug. The row-count assertion is what
    /// makes the next boundary break say which side is wrong.
    fn readme_table(readme: &str) -> &str {
        let start = readme
            .find("| Variable | Required | Default | Purpose |")
            .expect("README has no configuration table");
        let end = readme[start..]
            .find("\n\n")
            .expect("the configuration table is not followed by a blank line");
        let table = &readme[start..start + end];
        assert!(
            table.lines().count() > 15,
            "EXTRACTION_EMPTY: found {} table rows, expected more than 15. The boundary search is wrong, not the document.",
            table.lines().count()
        );
        table
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
            "CHANGELOG.md has no dated section for {version}; either the cut was not made or Cargo.toml moved without it"
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
}
