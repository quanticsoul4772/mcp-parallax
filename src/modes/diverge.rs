//! The Diverge corrective (012) — independent perspectives for anchoring.
//!
//! When the caller is locked onto one framing of a problem, asking the model to
//! "reconsider" in the same context deepens the commitment. `diverge` runs `k`
//! stance-blind passes, each under a distinct **generative** lens (invert the
//! goal, change the actor, shift the horizon, deny the load-bearing assumption,
//! reframe the problem class), so the passes are pushed off the anchored frame in
//! different directions. The server labels each perspective with its lens and
//! returns the **deterministically deduplicated** set — the divergence
//! counterpart to `verify`'s convergence. It reuses `verify`'s ensemble
//! orchestration and lens pattern but **not** its verdict aggregation (there is
//! no verdict to converge on): it collects and dedups, it does not vote.

use crate::error::AppError;
use crate::modes::verify::dominant_failure;
use crate::modes::{CorrectiveMode, ModeRegistry};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Tool id as exposed over MCP.
pub const DIVERGE_ID: &str = "diverge";

/// The MCP tool description — the routing text (kept in sync with contracts/).
pub const DIVERGE_DESCRIPTION: &str = "Break out of a single framing of a problem. Runs \
    parallel stance-blind passes, each attacking the problem from a distinct angle (invert the \
    goal, change whose problem it is, shift the time horizon, deny the load-bearing assumption, \
    reframe the problem class), and returns a deduplicated set of genuinely different framings - \
    each a one-line reframing plus what it changes, labeled with the angle that produced it. Use \
    when you are anchored or tunnel-visioned and need real alternatives, not a more confident \
    version of the framing you already hold. To judge whether a claim is true use verify; to \
    commit to one next step use unstick.";

/// The divergence prompt. Placeholders exist for the lens (a fixed generative
/// perspective, not caller-supplied), the problem, and the context ONLY — no slot
/// for the caller's stance, preferred framing, or history (blindness is
/// structural). The `<<lens>>` slot carries one of [`LENSES`], assigned per pass.
const PROMPT_TEMPLATE: &str = "You are helping someone who is stuck on ONE framing of a problem \
    see it differently. You know nothing about who they are or which answer they prefer.\n\
    \n\
    Divergence lens for this pass — reframe the problem THROUGH it, do not just restate the \
    problem:\n<<lens>>\n\
    \n\
    Rules:\n\
    1. Produce ONE genuinely different framing of the problem under this lens — a reframing the \
    person locked on the obvious reading would not have reached on their own.\n\
    2. `framing`: one self-contained sentence stating the alternative framing.\n\
    3. `implication`: one self-contained sentence on what this framing changes or what it would \
    mean to act on it.\n\
    4. Do not judge whether anything is true and do not recommend a single action — only open up \
    a way of seeing the problem. If this lens genuinely yields nothing new, give the closest \
    honest reframing rather than a hollow restatement.\n\
    \n\
    Problem the person is anchored on:\n<<problem>>\n\
    \n\
    Context provided with the problem (may be empty):\n<<context>>\n";

/// A named generative perspective assigned to a `diverge` pass. The lens changes
/// the *direction* a pass reframes — never *what it knows about the asker* — so
/// stance-blindness is preserved (FR-005).
#[derive(Debug, Clone, Copy)]
struct Lens {
    /// Short identifier, surfaced as the perspective's label (FR-003).
    name: &'static str,
    /// The one-paragraph instruction injected at the `<<lens>>` slot.
    directive: &'static str,
}

