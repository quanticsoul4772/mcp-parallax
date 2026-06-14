//! The Source-Grounded Verify corrective (008).
//!
//! `verify` with machine-assembled evidence: the caller names source locators,
//! the server reads the verbatim text ([`crate::grounded::assemble`]), and the
//! same stance-blind ensemble as `verify` judges that evidence. The caller
//! cannot paraphrase or smuggle a conclusion into the evidence — it is pulled
//! from disk, never authored by the model. Aggregation is shared with `verify`
//! ([`crate::modes::verify::aggregate_core`]); only the assembly stage, the
//! `missing_evidence` field, and the manifest are new.

use crate::error::AppError;
use crate::grounded::assemble::{assemble, RawUnit};
use crate::grounded::{AssemblyLimits, ManifestEntry, SourceLocator};
use crate::modes::grounded_compute::{agreed_spec, settle};
use crate::modes::verify::{aggregate_core, VerdictKind};
use crate::modes::{CorrectiveMode, ModeRegistry};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use crate::traits::source::SourceReader;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Injected dependencies for the grounded-verify tool — present only when
/// `GROUNDED_VERIFY_ROOT` is configured (008 FR-001).
pub struct GroundedDeps {
    /// The model client for the stance-blind passes.
    pub model_client: Arc<dyn ModelClient>,
    /// The root-confined source reader.
    pub reader: Arc<dyn SourceReader>,
    /// The registered grounded-verify mode.
    pub mode: CorrectiveMode,
    /// Byte/locator ceilings.
    pub limits: AssemblyLimits,
    /// Claim length bound (`INPUT_MAX_CHARS`).
    pub max_claim_chars: usize,
}

impl GroundedDeps {
    /// Run grounded-verify with these dependencies.
    ///
    /// # Errors
    ///
    /// See [`run`].
    pub async fn evaluate(&self, params: &GroundedVerifyParams) -> Result<GroundedRun, AppError> {
        run(
            self.model_client.as_ref(),
            self.reader.as_ref(),
            &self.mode,
            params,
            self.limits,
            self.max_claim_chars,
        )
        .await
    }
}

/// Tool id as exposed over MCP.
pub const GROUNDED_VERIFY_ID: &str = "grounded_verify";

/// The MCP tool description — the routing text.
pub const GROUNDED_VERIFY_DESCRIPTION: &str = "Verify a claim against verbatim source you name. \
    You give a claim and a set of source locators (file paths or file/line ranges within the \
    configured root); the server reads that exact text and runs independent stance-blind passes \
    over it - you cannot paraphrase or bias the evidence. Returns a verdict (supported/refuted), \
    findings citing the source, a confidence from cross-pass agreement, an evidence manifest of \
    exactly what was read, and a completeness signal naming any evidence you did not provide. Use \
    when a claim must be checked against source you should not be trusted to summarize.";

/// The verifier prompt. Placeholders exist for the claim and the assembled
/// evidence ONLY — no slot for caller stance or conversation (blindness is
/// structural). `missing_evidence` surfaces omissions rather than refusing.
const PROMPT_TEMPLATE: &str = "You are an independent verifier. Judge the claim below strictly \
    against the SOURCE EVIDENCE provided - verbatim excerpts from named files. You know nothing \
    about who made the claim or how confident they are.\n\
    \n\
    Rules:\n\
    1. Judge ONLY against the evidence shown. If the evidence supports the claim under its \
    strongest reading, support it; if it contradicts the claim, refute it.\n\
    2. Every finding in a refutation must name the specific contradiction and point to the file \
    (and lines) in the evidence. Vague doubt is not a finding.\n\
    3. In `missing_evidence`, name any source you would need to judge the claim fully that is NOT \
    present in the evidence (for example, \"the definition of the function under test\"). Leave it \
    empty when the evidence is sufficient. Do not refuse for missing evidence - judge on what is \
    present and list what is missing.\n\
    4. Set `needs_computation` to true ONLY when the claim's truth hinges on an exact computation \
    over the source that you cannot perform reliably by reading - a precise line count, a count of \
    matches, a byte/size measure, or a numeric comparison that depends on such a count. In that \
    case a deterministic engine, not a reader's estimate, must decide it. Set it false for ordinary \
    judgment claims about what the source says or does.\n\
    5. When (and only when) you set `needs_computation`, also describe the computation so the engine \
    can run it: set `compute_property` to one of \"lines\", \"bytes\", or \"matches\"; \
    `compute_match_literal` to the exact string to count (only for \"matches\", else null); \
    `compute_operator` to one of \">\", \">=\", \"<\", \"<=\", \"==\", \"!=\"; and \
    `compute_threshold` to the integer the claim compares against. If the claim's computable part \
    is none of line/byte/match counts, leave these null - the server will route it onward. Never \
    state the counted value or the final verdict yourself; only name what to count and the bound.\n\
    6. When you set `needs_computation`, set `verdict` to \"supported\" and leave `findings` empty - \
    your verdict is not used for a computed claim; the engine decides. Do NOT add a refutation \
    finding for a computable claim.\n\
    7. Findings and missing_evidence entries are self-contained single sentences.\n\
    \n\
    Claim to verify:\n<<claim>>\n\
    \n\
    Source evidence (verbatim):\n<<evidence>>\n";

