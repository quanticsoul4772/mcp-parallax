//! The trajectory boundary (checkpoint layer) — the seventh seam.
//!
//! The harness's hooks hand the server a transcript path; everything the
//! detectors consume comes through this trait, so checkpoint logic tests
//! without disk (Principle IV). `FsTrajectoryReader` bounds the new
//! read capability (Principle VI / data-model.md 006 §5): strict path
//! validation, session match, tail window — never an arbitrary file read.

use crate::checkpoint::trajectory::{parse_lines, TrajectoryWindow};
use crate::checkpoint::{WINDOW_BYTES, WINDOW_ENTRIES};
use crate::error::AppError;
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// Bounded, validated access to one session's trajectory.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait TrajectoryReader: Send + Sync {
    /// Read the recent window of the trajectory at `path`, verifying it
    /// belongs to `session_id`.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::ValidationFailure`] for any path-validation or
    /// session-mismatch violation, [`AppError::Storage`] for I/O failures.
    async fn read(&self, path: &str, session_id: &str) -> Result<TrajectoryWindow, AppError>;
}

/// Production [`TrajectoryReader`]: validated, bounded JSONL tail read.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsTrajectoryReader;

#[async_trait::async_trait]
impl TrajectoryReader for FsTrajectoryReader {
    async fn read(&self, path: &str, session_id: &str) -> Result<TrajectoryWindow, AppError> {
        // Validation order (data-model.md 006 §5): canonicalize (fails for a
        // missing file), regular file, `.jsonl` extension — all before any
        // content is read.
        let canonical = tokio::fs::canonicalize(path).await.map_err(|e| {
            AppError::ValidationFailure(format!("transcript path does not resolve: {e}"))
        })?;
        let metadata = tokio::fs::metadata(&canonical)
            .await
            .map_err(|e| AppError::Storage(format!("transcript metadata read failed: {e}")))?;
        if !metadata.is_file() {
            return Err(AppError::ValidationFailure(
                "transcript path is not a regular file".to_string(),
            ));
        }
        if canonical.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            return Err(AppError::ValidationFailure(
                "transcript path does not have the .jsonl extension".to_string(),
            ));
        }

        // Tail window: at most WINDOW_BYTES from the end of the file.
        let mut file = tokio::fs::File::open(&canonical)
            .await
            .map_err(|e| AppError::Storage(format!("transcript open failed: {e}")))?;
        let len = metadata.len();
        let skipped_partial = if len > WINDOW_BYTES {
            file.seek(SeekFrom::Start(len - WINDOW_BYTES))
                .await
                .map_err(|e| AppError::Storage(format!("transcript seek failed: {e}")))?;
            true
        } else {
            false
        };
        // Hard read limiter: the file may grow between the stat and the read
        // (the harness appends to live transcripts) — `take` enforces the
        // bound regardless (review finding 1).
        let mut bytes = Vec::with_capacity(usize::try_from(len.min(WINDOW_BYTES)).unwrap_or(0));
        let mut limited = file.take(WINDOW_BYTES);
        limited
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| AppError::Storage(format!("transcript read failed: {e}")))?;
        let text = String::from_utf8_lossy(&bytes);
        let mut lines: Vec<&str> = text.lines().collect();
        if skipped_partial && !lines.is_empty() {
            // The first line of a mid-file window is almost certainly cut.
            lines.remove(0);
        }

        let (mut entries, found_session) = parse_lines(&lines);
        // Session match is part of the capability bound: the caller may only
        // read the trajectory it claims to be checkpointing.
        match found_session {
            Some(found) if found == session_id => {}
            Some(found) => {
                return Err(AppError::ValidationFailure(format!(
                    "transcript belongs to session {found}, not {session_id}"
                )));
            }
            None => {
                return Err(AppError::ValidationFailure(
                    "transcript carries no session id".to_string(),
                ));
            }
        }
        if entries.len() > WINDOW_ENTRIES {
            entries.drain(..entries.len() - WINDOW_ENTRIES);
        }
        Ok(TrajectoryWindow {
            session_id: session_id.to_string(),
            entries,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn tool_line(session: &str, id: u32, command: &str) -> String {
        json!({
            "type": "assistant",
            "sessionId": session,
            "message": { "role": "assistant", "content": [
                { "type": "tool_use", "id": format!("t{id}"), "name": "Bash",
                  "input": { "command": command } }
            ]}
        })
        .to_string()
    }

    fn write_transcript(dir: &std::path::Path, name: &str, lines: &[String]) -> String {
        let path = dir.join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        path.to_string_lossy().to_string()
    }

    #[tokio::test]
    async fn happy_path_reads_a_session_matched_window() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "t.jsonl",
            &[tool_line("s1", 1, "cargo test"), tool_line("s1", 2, "ls")],
        );
        let window = FsTrajectoryReader.read(&path, "s1").await.unwrap();
        assert_eq!(window.session_id, "s1");
        assert_eq!(window.entries.len(), 2);
    }

    #[tokio::test]
    async fn missing_file_is_a_validation_failure() {
        let err = FsTrajectoryReader
            .read("definitely/not/here.jsonl", "s1")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ValidationFailure(_)), "{err}");
    }

    #[tokio::test]
    async fn wrong_extension_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_transcript(dir.path(), "t.txt", &[tool_line("s1", 1, "x")]);
        let err = FsTrajectoryReader.read(&path, "s1").await.unwrap_err();
        assert!(err.to_string().contains(".jsonl"), "{err}");
    }

    #[tokio::test]
    async fn session_mismatch_is_rejected_naming_both_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_transcript(dir.path(), "t.jsonl", &[tool_line("other", 1, "x")]);
        let err = FsTrajectoryReader.read(&path, "s1").await.unwrap_err();
        let text = err.to_string();
        assert!(text.contains("other") && text.contains("s1"), "{text}");
    }

    #[tokio::test]
    async fn sessionless_transcript_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_transcript(dir.path(), "t.jsonl", &["{}".to_string()]);
        let err = FsTrajectoryReader.read(&path, "s1").await.unwrap_err();
        assert!(err.to_string().contains("no session id"), "{err}");
    }

    #[tokio::test]
    async fn oversize_transcript_truncates_to_the_tail_window() {
        let dir = tempfile::tempdir().unwrap();
        // Far more entries than WINDOW_ENTRIES; each line is unique.
        let lines: Vec<String> = (0..(u32::try_from(WINDOW_ENTRIES).unwrap() + 50))
            .map(|i| tool_line("s1", i, &format!("cmd {i}")))
            .collect();
        let path = write_transcript(dir.path(), "t.jsonl", &lines);
        let window = FsTrajectoryReader.read(&path, "s1").await.unwrap();
        assert_eq!(window.entries.len(), WINDOW_ENTRIES);
        // The window is the TAIL: the last command must be present.
        let last = window.entries.last().unwrap();
        let crate::checkpoint::trajectory::TrajectoryEntry::ToolCall {
            normalized_input, ..
        } = last
        else {
            panic!("expected a tool call");
        };
        assert!(normalized_input.contains(&format!("cmd {}", WINDOW_ENTRIES + 49)));
    }
}