/// The fixed divergence lens set (research D1). Pass `i` uses
/// `LENSES[i % LENSES.len()]` (research D2). These are *generative* angles (open
/// the space), the counterpart to `verify`'s *critical* lenses.
const LENSES: &[Lens] = &[
    Lens {
        name: "invert",
        directive: "Flip the goal. What if the opposite of the stated aim were the point — what \
            if the thing being treated as the problem is actually the solution, or the thing to \
            protect? Reframe from the inversion.",
    },
    Lens {
        name: "actor",
        directive: "Change whose problem this is. A different stakeholder, role, or user — the \
            person who maintains it, the newcomer, the one who pays, the adversary — sees this \
            situation as what, exactly? Reframe from their vantage.",
    },
    Lens {
        name: "horizon",
        directive: "Shift the time scale. What does this look like at ten times shorter or ten \
            times longer a horizon — in an hour versus a decade? Reframe around the scale where \
            the real stakes live.",
    },
    Lens {
        name: "assumption",
        directive: "Name the single load-bearing assumption the stated framing quietly rests on, \
            then deny it. If that assumption were false, what does the problem become? Reframe \
            from its negation.",
    },
    Lens {
        name: "class",
        directive: "Reframe the problem's category. What KIND of problem is this really — is it \
            actually a different class (a coordination problem dressed as a technical one, a \
            naming problem dressed as a design one)? Reframe to the truer class.",
    },
];

/// Tool input: the problem the caller is anchored on, plus optional neutral context.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct DivergeParams {
    /// The framing the caller is anchored on, stated neutrally.
    pub problem: String,
    /// Optional neutral context a pass may consult — the only extra information a
    /// pass receives. There is no slot for the caller's stance or preferred answer.
    pub context: Option<String>,
}

/// What each pass is grammar-constrained to produce (data-model.md).
///
/// Two flat strings, nothing for the sanitizer to strip. The lens is NOT a model
/// field; the server assigns and labels it by pass index (FR-003).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct DivergePass {
    /// The one-line reframing of the problem under this pass's lens.
    pub framing: String,
    /// What this framing changes / its key consequence.
    pub implication: String,
}

/// One returned framing — a [`DivergePass`] labeled with the lens that produced
/// it (server-assembled).
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct Perspective {
    /// The assigned lens name (server-labeled).
    pub lens: String,
    /// The reframing.
    pub framing: String,
    /// What it changes.
    pub implication: String,
}

/// The aggregated tool output (data-model.md) — the deduplicated set of distinct
/// framings. No verdict, no confidence: `diverge` scatters, it does not converge.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct DivergeResult {
    /// The deduplicated perspectives, in pass order (≤ k distinct).
    pub perspectives: Vec<Perspective>,
    /// Number of passes that completed.
    pub passes: u32,
}

/// One Diverge run: the result plus summed token usage for the record.
#[derive(Debug)]
pub struct DivergeRun {
    /// The deduplicated perspective set.
    pub result: DivergeResult,
    /// Input tokens summed across completed passes.
    pub input_tokens: u64,
    /// Output tokens summed across completed passes.
    pub output_tokens: u64,
}

/// Near-identical framings (token-set Jaccard ≥ this) are collapsed (research D4).
const DEDUP_THRESHOLD: f64 = 0.8;

/// Register the diverge mode (boot-time; enforces flat+closed).
///
/// # Errors
///
/// Propagates the registry's schema-invariant failure.
pub fn register(registry: &mut ModeRegistry, ensemble_k: u8) -> Result<(), AppError> {
    let schema = serde_json::to_value(schemars::schema_for!(DivergePass))
        .map_err(|e| AppError::ValidationFailure(format!("schema serialization: {e}")))?;
    registry.register(
        DIVERGE_ID,
        DIVERGE_DESCRIPTION,
        PROMPT_TEMPLATE,
        schema,
        ensemble_k,
    )
}

/// Build the per-pass prompt. The lens (a fixed generative perspective), the
/// problem, and the context are the only dynamic content; the lens carries no
/// caller stance.
fn build_prompt(template: &str, lens: &str, problem: &str, context: Option<&str>) -> String {
    template
        .replace("<<lens>>", lens)
        .replace("<<problem>>", problem)
        .replace("<<context>>", context.unwrap_or(""))
}

