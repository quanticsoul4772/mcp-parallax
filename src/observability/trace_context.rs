//! The caller's trace context, read from a tool request's `_meta` (058).

use opentelemetry::Context;

/// The caller's trace context, read from a tool request's `_meta` (058).
///
/// MCP 2026-07-28 carries W3C Trace Context in `_meta` as `traceparent` and
/// `tracestate`. A span started under this context joins the caller's trace;
/// with no valid `traceparent` there is no `CallerTrace` and the span stays a
/// root trace, as before. `baggage` is never read. Never stored or logged.
#[derive(Debug, Clone)]
pub struct CallerTrace {
    context: Context,
}

impl CallerTrace {
    /// Parse the caller's trace context, or `None` without a valid
    /// `traceparent`.
    ///
    /// rmcp's accessors return `None` for a non-string value, and the SDK's
    /// W3C propagator rejects malformed and all-zero ids and drops an invalid
    /// `tracestate` while keeping the parent.
    #[must_use]
    pub fn from_meta(meta: &rmcp::model::MetaObject) -> Option<Self> {
        use opentelemetry::propagation::TextMapPropagator as _;
        use opentelemetry::trace::TraceContextExt as _;

        let traceparent = meta.get_traceparent()?;
        let mut carrier = std::collections::HashMap::with_capacity(2);
        carrier.insert("traceparent".to_string(), traceparent.to_string());
        if let Some(tracestate) = meta.get_tracestate() {
            carrier.insert("tracestate".to_string(), tracestate.to_string());
        }
        let context = opentelemetry_sdk::propagation::TraceContextPropagator::new()
            .extract_with_context(&Context::new(), &carrier);
        context
            .span()
            .span_context()
            .is_valid()
            .then_some(Self { context })
    }

    /// The caller's span, as a remote span context.
    #[must_use]
    pub fn span_context(&self) -> opentelemetry::trace::SpanContext {
        use opentelemetry::trace::TraceContextExt as _;
        self.context.span().span_context().clone()
    }
}

/// The context a tool call's span starts under: the caller's trace when the
/// request carried one (058), otherwise empty, which makes a root span.
pub(super) fn parent_context(caller: Option<&CallerTrace>) -> Context {
    caller.map_or_else(Context::new, |trace| trace.context.clone())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(super) mod tests {
    use super::*;

    const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    pub(in crate::observability) fn meta(
        pairs: &[(&str, serde_json::Value)],
    ) -> rmcp::model::MetaObject {
        let mut meta = rmcp::model::MetaObject::new();
        for (key, value) in pairs {
            meta.0.insert((*key).to_string(), value.clone());
        }
        meta
    }

    pub(in crate::observability) fn text(value: &str) -> serde_json::Value {
        serde_json::Value::String(value.to_string())
    }

    /// T001: a valid `traceparent` gives the caller's span as a remote parent.
    #[test]
    fn a_valid_traceparent_is_the_parent() {
        let trace = CallerTrace::from_meta(&meta(&[("traceparent", text(TRACEPARENT))]))
            .expect("a valid traceparent parses");
        let span = trace.span_context();
        assert_eq!(
            span.trace_id().to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
        assert_eq!(span.span_id().to_string(), "00f067aa0ba902b7");
        assert!(span.is_remote());
        assert!(span.is_sampled());
    }

    /// T001: a valid `tracestate` travels with the parent.
    #[test]
    fn a_valid_tracestate_is_carried() {
        let trace = CallerTrace::from_meta(&meta(&[
            ("traceparent", text(TRACEPARENT)),
            ("tracestate", text("vendor=abc")),
        ]))
        .expect("parses");
        assert_eq!(trace.span_context().trace_state().header(), "vendor=abc");
    }

    /// T001: an invalid `tracestate` is dropped; the parent still holds.
    #[test]
    fn an_invalid_tracestate_is_dropped_but_the_parent_holds() {
        let trace = CallerTrace::from_meta(&meta(&[
            ("traceparent", text(TRACEPARENT)),
            ("tracestate", text("not a valid tracestate!")),
        ]))
        .expect("the traceparent alone is enough");
        assert_eq!(trace.span_context().trace_state().header(), "");
        assert_eq!(
            trace.span_context().trace_id().to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
    }

    /// T001: malformed and all-zero values give no parent.
    #[test]
    fn a_malformed_or_zero_traceparent_gives_no_parent() {
        for bad in [
            "garbage",
            "00-4bf92f3577b34da6a3ce929d0e0e4736",
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
        ] {
            assert!(
                CallerTrace::from_meta(&meta(&[("traceparent", text(bad))])).is_none(),
                "{bad}"
            );
        }
    }

    /// T001: a value that is not a string is absent.
    #[test]
    fn a_non_string_traceparent_is_absent() {
        assert!(CallerTrace::from_meta(&meta(&[("traceparent", serde_json::json!(42))])).is_none());
    }

    /// T001: `tracestate` or `baggage` without a `traceparent` parents nothing,
    /// and `baggage` is never read.
    #[test]
    fn tracestate_or_baggage_alone_gives_no_parent() {
        assert!(CallerTrace::from_meta(&meta(&[("tracestate", text("vendor=abc"))])).is_none());
        assert!(CallerTrace::from_meta(&meta(&[("baggage", text("user=x"))])).is_none());
        assert!(CallerTrace::from_meta(&meta(&[])).is_none());
    }

    /// T001: a sampled-off flag is kept, so the parent-based sampler can follow
    /// the caller's decision (FR-009).
    #[test]
    fn a_sampled_off_flag_is_kept() {
        let off = TRACEPARENT.replace("-01", "-00");
        let trace = CallerTrace::from_meta(&meta(&[("traceparent", text(&off))]))
            .expect("a sampled-off traceparent is still valid");
        assert!(!trace.span_context().is_sampled());
    }
}
