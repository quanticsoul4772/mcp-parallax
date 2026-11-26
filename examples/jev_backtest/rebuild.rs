//! Turn rebuilding and the saved results file (064 FR-011, SC-006).
//!
//! Turn rows store only a fingerprint of the state Jev saw and the
//! transcript's path. A stored turn is rebuilt by cutting its transcript at
//! the row's time, reading the window with the server's own reader, and
//! building the state with the server's own function; it is used only when
//! the fingerprint matches.

use mcp_parallax::checkpoint::review;
use mcp_parallax::checkpoint::trajectory::TrajectoryWindow;
use mcp_parallax::error::Outcome;
use mcp_parallax::jev::measure::{self, Results};
use mcp_parallax::jev::metrics::{split, Split};
use mcp_parallax::jev::redact::{RedactedState, Redactor};
use mcp_parallax::jev::report::CONFIRM_PERCENT;
use mcp_parallax::jev::shadow::turn_state;
use mcp_parallax::storage::jev::{JevOutcome, JevRecord};
use mcp_parallax::traits::classifier::JevSite;
use mcp_parallax::traits::trajectory::{FsTrajectoryReader, TrajectoryReader};
use std::path::Path;

/// Where live runs save their results, beside the databases (gitignored).
pub const RESULTS_FILE: &str = "./data/jev-measurements.json";

/// A successful tune-split row of `site`; the sealed confirm split is never
/// read.
pub fn tune_success(row: &JevRecord, site: JevSite) -> bool {
    row.site == site
        && row.outcome == JevOutcome::Call(Outcome::Success)
        && split(&row.id, CONFIRM_PERCENT) == Split::Tune
}

/// A stored turn, rebuilt.
///
/// The fingerprint covers the redacted state: the statement pairs, the
/// activity line and the first excerpt of the final message. Every input the
/// measurements use comes from those, so a match means the measured inputs
/// are the server's. The window and final message themselves may still
/// differ from the server's in ways the state does not show (an entry that
/// mined no pair, text past the excerpt).
pub struct Rebuilt {
    /// The state Jev saw, fingerprint-checked.
    pub state: RedactedState,
    /// The window as rebuilt.
    pub window: TrajectoryWindow,
    /// The turn activity summary.
    pub activity: String,
    /// The final assistant message as rebuilt.
    pub final_message: String,
}

/// Rebuild one stored turn, or say why it can't be.
pub async fn rebuild(row: &JevRecord) -> Result<Rebuilt, &'static str> {
    let path = row.transcript_ref.as_deref().ok_or("no transcript path")?;
    let session = row.harness_session_id.as_deref().ok_or("no session id")?;
    let bytes = std::fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "transcript missing"
        } else {
            "transcript unreadable"
        }
    })?;
    // Lossy, as the server's reader decodes it.
    let text = String::from_utf8_lossy(&bytes);
    let cut = measure::transcript_until(&text, row.created_at);
    let final_message = measure::last_assistant_text(&cut).ok_or("no final message")?;
    let temp = std::env::temp_dir().join(format!("jev-rebuild-{}.jsonl", row.id));
    std::fs::write(&temp, cut.as_bytes()).map_err(|_| "temp file not written")?;
    let window = FsTrajectoryReader
        .read(&temp.to_string_lossy(), session)
        .await;
    let _ = std::fs::remove_file(&temp);
    let (window, _lines) = window.map_err(|_| "window unreadable")?;
    let activity = review::turn_activity(&window);
    let built = turn_state(&Redactor::from_env(), &window, &activity, &final_message)
        .map_err(|_| "redaction failed")?;
    if row.state_hash.as_deref() != Some(built.hash.as_str()) {
        return Err("fingerprint mismatch");
    }
    Ok(Rebuilt {
        state: built.state,
        window,
        activity,
        final_message,
    })
}

/// The question ids a stored row asked.
pub fn asked(row: &JevRecord) -> Vec<String> {
    row.questions_json
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|q| q["id"].as_str().map(String::from))
        .collect()
}

/// The saved results; a run with no file yet starts empty. A broken file
/// stops the run rather than being overwritten.
pub fn load() -> Results {
    measure::read_results(Path::new(RESULTS_FILE), false).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    })
}

/// Save the results, replacing the file.
pub fn save(results: &Results) {
    std::fs::create_dir_all("data").expect("data folder");
    std::fs::write(
        RESULTS_FILE,
        serde_json::to_string_pretty(results).expect("results serialize"),
    )
    .expect("results file");
}

/// Replace the skip and loss counts a measurement recorded under `prefix`.
pub fn record_skips(
    results: &mut Results,
    prefix: &str,
    site: &str,
    skips: &std::collections::BTreeMap<String, u32>,
) {
    let tag = format!("{prefix}: ");
    results
        .skipped
        .retain(|(s, reason, _)| !(s == site && reason.starts_with(&tag)));
    for (reason, count) in skips {
        if *count > 0 {
            results
                .skipped
                .push((site.to_string(), format!("{tag}{reason}"), *count));
        }
    }
}