/// Validate input before any model call (FR-008): empty/whitespace or oversize
/// problem rejected, never silently trimmed.
fn check_input(params: &DivergeParams, max_chars: usize) -> Result<(), AppError> {
    if params.problem.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "problem is empty or whitespace-only".to_string(),
        ));
    }
    let len = params.problem.chars().count();
    if len > max_chars {
        return Err(AppError::InvalidInput(format!(
            "problem is {len} characters; the configured maximum is {max_chars} \
             (INPUT_MAX_CHARS); it was not trimmed"
        )));
    }
    Ok(())
}

/// Run one Diverge invocation: k parallel lensed passes, then collect + dedup.
///
/// # Errors
///
/// `InvalidInput` before any model call; otherwise the dominant failure class
/// when **zero** passes complete (a scatter tool has no quorum — any completed
/// pass is a usable perspective).
pub async fn run(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    params: &DivergeParams,
    max_chars: usize,
) -> Result<DivergeRun, AppError> {
    check_input(params, max_chars)?;

    // Each pass reframes under a distinct lens (research D1/D2): pass i uses
    // LENSES[i % LENSES.len()], so the framings scatter in different directions.
    let passes = futures::future::join_all((0..mode.ensemble_k).map(|i| {
        let lens = LENSES[usize::from(i) % LENSES.len()];
        let prompt = build_prompt(
            mode.prompt_template,
            &format!("{}: {}", lens.name, lens.directive),
            &params.problem,
            params.context.as_deref(),
        );
        async move { one_pass(client, mode, &prompt).await }
    }))
    .await;

    aggregate(passes)
}

/// One stance-blind pass: constrained completion → local validation → typed
/// perspective. A pass with an empty/whitespace `framing` is a failed pass.
async fn one_pass(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    prompt: &str,
) -> Result<(DivergePass, u64, u64), AppError> {
    let completion = client.complete(prompt, &mode.sanitized_schema).await?;
    validate(&mode.output_schema, &completion.value)?;
    let pass: DivergePass = serde_json::from_value(completion.value)
        .map_err(|e| AppError::ValidationFailure(format!("perspective shape: {e}")))?;
    if pass.framing.trim().is_empty() {
        return Err(AppError::ValidationFailure(
            "perspective with an empty framing".to_string(),
        ));
    }
    Ok((pass, completion.input_tokens, completion.output_tokens))
}

/// Collect each completed pass's perspective (labeled by its lens index), dedup
/// deterministically, and assemble the result (research D5). Zero completed →
/// the dominant failure class; no quorum, no verdict.
fn aggregate(
    passes: Vec<Result<(DivergePass, u64, u64), AppError>>,
) -> Result<DivergeRun, AppError> {
    let mut perspectives: Vec<Perspective> = Vec::new();
    let mut failures: Vec<AppError> = Vec::new();
    let (mut input_tokens, mut output_tokens) = (0_u64, 0_u64);

    for (i, pass) in passes.into_iter().enumerate() {
        match pass {
            Ok((p, inp, out)) => {
                let lens = LENSES[i % LENSES.len()].name;
                perspectives.push(Perspective {
                    lens: lens.to_string(),
                    framing: p.framing,
                    implication: p.implication,
                });
                input_tokens += inp;
                output_tokens += out;
            }
            Err(e) => failures.push(e),
        }
    }

    if perspectives.is_empty() {
        return Err(dominant_failure(failures));
    }

    #[allow(clippy::cast_possible_truncation)] // bounded by k: u8
    let passes_completed = perspectives.len() as u32;
    let perspectives = dedup(perspectives);

    Ok(DivergeRun {
        result: DivergeResult {
            perspectives,
            passes: passes_completed,
        },
        input_tokens,
        output_tokens,
    })
}

