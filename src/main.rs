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

use mcp_parallax::client::default_model_client;
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

    // BYOM (design §3.1): the backend is chosen by `PARALLAX_BACKEND`; the
    // factory hands back the right adapter and the server never knows which.
    let client = default_model_client(&config);
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
        backend = config.backend.as_str(),
        model = %config.default_model(),
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

MODEL BACKEND (one backend for every call site, chosen at startup):
    PARALLAX_BACKEND              Model backend: anthropic or openai_compat
                                  (default: anthropic)
    ANTHROPIC_API_KEY             Anthropic API key; REQUIRED when PARALLAX_BACKEND
                                  is anthropic (the default), unused otherwise
    ANTHROPIC_MODEL               Default model id for the anthropic backend
                                  (default: claude-opus-4-8)
    ANTHROPIC_API_BASE            Anthropic API endpoint
                                  (default: https://api.anthropic.com)
    OPENAI_API_KEY                OpenAI-compatible API key; REQUIRED when
                                  PARALLAX_BACKEND is openai_compat and the
                                  endpoint is the default OpenAI one; keyless
                                  local endpoints (Ollama, LM Studio, vLLM)
                                  need none
    OPENAI_MODEL                  Model id on the openai_compat endpoint; REQUIRED
                                  when PARALLAX_BACKEND is openai_compat
    OPENAI_API_BASE               OpenAI-compatible API endpoint, /v1-suffixed
                                  (default: https://api.openai.com/v1)
    OPENAI_STRUCTURED_OUTPUT      Structured-output ladder for openai_compat:
                                  auto, json_schema, json_object, tool_shim,
                                  prompt_only (default: auto)

CORE (always read):
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
    EXA_API_KEY                   Enables `research` (Exa search and page text)
    GROUNDED_VERIFY_ROOT          Enables `grounded_verify`; the single root that
                                  locators resolve within, canonicalized at startup;
                                  files named as secrets (.env, keys) are refused;
                                  `.` gives each session its own folder

RESEARCH:
    FETCH_TIMEOUT_MS              Per-source fetch timeout in ms (default: 10000)
    EXA_MAX_AGE_HOURS             Page-text freshness in hours, -1..=720; 0 always
                                  live, -1 cache only (default: 168)
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
    On the openai_compat backend, effort is a documented no-op: the level is
    dropped at the wire and a notice is logged, never silently mistranslated

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

/// Checks CLAUDE.md's active-feature block, paragraph by paragraph. Every
/// `specs/<name>` link must exist; every "N tasks closed" claim is checked
/// against the link in its own paragraph (closed count equals N, none open),
/// and a claim with no link in its paragraph is a problem, never skipped.
/// Returns how many links were checked and the problems found.
#[cfg(test)]
fn active_block_problems(
    block: &str,
    exists: &dyn Fn(&str) -> bool,
    tasks: &dyn Fn(&str) -> String,
) -> (usize, Vec<String>) {
    let mut checked = 0;
    let mut problems = Vec::new();
    for paragraph in block.split("\n\n") {
        // Each link with its byte offset in the paragraph, so a claim can be
        // paired with the link nearest it (review of 056).
        let mut links: Vec<(usize, String)> = Vec::new();
        let mut from = 0;
        while let Some(found) = paragraph[from..].find("specs/") {
            let at = from + found;
            from = at + "specs/".len();
            let name: String = paragraph[from..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !name.is_empty() {
                if !exists(&name) {
                    problems.push(format!(
                        "the block links `specs/{name}`, which does not exist"
                    ));
                }
                checked += 1;
                links.push((at, name));
            }
        }

        let mut from = 0;
        while let Some(found) = paragraph[from..].find("tasks closed") {
            let marker = from + found;
            from = marker + "tasks closed".len();
            // Only the digits directly before the claim, past whitespace, are
            // its count: "055 followed; all tasks closed" has no number.
            let count: String = paragraph[..marker]
                .chars()
                .rev()
                .skip_while(|c| c.is_whitespace())
                .take_while(char::is_ascii_digit)
                .collect();
            let Ok(claimed) = count.chars().rev().collect::<String>().parse::<usize>() else {
                problems.push("a \"tasks closed\" claim has no number before it".to_string());
                continue;
            };
            let Some((_, name)) = links.iter().min_by_key(|(at, _)| at.abs_diff(marker)) else {
                problems.push(format!(
                    "the claim \"{claimed} tasks closed\" has no specs/ link in its paragraph"
                ));
                continue;
            };
            let text = tasks(name);
            let closed = text.matches("- [X] ").count() + text.matches("- [x] ").count();
            let open = text.matches("- [ ] ").count();
            if closed != claimed {
                problems.push(format!(
                    "the block claims {claimed} tasks closed for {name}; tasks.md has {closed}"
                ));
            }
            if open != 0 {
                problems.push(format!("{name} has {open} tasks still open"));
            }
        }
    }
    (checked, problems)
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

        // Keys used only by config.rs's own tests, derived from its test module.
        let fixtures = crate::config_facts::test_only_keys(include_str!("config.rs"));

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
            if looks_like_env && !fixtures.iter().any(|f| f == token) && !help.contains(token) {
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
    /// CLAUDE.md's active-feature block was the last operator-facing document
    /// with nothing checking it, and it went stale exactly as `--help` and the
    /// README did before 034 and 039.
    ///
    /// **Only the derivable parts are bound, and the limit is the point.**
    /// "Which feature is active" is a statement of intent — no source of truth
    /// exists for it, so no check can catch the block naming an old feature
    /// while newer ones ship. What *is* derivable is checked: every
    /// `specs/<dir>` the block links must exist, and any claim of the form
    /// "all N tasks closed" must match that feature's `tasks.md`.
    ///
    /// Claiming otherwise would be worse than the gap. A check that appears to
    /// cover staleness while covering only link validity is the shape 036 and
    /// 039 shipped — partial derivation reported as derivation.
    /// A claim belongs to its own paragraph's link, not to every link in the
    /// block: a 056 pointer paragraph before 040's "all 34 tasks closed"
    /// paragraph was checked against 040's count (2026-11-20).
    #[test]
    fn a_task_count_claim_is_checked_against_its_own_paragraphs_link() {
        let block = "Active: `056` ([plan](specs/056-x/plan.md)).\n\n\
                     The last run was 040 — all\n34 tasks closed ([plan](specs/040-y/plan.md)).";
        let exists = |_: &str| true;
        let tasks = |name: &str| match name {
            "056-x" => "- [ ] T001 open\n".to_string(),
            _ => "- [X] T\n".repeat(34),
        };
        let (checked, problems) = super::active_block_problems(block, &exists, &tasks);
        assert_eq!(checked, 2);
        assert!(problems.is_empty(), "{problems:?}");
    }

    /// Review of 056: a paragraph linking two features is checked against the
    /// link nearest each claim, not the paragraph's first link.
    #[test]
    fn a_claim_is_checked_against_the_nearest_link_in_its_paragraph() {
        let block = "040 shipped ([plan](specs/040-x/plan.md)); 055 followed, all 2 tasks \
                     closed ([plan](specs/055-y/plan.md)).";
        let tasks = |name: &str| match name {
            "055-y" => "- [X] a\n- [X] b\n".to_string(),
            _ => "- [X] a\n".to_string(),
        };
        let (_, problems) = super::active_block_problems(block, &|_| true, &tasks);
        assert!(problems.is_empty(), "{problems:?}");
    }

    /// Review of 056: only the digits directly before "tasks closed" (past
    /// whitespace) are the count; "055 ... all tasks closed" has none.
    #[test]
    fn a_number_far_before_the_claim_is_not_its_count() {
        let block = "055 followed; all tasks closed ([plan](specs/055-y/plan.md)).";
        let (_, problems) = super::active_block_problems(block, &|_| true, &|_| String::new());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("no number"), "{problems:?}");
    }

    /// A claim with no specs/ link in its paragraph cannot be checked, and is
    /// reported rather than skipped.
    #[test]
    fn a_task_count_claim_without_a_link_in_its_paragraph_is_a_problem() {
        let block = "055 followed; all 47 tasks closed.";
        let (_, problems) = super::active_block_problems(block, &|_| true, &|_| String::new());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("47"), "{problems:?}");
    }

    /// A claimed count that differs from the linked tasks.md is a problem, and
    /// so is an open task.
    #[test]
    fn a_wrong_task_count_or_an_open_task_is_a_problem() {
        let block = "all 3 tasks closed ([plan](specs/040-y/plan.md)).";
        let two = |_: &str| "- [X] a\n- [X] b\n".to_string();
        let (_, problems) = super::active_block_problems(block, &|_| true, &two);
        assert_eq!(problems.len(), 1, "{problems:?}");
        let open = |_: &str| "- [X] a\n- [X] b\n- [X] c\n- [ ] d\n".to_string();
        let (_, problems) = super::active_block_problems(block, &|_| true, &open);
        assert_eq!(problems.len(), 1, "{problems:?}");
    }

    /// 058: GitHub task lists accept `[x]` as well as `[X]`; a lowercase tick
    /// is a closed task, not a missing one.
    #[test]
    fn a_lowercase_tick_counts_as_closed() {
        let block = "all 2 tasks closed ([plan](specs/058-y/plan.md)).";
        let mixed = |_: &str| "- [x] a\n- [X] b\n".to_string();
        let (_, problems) = super::active_block_problems(block, &|_| true, &mixed);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn the_active_feature_block_links_and_task_counts_are_real() {
        let claude_md = include_str!("../CLAUDE.md").replace("\r\n", "\n");
        let start = claude_md
            .find("<!-- SPECKIT START -->")
            .expect("CLAUDE.md has no active-feature block");
        let end = claude_md
            .find("<!-- SPECKIT END -->")
            .expect("the active-feature block is unterminated");
        let block = &claude_md[start..end];

        let specs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("specs");
        let exists = |name: &str| specs.join(name).is_dir();
        let tasks = |name: &str| {
            std::fs::read_to_string(specs.join(name).join("tasks.md")).unwrap_or_default()
        };
        let (checked, problems) = super::active_block_problems(block, &exists, &tasks);
        assert!(problems.is_empty(), "{}", problems.join("\n"));
        assert!(
            checked > 0,
            "no specs/ link found in the active-feature block; the search is wrong, not the block"
        );
    }

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

    /// The pinned toolchain is written in `rust-toolchain.toml` and again in
    /// every workflow job, and the MSRV job pins `Cargo.toml`'s
    /// `rust-version`. Nothing else binds them. Every workflow file is read
    /// at test time, so one added later is covered too.
    #[test]
    fn every_workflow_pins_the_toolchain_files_version() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let quoted = |text: &str, key: &str| -> Option<String> {
            let line = text.lines().find(|l| l.trim_start().starts_with(key))?;
            line.split('"').nth(1).map(str::to_string)
        };
        let channel = quoted(include_str!("../rust-toolchain.toml"), "channel")
            .expect("rust-toolchain.toml has no quoted channel");
        let msrv = quoted(include_str!("../Cargo.toml"), "rust-version")
            .expect("Cargo.toml has no quoted rust-version");
        let msrv_pin = format!("{msrv}.0");

        let marker = "dtolnay/rust-toolchain@";
        let mut pins = 0;
        let mut msrv_pins = 0;
        let mut files: Vec<_> = std::fs::read_dir(root.join(".github/workflows"))
            .expect("no .github/workflows folder")
            .map(|entry| entry.expect("unreadable workflow entry").path())
            .collect();
        files.sort();
        for path in files {
            let text = std::fs::read_to_string(&path).expect("unreadable workflow file");
            let name = path.display();
            let mut file_pins = 0;
            for (n, line) in text.lines().enumerate() {
                let Some(at) = line.find(marker) else {
                    continue;
                };
                let pin = line[at + marker.len()..].trim();
                file_pins += 1;
                if pin == msrv_pin {
                    msrv_pins += 1;
                } else {
                    assert_eq!(
                        pin,
                        channel,
                        "{name}:{} pins {pin}, but rust-toolchain.toml says {channel}",
                        n + 1
                    );
                }
            }
            assert!(
                file_pins > 0 || !text.contains("cargo "),
                "{name} runs cargo but pins no toolchain"
            );
            pins += file_pins;
        }
        assert!(
            pins > 0,
            "no toolchain pin found in any workflow; the search is wrong"
        );
        assert_eq!(
            msrv_pins, 1,
            "expected exactly one pin of the MSRV {msrv_pin} (Cargo.toml rust-version {msrv})"
        );
    }

    /// Every command starting with `prefix` (e.g. `cargo clippy`) in `text`,
    /// cut where the command ends in prose, markdown or shell (`&&`, a
    /// backtick, a quote, ` #`).
    fn commands(prefix: &str, text: &str) -> Vec<String> {
        let mut found = Vec::new();
        for line in text.lines() {
            let mut rest = line;
            while let Some(at) = rest.find(prefix) {
                let tail = &rest[at..];
                let end = ["&&", "`", "'", "\"", " #"]
                    .iter()
                    .filter_map(|stop| tail.find(stop))
                    .min()
                    .unwrap_or(tail.len());
                found.push(tail[..end].trim().to_string());
                rest = &tail[end..];
            }
        }
        found
    }

    /// ci.yml's clippy commands. Its header comment tells people to run
    /// `cargo +<ver> clippy …` to reproduce the gate; that is read as
    /// `cargo clippy …` so it is held to the same command as the jobs.
    fn ci_clippy_commands() -> Vec<String> {
        let mut pinned = 0;
        let ci_text = include_str!("../.github/workflows/ci.yml")
            .lines()
            .map(|line| {
                if let Some(at) = line.find("cargo +") {
                    let after = &line[at + "cargo +".len()..];
                    if let Some(space) = after.find(' ') {
                        if after[space + 1..].starts_with("clippy") {
                            pinned += 1;
                            return format!("{}cargo {}", &line[..at], &after[space + 1..]);
                        }
                    }
                }
                line.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            pinned > 0,
            "ci.yml's `cargo +<ver> clippy` reproduce line was not found"
        );
        // A command continued onto the next line would leave only its first
        // line as the reference, and every document would be compared to that.
        assert!(
            !ci_text
                .lines()
                .any(|l| l.contains("cargo clippy") && l.trim_end().ends_with('\\')),
            "a ci.yml clippy command continues onto the next line; the reference would be cut short"
        );
        commands("cargo clippy", &ci_text)
    }

    /// The lint gate is written in CI and again in every file that tells a
    /// person or an agent how to run it. 044 widened CI to `--all-targets`;
    /// five of those files kept the narrower command, so `/validate` could
    /// pass while CI failed on a lint error in a test. CI is the reference.
    #[test]
    fn every_declared_clippy_command_matches_ci() {
        let ci = ci_clippy_commands();
        let reference = ci.first().expect("no `cargo clippy` run in ci.yml").clone();
        assert!(
            ci.iter().all(|c| *c == reference),
            "ci.yml runs clippy two different ways: {ci:?}"
        );

        let alias = include_str!("../.cargo/config.toml")
            .lines()
            .find(|l| l.split('=').next().is_some_and(|key| key.trim() == "lint"))
            .and_then(|l| l.split('"').nth(1))
            .map(|v| vec![format!("cargo {v}")])
            .unwrap_or_default();
        // CLAUDE.md describes the alias as `cargo lint   # clippy …`.
        let lint_line = commands(
            "cargo clippy",
            &include_str!("../CLAUDE.md")
                .lines()
                .filter(|l| l.contains("cargo lint"))
                .filter_map(|l| {
                    l.find("# clippy")
                        .map(|at| format!("cargo {}", &l[at + 2..]))
                })
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let declared: [(&str, Vec<String>); 10] = [
            (
                "CLAUDE.md",
                commands("cargo clippy", include_str!("../CLAUDE.md")),
            ),
            ("CLAUDE.md (cargo lint line)", lint_line),
            (
                "README.md",
                commands("cargo clippy", include_str!("../README.md")),
            ),
            (
                "CONTRIBUTING.md",
                commands("cargo clippy", include_str!("../CONTRIBUTING.md")),
            ),
            (
                ".specify/memory/constitution.md",
                commands(
                    "cargo clippy",
                    include_str!("../.specify/memory/constitution.md"),
                ),
            ),
            (
                ".claude/commands/validate.md",
                commands(
                    "cargo clippy",
                    include_str!("../.claude/commands/validate.md"),
                ),
            ),
            (
                ".claude/agents/code-reviewer.md",
                commands(
                    "cargo clippy",
                    include_str!("../.claude/agents/code-reviewer.md"),
                ),
            ),
            (
                ".github/pull_request_template.md",
                commands(
                    "cargo clippy",
                    include_str!("../.github/pull_request_template.md"),
                ),
            ),
            // Its `name:` key is a display label that also reads
            // `cargo clippy`; only `entry:` is run.
            (
                ".pre-commit-config.yaml",
                commands(
                    "cargo clippy",
                    &include_str!("../.pre-commit-config.yaml")
                        .lines()
                        .filter(|l| l.trim_start().starts_with("entry:"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            ),
            (".cargo/config.toml (lint alias)", alias),
        ];

        let mut problems = Vec::new();
        for (file, commands) in &declared {
            if commands.is_empty() {
                problems.push(format!("{file}: no clippy command found"));
            }
            for command in commands {
                if *command != reference {
                    problems.push(format!("{file}: `{command}`"));
                }
            }
        }
        assert!(
            problems.is_empty(),
            "these differ from ci.yml's `{reference}`:\n{}",
            problems.join("\n")
        );
    }

    /// The gate's fmt and test commands, bound to ci.yml the same way as its
    /// clippy command. CLAUDE.md's `cargo test <module>` and `cargo test
    /// <name> -- --exact` lines are usage examples, not the gate, and the
    /// pre-commit fmt hook formats rather than checks; neither is read.
    #[test]
    fn every_declared_fmt_and_test_command_matches_ci() {
        let ci_text = include_str!("../.github/workflows/ci.yml");
        let reference = |prefix: &str| {
            let runs = commands(prefix, ci_text);
            assert!(!runs.is_empty(), "no `{prefix}` run in ci.yml");
            assert!(
                !ci_text
                    .lines()
                    .any(|l| l.contains(prefix) && l.trim_end().ends_with('\\')),
                "a ci.yml `{prefix}` command continues onto the next line; the reference would be cut short"
            );
            let first = runs[0].clone();
            assert!(
                runs.iter().all(|r| *r == first),
                "ci.yml runs `{prefix}` two different ways: {runs:?}"
            );
            first
        };
        let fmt = reference("cargo fmt");
        let test = reference("cargo test");

        let claude: String = include_str!("../CLAUDE.md")
            .lines()
            .filter(|l| !l.contains("cargo test <"))
            .collect::<Vec<_>>()
            .join("\n");
        let files: [(&str, &str, bool); 7] = [
            ("CLAUDE.md", &claude, true),
            ("README.md", include_str!("../README.md"), true),
            ("CONTRIBUTING.md", include_str!("../CONTRIBUTING.md"), true),
            (
                ".specify/memory/constitution.md",
                include_str!("../.specify/memory/constitution.md"),
                true,
            ),
            (
                ".claude/commands/validate.md",
                include_str!("../.claude/commands/validate.md"),
                true,
            ),
            (
                ".claude/agents/code-reviewer.md",
                include_str!("../.claude/agents/code-reviewer.md"),
                false,
            ),
            (
                ".github/pull_request_template.md",
                include_str!("../.github/pull_request_template.md"),
                true,
            ),
        ];

        let mut problems = Vec::new();
        // The bool says whether the file must declare an fmt command. A file
        // that need not (code-reviewer.md) is still checked if it gains one.
        for (file, text, declares_fmt) in files {
            for (prefix, expected, required) in [
                ("cargo test", &test, true),
                ("cargo fmt", &fmt, declares_fmt),
            ] {
                let found = commands(prefix, text);
                if required && found.is_empty() {
                    problems.push(format!("{file}: no `{prefix}` command found"));
                }
                for command in found.iter().filter(|c| *c != expected) {
                    problems.push(format!("{file}: `{command}` (ci.yml: `{expected}`)"));
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
