//! The schema pipeline — the concrete form of the constrained-output contract.
//!
//! One `schemars`-derived schema per mode output type feeds **both hops**: the
//! rmcp tool `outputSchema` (MCP client ← server) and the Anthropic
//! `output_config.format.schema` (server → model). Between "derive" and "send
//! to Anthropic" sits the [`sanitize`] transform (the API accepts only a
//! grammar subset), and on the way back the [`validate`] check re-imposes
//! exactly the constraints the sanitizer stripped.
//!
//! **API grammar guarantees shape; the local validator guarantees the value
//! constraints the grammar can't.** Neither is redundant.

pub mod sanitize;
pub mod validate;

pub use sanitize::sanitize;
pub use validate::validate;