/// Deterministic dedup (research D4): collapse perspectives whose normalized
/// `framing` token sets have Jaccard ≥ [`DEDUP_THRESHOLD`], keeping the earlier
/// (lower-index) one. Keys on `framing` only — the perspective's identity.
fn dedup(perspectives: Vec<Perspective>) -> Vec<Perspective> {
    let mut kept: Vec<Perspective> = Vec::new();
    let mut kept_tokens: Vec<HashSet<String>> = Vec::new();
    for p in perspectives {
        let tokens = normalize(&p.framing);
        let is_dup = kept_tokens
            .iter()
            .any(|prior| jaccard(&tokens, prior) >= DEDUP_THRESHOLD);
        if !is_dup {
            kept_tokens.push(tokens);
            kept.push(p);
        }
    }
    kept
}

/// Normalize a framing into a token set: lowercase, split on non-alphanumeric,
/// drop empties.
fn normalize(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// Jaccard similarity of two token sets: `|A ∩ B| / |A ∪ B|`. Two empty sets are
/// treated as identical (`1.0`) so degenerate framings still collapse.
fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)] // token counts are small
    {
        intersection as f64 / union as f64
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::traits::client::{Completion, MockModelClient};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn test_mode(k: u8) -> CorrectiveMode {
        let mut registry = ModeRegistry::new();
        register(&mut registry, k).unwrap();
        registry.get(DIVERGE_ID).unwrap().clone()
    }

    fn params(problem: &str, context: Option<&str>) -> DivergeParams {
        DivergeParams {
            problem: problem.to_string(),
            context: context.map(ToString::to_string),
        }
    }

    fn scripted_client(results: Vec<Result<Value, AppError>>) -> MockModelClient {
        let cursor = Arc::new(AtomicUsize::new(0));
        let mut mock = MockModelClient::new();
        mock.expect_complete().returning(move |_, _| {
            let i = cursor.fetch_add(1, Ordering::SeqCst);
            match &results[i % results.len()] {
                Ok(value) => Ok(Completion {
                    value: value.clone(),
                    input_tokens: 100,
                    output_tokens: 10,
                }),
                Err(AppError::Refusal(m)) => Err(AppError::Refusal(m.clone())),
                Err(other) => Err(AppError::Client(other.to_string())),
            }
        });
        mock
    }

    fn persp(framing: &str, implication: &str) -> Value {
        json!({ "framing": framing, "implication": implication })
    }

    // ---- T006: schema, lenses, dedup (pure) --------------------------------

    #[test]
    fn pass_schema_registers_flat_and_closed() {
        let mode = test_mode(3);
        assert_eq!(mode.sanitized_schema["additionalProperties"], json!(false));
        assert_eq!(
            mode.sanitized_schema["properties"]["framing"]["type"],
            json!("string")
        );
        assert_eq!(
            mode.sanitized_schema["properties"]["implication"]["type"],
            json!("string")
        );
        // The lens is not a model field — the server assigns it.
        assert!(mode.sanitized_schema["properties"].get("lens").is_none());
    }

    #[test]
    fn lens_set_is_nonempty_with_unique_names_and_cycles() {
        assert!(!LENSES.is_empty());
        let mut names: Vec<&str> = LENSES.iter().map(|l| l.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "lens names must be unique");
        // Assignment cycles past the set.
        assert_eq!(LENSES[LENSES.len() % LENSES.len()].name, LENSES[0].name);
    }

    #[test]
    fn each_pass_prompt_is_pairwise_distinct() {
        let prompts: Vec<String> = (0..LENSES.len())
            .map(|i| {
                let lens = LENSES[i % LENSES.len()];
                build_prompt(
                    PROMPT_TEMPLATE,
                    &format!("{}: {}", lens.name, lens.directive),
                    "a problem",
                    None,
                )
            })
            .collect();
        for a in 0..prompts.len() {
            for b in (a + 1)..prompts.len() {
                assert_ne!(
                    prompts[a], prompts[b],
                    "lenses {a},{b} produced identical prompts"
                );
            }
        }
    }

    #[test]
    fn dedup_collapses_near_identical_and_keeps_distinct() {
        let input = vec![
            Perspective {
                lens: "invert".into(),
                framing: "More steps actually build user trust".into(),
                implication: "x".into(),
            },
            // near-identical framing (≥0.8 token overlap) → collapses, earlier kept
            Perspective {
                lens: "actor".into(),
                framing: "More steps actually build user trust now".into(),
                implication: "y".into(),
            },
            // genuinely distinct → kept
            Perspective {
                lens: "class".into(),
                framing: "This is a naming problem not a flow problem".into(),
                implication: "z".into(),
            },
        ];
        let out = dedup(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].lens, "invert"); // the earlier of the near-identical pair
        assert_eq!(out[1].lens, "class");
    }

    #[test]
    fn jaccard_and_normalize_behave() {
        let a = normalize("The quick, brown FOX!");
        let b = normalize("the quick brown fox");
        assert!((jaccard(&a, &b) - 1.0).abs() < f64::EPSILON); // same tokens
        let c = normalize("a totally different sentence");
        assert!(jaccard(&a, &c) < 0.2);
    }

    // ---- T007: the wired path through run ----------------------------------

    #[tokio::test]
    async fn returns_one_labeled_perspective_per_completed_pass() {
        let mode = test_mode(3);
        let client = scripted_client(vec![
            Ok(persp("invert framing", "impl a")),
            Ok(persp("actor framing", "impl b")),
            Ok(persp("horizon framing", "impl c")),
        ]);
        let out = run(&client, &mode, &params("p", None), 50_000)
            .await
            .unwrap();
        assert_eq!(out.result.passes, 3);
        assert_eq!(out.result.perspectives.len(), 3);
        assert_eq!(out.result.perspectives[0].lens, "invert");
        assert_eq!(out.result.perspectives[1].lens, "actor");
        assert_eq!(out.result.perspectives[2].lens, "horizon");
        assert_eq!(out.input_tokens, 300);
    }

    #[tokio::test]
    async fn an_empty_framing_pass_is_dropped() {
        let mode = test_mode(3);
        let client = scripted_client(vec![
            Ok(persp("real framing", "impl")),
            Ok(persp("   ", "impl")), // empty framing → failed pass
            Ok(persp("another framing", "impl")),
        ]);
        let out = run(&client, &mode, &params("p", None), 50_000)
            .await
            .unwrap();
        assert_eq!(out.result.perspectives.len(), 2);
        // The middle pass (lens `actor`) failed; the survivors keep the lenses of
        // their own pass indices (0 → invert, 2 → horizon) — a failed pass must
        // not shift later labels (lens-index alignment).
        assert_eq!(out.result.perspectives[0].lens, "invert");
        assert_eq!(out.result.perspectives[1].lens, "horizon");
    }

    #[tokio::test]
    async fn zero_completed_passes_returns_the_dominant_failure() {
        let mode = test_mode(2);
        let client = scripted_client(vec![
            Err(AppError::Refusal("declined".into())),
            Err(AppError::Refusal("declined".into())),
        ]);
        let err = run(&client, &mode, &params("p", None), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Refusal(_)), "got: {err}");
    }

    #[tokio::test]
    async fn empty_problem_is_rejected_before_any_model_call() {
        let mode = test_mode(3);
        let mut client = MockModelClient::new();
        client.expect_complete().times(0);
        let err = run(&client, &mode, &params("   ", None), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    // ---- T008: stance-blindness is structural ------------------------------

    #[test]
    fn prompt_has_only_lens_problem_and_context_slots() {
        let prompt = build_prompt(PROMPT_TEMPLATE, "L", "P", Some("C"));
        let expected = PROMPT_TEMPLATE
            .replace("<<lens>>", "L")
            .replace("<<problem>>", "P")
            .replace("<<context>>", "C");
        assert_eq!(prompt, expected);
        assert_eq!(PROMPT_TEMPLATE.matches("<<").count(), 3);
        assert!(
            PROMPT_TEMPLATE.contains("<<lens>>")
                && PROMPT_TEMPLATE.contains("<<problem>>")
                && PROMPT_TEMPLATE.contains("<<context>>")
        );
    }
}
