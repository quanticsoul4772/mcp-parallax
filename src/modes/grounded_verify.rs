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
use crate::grounded::assemble::assemble;
use crate::grounded::{AssemblyLimits, ManifestEntry, SourceLocator};
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
    4. Findings and missing_evidence entries are self-contained single sentences.\n\
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

/// What each pass is grammar-constrained to produce — verify's `PassVerdict`
/// plus `missing_evidence`. Flat + closed (Constitution II).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GroundedPass {
    /// supported | refuted.
    pub verdict: VerdictKind,
    /// Specific findings; non-empty when refuting (validator-enforced).
    pub findings: Vec<String>,
    /// Source classes the pass would need but was not given; empty when the
    /// evidence suffices.
    pub missing_evidence: Vec<String>,
}

/// The aggregated tool output (data-model.md). `confidence` and `manifest` are
/// server-assembled — never model self-report (FR-012).
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct GroundedVerdict {
    /// Majority verdict across passes; ties resolve to refuted.
    pub verdict: VerdictKind,
    /// Deduplicated findings from the majority-side passes.
    pub findings: Vec<String>,
    /// Agreement ratio (majority count / passes completed).
    pub confidence: f64,
    /// Number of verification passes that completed.
    pub passes: u32,
    /// Union of the passes' `missing_evidence` — empty when nothing material
    /// was omitted.
    pub missing_evidence: Vec<String>,
    /// The audit manifest of exactly what was read.
    pub manifest: Vec<ManifestEntry>,
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

    aggregate(passes, mode.ensemble_k, assembled.manifest)
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

/// Aggregate: union the completeness signals, then share verify's verdict math.
fn aggregate(
    passes: Vec<Result<(GroundedPass, u64, u64), AppError>>,
    k: u8,
    manifest: Vec<ManifestEntry>,
) -> Result<GroundedRun, AppError> {
    // Completeness is the union/dedup of every completed pass's missing list.
    let mut missing_evidence: Vec<String> = Vec::new();
    for (pass, _, _) in passes.iter().filter_map(|p| p.as_ref().ok()) {
        for item in &pass.missing_evidence {
            if !item.trim().is_empty() && !missing_evidence.contains(item) {
                missing_evidence.push(item.clone());
            }
        }
    }

    let core = passes
        .into_iter()
        .map(|pass| pass.map(|(p, inp, out)| (p.verdict, p.findings, inp, out)))
        .collect();
    let run = aggregate_core(core, k)?;

    Ok(GroundedRun {
        verdict: GroundedVerdict {
            verdict: run.verdict.verdict,
            findings: run.verdict.findings,
            confidence: run.verdict.confidence,
            passes: run.verdict.passes,
            missing_evidence,
            manifest,
        },
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

    #[test]
    fn pass_schema_registers_flat_and_closed_with_missing_evidence() {
        let mode = test_mode(3);
        assert_eq!(mode.sanitized_schema["additionalProperties"], json!(false));
        assert_eq!(
            mode.sanitized_schema["properties"]["verdict"]["enum"],
            json!(["supported", "refuted"])
        );
        assert!(mode.sanitized_schema["properties"]["missing_evidence"].is_object());
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
            json!({ "verdict": "supported", "findings": [], "missing_evidence": ["the caller's config"] }),
            json!({ "verdict": "supported", "findings": [], "missing_evidence": [] }),
            json!({ "verdict": "supported", "findings": [], "missing_evidence": ["the caller's config"] }),
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
        assert_eq!(out.verdict.verdict, VerdictKind::Supported);
        assert!((out.verdict.confidence - 1.0).abs() < f64::EPSILON);
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
}
