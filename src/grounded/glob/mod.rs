//! Glob expansion for grounded-verify (009): a custom extended-glob engine.
//!
//! [`translate`] compiles a pattern to a backtracking regex (the full grammar,
//! incl. extglob); [`expand`] walks the folder a glob starts with and returns
//! the deterministic, confined set of matching files.

pub mod expand;
pub mod translate;