/// Tool input: the claim plus the locators to read.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct GroundedVerifyParams {
    /// The claim to verify, stated neutrally — the only caller prose the passes
    /// ever see.
    pub claim: String,
    /// Source locators to read verbatim as the evidence (non-empty).
    pub locators: Vec<SourceLocator>,
}

/// What each pass is grammar-constrained to produce.
///
/// Verify's `PassVerdict` plus `missing_evidence` and `needs_computation`. Flat
/// and closed (Constitution II): all four properties are scalars or arrays of
/// scalars.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GroundedPass {
    /// supported | refuted.
    pub verdict: VerdictKind,
    /// Specific findings; non-empty when refuting (validator-enforced).
    pub findings: Vec<String>,
    /// Source classes the pass would need but was not given; empty when the
    /// evidence suffices. Advisory completeness signal (008) — it does **not**
    /// trigger abstention.
    pub missing_evidence: Vec<String>,
    /// Set when the claim's truth is a computable property of the source the
    /// pass cannot settle by reading (a precise count/measure/comparison). The
    /// **only** abstain trigger (010, FR-005/FR-006): a majority routes the
    /// claim to the deterministic `check` layer instead of judging it.
    pub needs_computation: bool,
    /// What to count (011): `"lines"` | `"bytes"` | `"matches"`, else null.
    /// A **nullable string**, not an enum — `Option<enum>` would emit `anyOf`,
    /// which the flat-schema gate rejects; the closed set is server-validated.
    pub compute_property: Option<String>,
    /// The literal to count, for `compute_property == "matches"` (011); null otherwise.
    pub compute_match_literal: Option<String>,
    /// The comparison the claim asserts (011): `">"` | `">="` | `"<"` | `"<="`
    /// | `"=="` | `"!="`, else null. Nullable string, server-validated (see above).
    pub compute_operator: Option<String>,
    /// The numeric bound the claim compares against (011); null otherwise.
    pub compute_threshold: Option<i64>,
}

/// The server-assembled output verdict (010).
///
/// Distinct from the per-pass [`VerdictKind`] (shared with `verify`): a
/// non-decision is a first-class outcome here, and keeping `Inconclusive` out of
/// the shared per-pass enum is what lets `verify` stay unchanged (FR-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(inline)]
pub enum GroundedVerdictKind {
    /// The evidence supports the claim under its strongest reading.
    Supported,
    /// The evidence contradicts the claim with a named concrete error.
    Refuted,
    /// The passes self-report (`needs_computation`) that the decisive fact is a
    /// computation they cannot perform — route to `check`.
    Inconclusive,
}

impl From<VerdictKind> for GroundedVerdictKind {
    fn from(v: VerdictKind) -> Self {
        match v {
            VerdictKind::Supported => Self::Supported,
            VerdictKind::Refuted => Self::Refuted,
        }
    }
}

