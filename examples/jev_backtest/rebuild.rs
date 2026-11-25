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
use std::collections::BTreeMap;

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
pub struct Rebuilt {
    /// The state Jev saw, fingerprint-checked.
    pub state: RedactedState,
    /// The window the server read.
    pub window: TrajectoryWindow,
    /// The turn activity summary.
    pub activity: String,
    /// The final assistant message.
    pub final_message: String,
}

/// Rebuild one stored turn, or say why it can't be.
pub async fn rebuild(row: &JevRecord) -> Result<Rebuilt, &'static str> {
    let path = row.transcript_ref.as_deref().ok_or("no transcript path")?;
    let session = row.harness_session_id.as_deref().ok_or("no session id")?;
    let text = std::fs::read_to_string(path).map_err(|_| "transcript missing")?;
    let cut = measure::transcript_until(&text, row.created_at);
    let final_message = measure::last_assistant_text(&cut).ok_or("no final message")?;
    let temp = std::env::temp_dir().join(format!("jev-rebuild-{}.jsonl", row.id));
    std::fs::write(&temp, format!("{cut}\n")).map_err(|_| "temp file not written")?;
    let window = FsTrajectoryReader
        .read(&temp.to_string_lossy(), session)
        .await;
    let _ = std::fs::remove_file(&temp);
    let window = window.map_err(|_| "window unreadable")?;
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

/// The saved results, or empty ones.
pub fn load(path: &str) -> Results {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
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

/// Replace the skip counts a measurement recorded under `prefix`.
pub fn record_skips(
    results: &mut Results,
    prefix: &str,
    site: &str,
    skips: &BTreeMap<&'static str, u32>,
) {
    let tag = format!("{prefix}: ");
    results
        .skipped
        .retain(|(s, reason, _)| !(s == site && reason.starts_with(&tag)));
    for (reason, count) in skips {
        results
            .skipped
            .push((site.to_string(), format!("{tag}{reason}"), *count));
    }
}
