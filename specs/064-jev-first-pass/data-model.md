# Data model: 064

## 1. `jev_records` table (`src/storage/jev.rs`)

One row per Jev request: one per reviewed turn, one per verified claim. Excluded claims also get one row each, marked `excluded`, with no request sent.

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | UUID v4 |
| `created_at` | TEXT | RFC 3339, from `TimeProvider` |
| `server_session_id` | TEXT | Process session UUID (the same value as `InvocationRecord.session_id`) |
| `harness_session_id` | TEXT NULL | Claude Code session id (turn sites; same as `CheckpointRecord.session_id`) |
| `site` | TEXT | `turn_review` \| `research_claims` (from `JevSite::ALL`) |
| `mode` | TEXT | `shadow` in 064 (from `JevMode::ALL`) |
| `questions_json` | TEXT | `[{id, version, option_order, answer, probabilities, band}]` (see §2) |
| `model_requested` | TEXT | e.g. `jev-1.14.0` |
| `model_answered` | TEXT NULL | From the response; NULL if no response |
| `outcome` | TEXT | One of `Outcome::ALL`, or `excluded` |
| `error_class` | TEXT NULL | Short class, e.g. `http_429`, `deadline`, `validator:choice_not_declared` |
| `request_id` | TEXT NULL | `x-typesafe-request-id` |
| `latency_ms` | INTEGER NULL | Request wall time |
| `input_tokens` | INTEGER NULL | From `usage` |
| `cost_usd` | REAL NULL | `input_tokens` × price |
| `applied` | INTEGER | Always 0 in 064 |
| `eval_id` | TEXT NULL | Checkpoint record id (turn sites) |
| `invocation_id` | TEXT NULL | Invocation record id (research sites) |
| `claim_key` | TEXT NULL | Normalized claim key (research) |
| `source_ids` | TEXT NULL | JSON array of source ids, in question order (research) |
| `label_json` | TEXT NULL | Enumerated big-model label (see §5); NULL if the review failed open |
| `label_source` | TEXT NULL | Model id plus prompt version that produced the label |
| `state_json` | TEXT NULL | Research only: the redacted state sent, for replay |
| `state_hash` | TEXT NULL | Turn sites: FNV-1a 64 (hex) of the redacted state; matches rows to states on replay, not a security hash |
| `transcript_ref` | TEXT NULL | Turn sites: transcript path |
| `redaction_count` | INTEGER | Number of redactions applied |
| `truncated` | INTEGER | 1 if the state was truncated (no decision is made) |

**Indexes:** `(site, created_at)`, `eval_id`, `invocation_id`.

**Link rules:**

- A turn-site row always has `eval_id`, matching the checkpoint record written in the same `record()` call.
- A research row always has `invocation_id`, matching the invocation record written by the same `RecordGuard`.
- All of a run's research rows are written in one transaction.

## 2. Types (`src/traits/classifier.rs`)

```rust
pub enum JevSite { TurnReview, ResearchClaims }          // ALL, as_str
pub enum JevMode { Off, Shadow }                         // ALL, as_str; `On` added in 065/066

pub struct JevQuestion { id: String, version: u32, kind: QuestionKind,
                         instructions: String, options: Vec<(String, Option<String>)> }
pub enum QuestionKind { Noul, Choice }
pub struct JevRequest { model: String, state: RedactedState, questions: Vec<JevQuestion> }
pub enum JevAnswer { Noul { p: f64 }, Choice { choice: String, probabilities: Vec<(String, f64)> } }
pub struct JevResponse { model_answered: String, answers: Vec<(String, JevAnswer)>,
                         input_tokens: Option<u64>, request_id: Option<String>, latency_ms: u64 }

// Async style and mock attribute follow the existing seams in src/traits/.
pub trait Classifier: Send + Sync {
    async fn classify(&self, req: &JevRequest) -> Result<JevResponse, AppError>;
    fn model_id(&self) -> &str;
}
```

- `RedactedState` is defined in `src/jev/redact.rs` and has no public constructor other than `redact(&str or serde_json::Value)`.
- Options are kept in the order declared (`Vec`, not a map).

## 3. `Candidate` origin (`src/checkpoint/review.rs`)

`Candidate` gains `origin: CandidateOrigin { Transcript, Memory }`. Memory pairs, built at `review.rs:192-203`, are `Memory`. Only `Transcript` pairs are put in the Jev state.

## 4. `review_once` return (`src/checkpoint/review.rs:348`)

`review_once` also returns `ReviewRaw { contradicts: bool, violates: bool, capture_kind: CaptureKind }` alongside its existing flags and proposal. `truncate()` (`:448`) returns `(String, bool)`, the bool meaning "was truncated".

## 5. Labels (`label_json`)

Enumerated fields only, never text:

- **Turn review:**
  - `{ contradicts: bool | null, violates: bool, capture_kind: "skill"|"lesson"|"none" | null }`
  - `contradicts` is null when the turn had any memory-derived candidate.
  - `capture_kind` is null when memory isn't configured.
- **Research claim:**
  - `{ support: "confirmed"|"unverified"|"contested"|"refuted", votes_supported: u32, votes_refuted: u32, votes_completed: u32, sources: u32 }`

## 6. Record additions

- `jev_cost_usd: Option<f64>` on `CheckpointRecord` (written by `record()`) and on `InvocationRecord` (summed from the accumulator).
- It is not added to `usage_by_model` and not routed through `NON_MODEL_USAGE`.

## 7. Research accumulator

```rust
pub struct JevShadow { rows: Mutex<Vec<PendingJevRow>>, excluded: AtomicU32 }
```

- The `JevShadow` is held in `RecordDims` beside `SearchCost`, `NonModelUsage` and `PageCounts`.
- The pipeline pushes `PendingJevRow { claim_key, source_ids, questions, response or error, label }`.
- `RecordGuard::finish` and `Drop` write all rows with the invocation id created in `RecordGuard::new`.

## 8. Outcome

`Outcome::ClassifierProvider` (`"classifier_provider"`) is added to `Outcome::ALL` (`src/error.rs`). The mapping table is in research D4.

## 9. Validation rules (`src/jev/policy.rs`)

An answer is valid only if all of these hold:

- each probability is finite and in [0,1];
- a choice is one of the declared options;
- every declared option has a probability;
- the probabilities sum to 1 within 0.01;
- answer ids equal the request's question ids, with no missing, extra or duplicate ids;
- `model_answered` equals the pinned model.

Invalid answers become `validation_failure` and are treated as missing. Band functions take `Option<f64>`, and a missing value never lands in an acting band in either direction.