/// The aggregated tool output (data-model.md). `confidence` and `manifest` are
/// server-assembled — never model self-report (FR-012).
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct GroundedVerdict {
    /// Majority verdict across passes; ties resolve to refuted; or
    /// `inconclusive` when a majority of passes set `needs_computation`.
    pub verdict: GroundedVerdictKind,
    /// Deduplicated findings from the majority-side passes.
    pub findings: Vec<String>,
    /// Agreement ratio (majority count / passes completed).
    pub confidence: f64,
    /// Number of verification passes that completed.
    pub passes: u32,
    /// Union of the passes' `missing_evidence` — empty when nothing material
    /// was omitted. Advisory only; never forces `inconclusive`.
    pub missing_evidence: Vec<String>,
    /// The audit manifest of exactly what was read.
    pub manifest: Vec<ManifestEntry>,
    /// Why the verdict is `inconclusive` (route-to-`check`); absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The comparison the engine decided (e.g. `1224 > 1000`), present only on a
    /// settled compute verdict (011); absent on judgment and abstain paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executed_form: Option<String>,
    /// The engine's raw result text for a settled compute verdict (011); absent
    /// otherwise. Mirrors `check`'s auditable output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_result: Option<String>,
}

/// One grounded-verify run: the verdict plus summed usage for the record.
#[derive(Debug)]
pub struct GroundedRun {
    /// The aggregated verdict, manifest, and completeness signal.
    pub verdict: GroundedVerdict,
    /// Input tokens summed across completed passes.
    pub input_tokens: u64,
    /// Output tokens summed across completed passes.
    pub output_tokens: u64,
}

/// Register the grounded-verify mode (boot-time; enforces flat+closed).
///
/// # Errors
///
/// Propagates the registry's schema-invariant failure.
pub fn register(registry: &mut ModeRegistry, ensemble_k: u8) -> Result<(), AppError> {
    let schema = serde_json::to_value(schemars::schema_for!(GroundedPass))
        .map_err(|e| AppError::ValidationFailure(format!("schema serialization: {e}")))?;
    registry.register(
        GROUNDED_VERIFY_ID,
        GROUNDED_VERIFY_DESCRIPTION,
        PROMPT_TEMPLATE,
        schema,
        ensemble_k,
    )
}

/// Build the per-pass prompt. Claim and assembled evidence are the only dynamic
/// content.
fn build_prompt(template: &str, claim: &str, evidence: &str) -> String {
    template
        .replace("<<claim>>", claim)
        .replace("<<evidence>>", evidence)
}

/// Reject an empty/oversize claim before any read or model call.
fn check_claim(claim: &str, max_claim_chars: usize) -> Result<(), AppError> {
    if claim.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "claim is empty or whitespace-only".to_string(),
        ));
    }
    let len = claim.chars().count();
    if len > max_claim_chars {
        return Err(AppError::InvalidInput(format!(
            "claim is {len} characters; the configured maximum is {max_claim_chars} \
             (INPUT_MAX_CHARS); it was not trimmed"
        )));
    }
    Ok(())
}

/// Run one grounded-verify invocation: validate, assemble evidence
/// all-or-nothing, k stance-blind passes, aggregate.
///
/// # Errors
///
/// `InvalidInput` for a bad claim or any unresolvable locator (before any model
/// call); otherwise the dominant failure class when fewer than quorum passes
/// complete.
pub async fn run(
    client: &dyn ModelClient,
    reader: &dyn SourceReader,
    mode: &CorrectiveMode,
    params: &GroundedVerifyParams,
    limits: AssemblyLimits,
    max_claim_chars: usize,
) -> Result<GroundedRun, AppError> {
    check_claim(&params.claim, max_claim_chars)?;
    let assembled = assemble(reader, &params.locators, limits)?;
    let prompt = build_prompt(mode.prompt_template, &params.claim, &assembled.text);

    let passes =
        futures::future::join_all((0..mode.ensemble_k).map(|_| one_pass(client, mode, &prompt)))
            .await;

    aggregate(
        passes,
        mode.ensemble_k,
        assembled.manifest,
        &assembled.units,
    )
}

/// One blind pass: constrained completion → local validation → typed pass.
async fn one_pass(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    prompt: &str,
) -> Result<(GroundedPass, u64, u64), AppError> {
    let completion = client.complete(prompt, &mode.sanitized_schema).await?;
    validate(&mode.output_schema, &completion.value)?;
    let pass: GroundedPass = serde_json::from_value(completion.value)
        .map_err(|e| AppError::ValidationFailure(format!("grounded verdict shape: {e}")))?;
    // The same calibrated rule as verify: a refutation with no named finding is
    // not evidence.
    if pass.verdict == VerdictKind::Refuted && pass.findings.iter().all(|f| f.trim().is_empty()) {
        return Err(AppError::ValidationFailure(
            "refutation without a named concrete error".to_string(),
        ));
    }
    Ok((pass, completion.input_tokens, completion.output_tokens))
}

