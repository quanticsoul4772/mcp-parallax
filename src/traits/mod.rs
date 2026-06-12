//! Mockable trait boundaries so the whole server tests without network or disk.
//!
//! Composition over trait inheritance: each component holds the concrete
//! dependencies it needs behind these traits, rather than inheriting behavior.
//! The seams: time, the model client, storage, embeddings, search, and fetch.

pub mod client;
pub mod clock;
pub mod embedder;
pub mod fetcher;
pub mod search;
pub mod storage;

pub use client::{Completion, ModelClient};
pub use clock::{SystemClock, TimeProvider};
pub use embedder::{Embedder, Embedding};
pub use fetcher::{FetchedPage, Fetcher};
pub use search::{SearchHit, SearchProvider};
pub use storage::Storage;
