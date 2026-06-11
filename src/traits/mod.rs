//! Mockable trait boundaries so the whole server tests without network or disk.
//!
//! Composition over trait inheritance: each component holds the concrete
//! dependencies it needs behind these traits, rather than inheriting behavior.
//! The three seams are time, the model client, and storage.

mod client;
mod clock;
mod storage;

pub use client::ModelClient;
pub use clock::{SystemClock, TimeProvider};
pub use storage::Storage;