/// Aggregate: union the completeness signals, count the abstain self-reports,
/// then either **settle** a computable claim (011) or share verify's verdict
/// math. `needs_computation` (a majority of completed passes) routes to compute:
/// an agreed in-class single-source purely-computable spec is counted and
/// settled by the engine; otherwise it abstains with `inconclusive` (010). A
/// non-empty `missing_evidence` is carried through as advisory and never abstains.
fn aggregate(
    passes: Vec<Result<(GroundedPass, u64, u64), AppError>>,
    k: u8,
    manifest: Vec<ManifestEntry>,
    units: &[RawUnit],
) -> Result<GroundedRun, AppError> {
    // Completeness is the union/dedup of every completed pass's missing list;
    // tally the abstain self-reports and collect the `needs_computation` passes
    // (for the compute spec) over the same completed passes.
    let mut missing_evidence: Vec<String> = Vec::new();
    let mut completed = 0_usize;
    let mut needs_computation = 0_usize;
    let mut nc_passes: Vec<&GroundedPass> = Vec::new();
    // A computable pass should carry no judgment finding (analyze M1/M2); if one
    // does, the claim is compound — settle nothing, abstain (M2).
    let mut compound = false;
    for (pass, _, _) in passes.iter().filter_map(|p| p.as_ref().ok()) {
        completed += 1;
        if pass.needs_computation {
            needs_computation += 1;
            nc_passes.push(pass);
            if pass.findings.iter().any(|f| !f.trim().is_empty()) {
                compound = true;
            }
        }
        for item in &pass.missing_evidence {
            if !item.trim().is_empty() && !missing_evidence.contains(item) {
                missing_evidence.push(item.clone());
            }
        }
    }

    let nc_majority = needs_computation * 2 > completed;
    // Settle only an agreed, in-class, single-source, purely-computable claim
    // (T006). The else-branch (no agreed spec / multi-source / compound / engine
    // error) falls through to the 010 abstain below (T008). Computed before
    // `passes` is consumed into `core` (the `nc_passes` borrows end here).
    let settled = if nc_majority && !compound && units.len() == 1 {
        agreed_spec(&nc_passes).and_then(|spec| settle(&spec, &units[0]))
    } else {
        None
    };

    let core = passes
        .into_iter()
        .map(|pass| pass.map(|(p, inp, out)| (p.verdict, p.findings, inp, out)))
        .collect();
    // `aggregate_core` enforces quorum; `completed` here equals the passes it
    // counts (both tally the same Ok set).
    let run = aggregate_core(core, k)?;

    // Three exits: a settled compute verdict (011), the 010 abstain on a
    // `needs_computation` majority that did not settle, or the judgment verdict.
    let verdict = if let Some(settled) = settled {
        GroundedVerdict {
            verdict: settled.verdict,
            findings: vec![settled.note],
            confidence: 1.0, // a deterministic settle, not an agreement ratio
            passes: run.verdict.passes,
            missing_evidence,
            manifest,
            reason: None,
            executed_form: Some(settled.executed_form),
            engine_result: Some(settled.engine_result),
        }
    } else if nc_majority {
        GroundedVerdict {
            verdict: GroundedVerdictKind::Inconclusive,
            findings: run.verdict.findings,
            confidence: run.verdict.confidence,
            passes: run.verdict.passes,
            missing_evidence,
            manifest,
            reason: Some("computable property — route to `check`".to_string()),
            executed_form: None,
            engine_result: None,
        }
    } else {
        GroundedVerdict {
            verdict: GroundedVerdictKind::from(run.verdict.verdict),
            findings: run.verdict.findings,
            confidence: run.verdict.confidence,
            passes: run.verdict.passes,
            missing_evidence,
            manifest,
            reason: None,
            executed_form: None,
            engine_result: None,
        }
    };

    Ok(GroundedRun {
        verdict,
        input_tokens: run.input_tokens,
        output_tokens: run.output_tokens,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::traits::client::{Completion, MockModelClient};
    use crate::traits::source::{MockSourceReader, SourceContent};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn test_mode(k: u8) -> CorrectiveMode {
        let mut registry = ModeRegistry::new();
        register(&mut registry, k).unwrap();
        registry.get(GROUNDED_VERIFY_ID).unwrap().clone()
    }

    fn limits() -> AssemblyLimits {
        AssemblyLimits {
            max_bytes: 262_144,
            max_locators: 64,
        }
    }

    fn params(claim: &str, paths: &[&str]) -> GroundedVerifyParams {
        GroundedVerifyParams {
            claim: claim.to_string(),
            locators: paths
                .iter()
                .map(|p| SourceLocator {
                    path: Some((*p).to_string()),
                    glob: None,
                    start_line: None,
                    end_line: None,
                })
                .collect(),
        }
    }

    fn ok_reader() -> MockSourceReader {
        let mut mock = MockSourceReader::new();
        mock.expect_read().returning(|path, _, _| {
            let text = format!("contents of {path}");
            let bytes = text.len() as u64;
            Ok(SourceContent { text, bytes })
        });
        mock
    }

    fn scripted_client(results: Vec<Value>) -> MockModelClient {
        let cursor = Arc::new(AtomicUsize::new(0));
        let mut mock = MockModelClient::new();
        mock.expect_complete().returning(move |_, _| {
            let i = cursor.fetch_add(1, Ordering::SeqCst);
            Ok(Completion {
                value: results[i % results.len()].clone(),
                input_tokens: 100,
                output_tokens: 10,
            })
        });
        mock
    }

    /// A canned pass body with the abstain flag explicit.
    #[allow(clippy::needless_pass_by_value)]
    fn gpass(verdict: &str, findings: Value, missing: Value, needs_computation: bool) -> Value {
        json!({
            "verdict": verdict,
            "findings": findings,
            "missing_evidence": missing,
            "needs_computation": needs_computation,
        })
    }

    #[test]
    fn pass_schema_registers_flat_and_closed_with_missing_evidence_and_needs_computation() {
        let mode = test_mode(3);
        assert_eq!(mode.sanitized_schema["additionalProperties"], json!(false));
        assert_eq!(
            mode.sanitized_schema["properties"]["verdict"]["enum"],
            json!(["supported", "refuted"])
        );
        assert!(mode.sanitized_schema["properties"]["missing_evidence"].is_object());
        // The new abstain flag is a flat boolean and stays in the closed schema.
        assert_eq!(
            mode.sanitized_schema["properties"]["needs_computation"]["type"],
            json!("boolean")
        );
        assert_eq!(mode.sanitized_schema["additionalProperties"], json!(false));
    }

    #[test]
    fn prompt_has_only_claim_and_evidence_slots() {
        assert_eq!(PROMPT_TEMPLATE.matches("<<").count(), 2);
        assert!(PROMPT_TEMPLATE.contains("<<claim>>") && PROMPT_TEMPLATE.contains("<<evidence>>"));
    }

    #[tokio::test]
    async fn empty_claim_is_rejected_before_any_read_or_model_call() {
        let mode = test_mode(3);
        let mut reader = MockSourceReader::new();
        reader.expect_read().times(0);
        let mut client = MockModelClient::new();
        client.expect_complete().times(0);

        let err = run(
            &client,
            &reader,
            &mode,
            &params("  ", &["a.rs"]),
            limits(),
            50_000,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn an_unresolvable_locator_aborts_before_any_model_call() {
        let mode = test_mode(3);
        let mut reader = MockSourceReader::new();
        reader.expect_read().returning(|path, _, _| {
            Err(AppError::InvalidInput(format!("source not found: {path}")))
        });
        let mut client = MockModelClient::new();
        client.expect_complete().times(0);

        let err = run(
            &client,
            &reader,
            &mode,
            &params("c", &["gone.rs"]),
            limits(),
            50_000,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("source not found: gone.rs"));
    }

    #[tokio::test]
    async fn verdict_manifest_and_completeness_are_assembled() {
        let mode = test_mode(3);
        let reader = ok_reader();
        let client = scripted_client(vec![
            gpass(
                "supported",
                json!([]),
                json!(["the caller's config"]),
                false,
            ),
            gpass("supported", json!([]), json!([]), false),
            gpass(
                "supported",
                json!([]),
                json!(["the caller's config"]),
                false,
            ),
        ]);

        let out = run(
            &client,
            &reader,
            &mode,
            &params("c", &["a.rs", "b.rs"]),
            limits(),
            50_000,
        )
        .await
        .unwrap();
        assert_eq!(out.verdict.verdict, GroundedVerdictKind::Supported);
        assert!((out.verdict.confidence - 1.0).abs() < f64::EPSILON);
        assert!(out.verdict.reason.is_none());
        // Manifest mirrors the two locators.
        assert_eq!(out.verdict.manifest.len(), 2);
        assert_eq!(out.verdict.manifest[0].path, "a.rs");
        // Completeness is the dedup union across passes.
        assert_eq!(
            out.verdict.missing_evidence,
            vec!["the caller's config".to_string()]
        );
        assert_eq!(out.input_tokens, 300);
    }

    // ---- T008 (010): the abstain trigger and no-over-abstention ------------

    #[tokio::test]
    async fn majority_needs_computation_returns_inconclusive_routed_to_check() {
        let mode = test_mode(3);
        let reader = ok_reader();
        // 2 of 3 passes self-report the decisive fact is a computation.
        let client = scripted_client(vec![
            gpass("refuted", json!(["estimated ~850 lines"]), json!([]), true),
            gpass("refuted", json!(["estimated ~850 lines"]), json!([]), true),
            gpass("supported", json!([]), json!([]), false),
        ]);

        let out = run(
            &client,
            &reader,
            &mode,
            &params("c", &["a.rs"]),
            limits(),
            50_000,
        )
        .await
        .unwrap()
        .verdict;
        assert_eq!(out.verdict, GroundedVerdictKind::Inconclusive);
        let reason = out.reason.expect("inconclusive carries a reason");
        assert!(reason.contains("check"), "reason routes to check: {reason}");
    }

    #[tokio::test]
    async fn advisory_missing_evidence_alone_does_not_force_inconclusive() {
        // No over-abstention: a confident verdict that merely lists non-decisive
        // missing evidence (no pass set needs_computation) stays supported.
        let mode = test_mode(3);
        let reader = ok_reader();
        let client = scripted_client(vec![
            gpass(
                "supported",
                json!([]),
                json!(["the caller's config"]),
                false,
            ),
            gpass(
                "supported",
                json!([]),
                json!(["a downstream helper"]),
                false,
            ),
            gpass("supported", json!([]), json!([]), false),
        ]);

        let out = run(
            &client,
            &reader,
            &mode,
            &params("c", &["a.rs"]),
            limits(),
            50_000,
        )
        .await
        .unwrap()
        .verdict;
        assert_eq!(out.verdict, GroundedVerdictKind::Supported);
        assert!(out.reason.is_none());
        // The advisory signal is still surfaced, just not as abstention.
        assert!(!out.missing_evidence.is_empty());
    }

    #[tokio::test]
    async fn a_single_needs_computation_among_three_is_not_a_majority() {
        // 1 of 3 is below the strict majority — the verdict stands.
        let mode = test_mode(3);
        let reader = ok_reader();
        let client = scripted_client(vec![
            gpass("refuted", json!(["the named error"]), json!([]), true),
            gpass("refuted", json!(["the named error"]), json!([]), false),
            gpass("refuted", json!(["the named error"]), json!([]), false),
        ]);

        let out = run(
            &client,
            &reader,
            &mode,
            &params("c", &["a.rs"]),
            limits(),
            50_000,
        )
        .await
        .unwrap()
        .verdict;
        assert_eq!(out.verdict, GroundedVerdictKind::Refuted);
        assert!(out.reason.is_none());
    }

    #[test]
    fn grounded_verdict_kind_maps_from_pass_verdict() {
        assert_eq!(
            GroundedVerdictKind::from(VerdictKind::Supported),
            GroundedVerdictKind::Supported
        );
        assert_eq!(
            GroundedVerdictKind::from(VerdictKind::Refuted),
            GroundedVerdictKind::Refuted
        );
    }

    // ---- 011: compute-settle helpers ---------------------------------------

    /// A computable pass: needs_computation set, verdict supported + empty
    /// findings (M1), and the compute fields naming the property/operator/bound.
    fn gcompute(property: &str, op: &str, threshold: i64, literal: Option<&str>) -> Value {
        json!({
            "verdict": "supported",
            "findings": [],
            "missing_evidence": [],
            "needs_computation": true,
            "compute_property": property,
            "compute_match_literal": literal,
            "compute_operator": op,
            "compute_threshold": threshold,
        })
    }

    fn reader_returning(text: String) -> MockSourceReader {
        let mut mock = MockSourceReader::new();
        mock.expect_read().returning(move |_, _, _| {
            Ok(SourceContent {
                text: text.clone(),
                #[allow(clippy::cast_possible_truncation)]
                bytes: text.len() as u64,
            })
        });
        mock
    }

    // The pure counting / spec-validation tests live with the unit in
    // `grounded_compute`; the tests below exercise the wired path through `run`.

    // ---- T007: settle the in-class single-source claim ---------------------

    #[tokio::test]
    async fn computable_line_count_over_1000_settles_supported() {
        let mode = test_mode(3);
        let reader = reader_returning("x\n".repeat(1224)); // 1224 lines
        let client = scripted_client(vec![
            gcompute("lines", ">", 1000, None),
            gcompute("lines", ">", 1000, None),
            gpass("supported", json!([]), json!([]), false),
        ]);

        let out = run(
            &client,
            &reader,
            &mode,
            &params("c", &["server.rs"]),
            limits(),
            50_000,
        )
        .await
        .unwrap()
        .verdict;
        assert_eq!(out.verdict, GroundedVerdictKind::Supported);
        assert_eq!(out.executed_form.as_deref(), Some("1224 > 1000"));
        assert_eq!(out.engine_result.as_deref(), Some("true"));
        assert_eq!(out.findings, vec!["counted 1224 lines".to_string()]);
        assert!((out.confidence - 1.0).abs() < f64::EPSILON);
        assert!(out.reason.is_none());
    }

    #[tokio::test]
    async fn computable_line_count_over_5000_settles_refuted() {
        let mode = test_mode(3);
        let reader = reader_returning("x\n".repeat(1224));
        let client = scripted_client(vec![
            gcompute("lines", ">", 5000, None),
            gcompute("lines", ">", 5000, None),
            gcompute("lines", ">", 5000, None),
        ]);

        let out = run(
            &client,
            &reader,
            &mode,
            &params("c", &["server.rs"]),
            limits(),
            50_000,
        )
        .await
        .unwrap()
        .verdict;
        assert_eq!(out.verdict, GroundedVerdictKind::Refuted);
        assert_eq!(out.executed_form.as_deref(), Some("1224 > 5000"));
        assert_eq!(out.engine_result.as_deref(), Some("false"));
    }

    #[tokio::test]
    async fn computable_byte_and_match_specs_settle() {
        let mode = test_mode(1);
        // bytes: "abcde" = 5 bytes, > 3 → supported.
        let out = run(
            &scripted_client(vec![gcompute("bytes", ">", 3, None)]),
            &reader_returning("abcde".to_string()),
            &mode,
            &params("c", &["f"]),
            limits(),
            50_000,
        )
        .await
        .unwrap()
        .verdict;
        assert_eq!(out.verdict, GroundedVerdictKind::Supported);
        assert_eq!(out.executed_form.as_deref(), Some("5 > 3"));

        // matches: "ababab" has 3 "ab", == 3 → supported.
        let out = run(
            &scripted_client(vec![gcompute("matches", "==", 3, Some("ab"))]),
            &reader_returning("ababab".to_string()),
            &mode,
            &params("c", &["f"]),
            limits(),
            50_000,
        )
        .await
        .unwrap()
        .verdict;
        assert_eq!(out.verdict, GroundedVerdictKind::Supported);
        assert_eq!(out.executed_form.as_deref(), Some("3 == 3"));
    }

    #[tokio::test]
    async fn a_lone_computable_pass_is_accepted_not_dropped() {
        // M1: verdict=supported + empty findings + compute fields passes one_pass.
        let mode = test_mode(1);
        let out = run(
            &scripted_client(vec![gcompute("lines", ">", 0, None)]),
            &reader_returning("a\nb\n".to_string()),
            &mode,
            &params("c", &["f"]),
            limits(),
            50_000,
        )
        .await
        .unwrap()
        .verdict;
        assert_eq!(out.verdict, GroundedVerdictKind::Supported);
        assert_eq!(out.executed_form.as_deref(), Some("2 > 0"));
    }

    // ---- T009: the abstain fallbacks (no over-reach) -----------------------

    #[tokio::test]
    async fn disagreeing_compute_specs_abstain() {
        let mode = test_mode(3);
        let client = scripted_client(vec![
            gcompute("lines", ">", 1000, None),
            gcompute("lines", ">", 2000, None),
            gcompute("bytes", "<", 50, None),
        ]);
        let out = run(
            &client,
            &reader_returning("x\n".repeat(1224)),
            &mode,
            &params("c", &["f"]),
            limits(),
            50_000,
        )
        .await
        .unwrap()
        .verdict;
        assert_eq!(out.verdict, GroundedVerdictKind::Inconclusive);
        assert!(out.executed_form.is_none());
    }

    #[tokio::test]
    async fn out_of_class_property_abstains() {
        let mode = test_mode(3);
        let client = scripted_client(vec![
            gcompute("functions", ">", 5, None),
            gcompute("functions", ">", 5, None),
            gpass("supported", json!([]), json!([]), false),
        ]);
        let out = run(
            &client,
            &reader_returning("x\n".repeat(10)),
            &mode,
            &params("c", &["f"]),
            limits(),
            50_000,
        )
        .await
        .unwrap()
        .verdict;
        assert_eq!(out.verdict, GroundedVerdictKind::Inconclusive);
        assert!(out.executed_form.is_none());
    }

    #[tokio::test]
    async fn multi_source_computable_abstains() {
        let mode = test_mode(3);
        let client = scripted_client(vec![
            gcompute("lines", ">", 1000, None),
            gcompute("lines", ">", 1000, None),
            gcompute("lines", ">", 1000, None),
        ]);
        // Two locators → two read units → not single-source → abstain.
        let out = run(
            &client,
            &reader_returning("x\n".repeat(1224)),
            &mode,
            &params("c", &["a", "b"]),
            limits(),
            50_000,
        )
        .await
        .unwrap()
        .verdict;
        assert_eq!(out.verdict, GroundedVerdictKind::Inconclusive);
        assert!(out.executed_form.is_none());
    }

    #[tokio::test]
    async fn compound_claim_with_a_judgment_finding_abstains() {
        // M2: a computable spec but with a substantive judgment finding → compound.
        let mode = test_mode(3);
        let mut compound = gcompute("lines", ">", 1000, None);
        compound["findings"] = json!(["the module is also poorly structured"]);
        // findings non-empty + refuted-with-finding is fine; set verdict refuted so
        // one_pass accepts the finding, and needs_computation marks it computable.
        compound["verdict"] = json!("refuted");
        let client = scripted_client(vec![
            compound.clone(),
            compound,
            gpass("supported", json!([]), json!([]), false),
        ]);
        let out = run(
            &client,
            &reader_returning("x\n".repeat(1224)),
            &mode,
            &params("c", &["f"]),
            limits(),
            50_000,
        )
        .await
        .unwrap()
        .verdict;
        assert_eq!(out.verdict, GroundedVerdictKind::Inconclusive);
        assert!(out.executed_form.is_none());
    }

    #[tokio::test]
    async fn non_computable_judgment_path_has_no_executed_form() {
        let mode = test_mode(3);
        let client = scripted_client(vec![
            gpass("supported", json!([]), json!([]), false),
            gpass("supported", json!([]), json!([]), false),
            gpass("supported", json!([]), json!([]), false),
        ]);
        let out = run(
            &client,
            &reader_returning("x".to_string()),
            &mode,
            &params("c", &["f"]),
            limits(),
            50_000,
        )
        .await
        .unwrap()
        .verdict;
        assert_eq!(out.verdict, GroundedVerdictKind::Supported);
        assert!(out.executed_form.is_none());
        assert!(out.engine_result.is_none());
    }
}
