//! The five single-shape corrective handlers (032).
//!
//! `verify`, `unstick`, `decide`, `elicit` and `diverge` are structurally
//! identical: read the caller's overrides, look the mode up, then run one
//! recorded call on the client the pool resolves for that site. They sat in
//! `server.rs` alongside everything else and helped make it the largest file
//! in the tree; both reviewers of 028 named this the split seam.
//!
//! Deliberately **only these five**. `check` and `grounded_verify` are
//! correctives too, but each holds its client in a startup-built deps struct
//! and needs a different entry point (`run_with` / `evaluate_with`), so moving
//! them would fold a reshape into what is otherwise a pure relocation. This
//! file is a move: no behaviour change, and the diff should read as one.
//!
//! A child module of `server`, so it reaches `Parallax`'s private fields and
//! `run_recorded` without widening any visibility.

use super::{
    CallSite, ErrorData, Json, Parallax, RecordDims, DECIDE_ID, DIVERGE_ID, ELICIT_ID, UNSTICK_ID,
    VERIFY_ID,
};
use crate::modes::decide::{self, DecideParams, DecideResult};
use crate::modes::diverge::{self, DivergeParams, DivergeResult};
use crate::modes::elicit::{self, ElicitParams, ElicitResult};
use crate::modes::unstick::{self, NextStep, UnstickParams};
use crate::modes::verify::{self, Verdict, VerifyParams};

impl Parallax {
    pub(super) async fn verify_with_ct(
        &self,
        params: VerifyParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<Verdict>, ErrorData> {
        let effort = params.effort;
        let mode = self.registry.get(VERIFY_ID).ok_or_else(|| {
            ErrorData::internal_error("verify mode not registered".to_string(), None)
        })?;
        self.run_recorded(
            VERIFY_ID,
            self.attributed(CallSite::Verify),
            RecordDims::ensemble(effort, params.passes.map(u32::from)),
            ct,
            async {
                verify::run(
                    self.pool
                        .for_site_with_effort(CallSite::Verify, effort)
                        .as_ref(),
                    mode,
                    &params,
                    self.max_claim_chars,
                )
                .await
                .map(|run| (run.verdict, run.input_tokens, run.output_tokens))
            },
        )
        .await
    }

    pub(super) async fn unstick_with_ct(
        &self,
        params: UnstickParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<NextStep>, ErrorData> {
        let effort = params.effort;
        let mode = self.registry.get(UNSTICK_ID).ok_or_else(|| {
            ErrorData::internal_error("unstick mode not registered".to_string(), None)
        })?;
        self.run_recorded(
            UNSTICK_ID,
            self.attributed(CallSite::Unstick),
            RecordDims::effort(effort),
            ct,
            async {
                unstick::run(
                    self.pool
                        .for_site_with_effort(CallSite::Unstick, effort)
                        .as_ref(),
                    mode,
                    &params,
                    self.max_claim_chars,
                )
                .await
                .map(|run| (run.step, run.input_tokens, run.output_tokens))
            },
        )
        .await
    }

    pub(super) async fn decide_with_ct(
        &self,
        params: DecideParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<DecideResult>, ErrorData> {
        let effort = params.effort;
        let mode = self.registry.get(DECIDE_ID).ok_or_else(|| {
            ErrorData::internal_error("decide mode not registered".to_string(), None)
        })?;
        self.run_recorded(
            DECIDE_ID,
            self.attributed(CallSite::Decide),
            RecordDims::effort(effort),
            ct,
            async {
                decide::run(
                    self.pool
                        .for_site_with_effort(CallSite::Decide, effort)
                        .as_ref(),
                    mode,
                    &params,
                    self.max_claim_chars,
                )
                .await
                .map(|run| (run.result, run.input_tokens, run.output_tokens))
            },
        )
        .await
    }

    pub(super) async fn elicit_with_ct(
        &self,
        params: ElicitParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<ElicitResult>, ErrorData> {
        let effort = params.effort;
        let mode = self.registry.get(ELICIT_ID).ok_or_else(|| {
            ErrorData::internal_error("elicit mode not registered".to_string(), None)
        })?;
        // Memory only enriches: pass it when configured, run without it otherwise.
        let memory = self.memory.as_deref();
        self.run_recorded(
            ELICIT_ID,
            self.attributed(CallSite::Elicit),
            RecordDims::effort(effort),
            ct,
            async {
                elicit::run(
                    self.pool
                        .for_site_with_effort(CallSite::Elicit, effort)
                        .as_ref(),
                    mode,
                    memory,
                    &params,
                    self.max_claim_chars,
                )
                .await
                .map(|run| (run.result, run.input_tokens, run.output_tokens))
            },
        )
        .await
    }

    pub(super) async fn diverge_with_ct(
        &self,
        params: DivergeParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<DivergeResult>, ErrorData> {
        let effort = params.effort;
        let mode = self.registry.get(DIVERGE_ID).ok_or_else(|| {
            ErrorData::internal_error("diverge mode not registered".to_string(), None)
        })?;
        self.run_recorded(
            DIVERGE_ID,
            self.attributed(CallSite::Diverge),
            RecordDims::ensemble(effort, params.passes.map(u32::from)),
            ct,
            async {
                diverge::run(
                    self.pool
                        .for_site_with_effort(CallSite::Diverge, effort)
                        .as_ref(),
                    mode,
                    &params,
                    self.max_claim_chars,
                )
                .await
                .map(|run| (run.result, run.input_tokens, run.output_tokens))
            },
        )
        .await
    }
}
