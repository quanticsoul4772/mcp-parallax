//! Spike S1 — brute-force vector scoring at v1 scale (T003, research.md S1).
//!
//! Validates the named sqlite-vec deviation: f32 embeddings round-trip as
//! BLOBs through the existing sqlx pool, and brute-force cosine over 5,000 ×
//! 1,024-dim vectors scores in well under 50 ms — so v1 needs no vector
//! extension (and none of the unsafe-FFI/workspace cost loading one implies).
//!
//! Run: `cargo run --release --example spike_bruteforce` (no key, no network)

// Spikes are dev tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(clippy::cast_precision_loss)]

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Row;
use std::time::Instant;

const N: usize = 5_000;
const DIMS: usize = 1_024;

/// Deterministic pseudo-random f32s (no rand dependency; reproducible spike).
fn lcg_vector(seed: u64, dims: usize) -> Vec<f32> {
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (0..dims)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            // Map the top 24 bits to [-1, 1).
            ((state >> 40) as f32) / 8_388_608.0 - 1.0
        })
        .collect()
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (mut dot, mut na, mut nb) = (0.0_f32, 0.0_f32, 0.0_f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    dot / (na.sqrt() * nb.sqrt()).max(f32::EPSILON)
}

#[tokio::main]
async fn main() {
    // 1. BLOB round-trip through the same pool shape production uses.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::raw_sql("CREATE TABLE vectors (id INTEGER PRIMARY KEY, embedding BLOB NOT NULL)")
        .execute(&pool)
        .await
        .expect("table");

    let insert_start = Instant::now();
    let mut tx = pool.begin().await.expect("tx");
    for i in 0..N {
        let blob = to_blob(&lcg_vector(i as u64 + 1, DIMS));
        sqlx::query("INSERT INTO vectors (id, embedding) VALUES (?, ?)")
            .bind(i as i64)
            .bind(blob)
            .execute(&mut *tx)
            .await
            .expect("insert");
    }
    tx.commit().await.expect("commit");
    println!(
        "inserted {N} x {DIMS}-dim vectors as BLOBs in {} ms",
        insert_start.elapsed().as_millis()
    );

    // 2. Load + decode (the per-recall read cost).
    let load_start = Instant::now();
    let rows = sqlx::query("SELECT id, embedding FROM vectors")
        .fetch_all(&pool)
        .await
        .expect("fetch");
    let vectors: Vec<(i64, Vec<f32>)> = rows
        .iter()
        .map(|r| {
            (
                r.get::<i64, _>("id"),
                from_blob(r.get::<&[u8], _>("embedding")),
            )
        })
        .collect();
    let load_ms = load_start.elapsed().as_millis();
    println!("loaded + decoded {} vectors in {load_ms} ms", vectors.len());

    // Round-trip fidelity.
    let original = lcg_vector(1, DIMS);
    let restored = &vectors.iter().find(|(id, _)| *id == 0).expect("row 0").1;
    assert_eq!(&original, restored, "BLOB round-trip must be bit-exact");

    // 3. Brute-force scoring (the spike's headline number).
    let query = lcg_vector(999_999, DIMS);
    let score_start = Instant::now();
    let mut scores: Vec<(i64, f32)> = vectors
        .iter()
        .map(|(id, v)| (*id, cosine(&query, v)))
        .collect();
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).expect("no NaN"));
    let score_ms = score_start.elapsed().as_millis();
    println!(
        "scored + ranked {N} vectors in {score_ms} ms (top score {:.4})",
        scores[0].1
    );

    assert!(
        score_ms < 50,
        "scoring took {score_ms} ms — the brute-force deviation does not hold"
    );
    println!(
        "\nSPIKE S1 PASS: BLOBs round-trip bit-exact through sqlx; brute-force \
         scoring at v1 scale is {score_ms} ms (< 50 ms). sqlite-vec stays the scale path."
    );
}
