//! Spike S1 — z3 0.20 (bundled) viability (T001; gates the 005 solver work).
//!
//! Validates research.md D1 before anything depends on it: the SMT-LIB 2
//! round trip (parse → check-sat → witness), unsat detection, the in-engine
//! timeout parameter, and — the load-bearing finding — that a malformed
//! script is DETECTABLE without unsafe FFI via the assertion-count check
//! (the crate's `from_string` records Z3 errors in the context but returns
//! `()`; counting parsed assertions against the expected count surfaces
//! parse failures deterministically).
//!
//! Run: `cargo run --example spike_z3` (offline; build time IS the spike's
//! first measurement — note the clean-build wall clock).

// Spikes are dev tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::format_push_string)]

use z3::{Params, SatResult, Solver};

fn solver_with_timeout(ms: u32) -> Solver {
    let solver = Solver::new();
    let mut params = Params::new();
    params.set_u32("timeout", ms);
    solver.set_params(&params);
    solver
}

fn count_asserts(script: &str) -> usize {
    script.matches("(assert").count()
}

fn main() {
    let mut pass = true;

    // 1. Satisfiable system → Sat + extractable witness.
    let sat_script = "\
        (declare-const x Int)\n\
        (declare-const y Int)\n\
        (assert (> x 2))\n\
        (assert (< y 10))\n\
        (assert (= (+ x y) 11))\n";
    let solver = solver_with_timeout(10_000);
    solver.from_string(sat_script);
    let parsed = solver.get_assertions().len();
    println!(
        "sat script: {parsed} assertions parsed (expected {})",
        count_asserts(sat_script)
    );
    if parsed != count_asserts(sat_script) {
        println!("   FAIL: assertion count mismatch on a valid script");
        pass = false;
    }
    match solver.check() {
        SatResult::Sat => {
            let model = solver.get_model().expect("model on sat");
            let witness = format!("{model}");
            println!("   sat; witness:\n{witness}");
            if !(witness.contains('x') && witness.contains('y')) {
                println!("   FAIL: witness does not name the variables");
                pass = false;
            }
        }
        other => {
            println!("   FAIL: expected Sat, got {other:?}");
            pass = false;
        }
    }

    // 2. Contradiction → Unsat.
    let unsat_script = "\
        (declare-const a Bool)\n\
        (assert a)\n\
        (assert (not a))\n";
    let solver = solver_with_timeout(10_000);
    solver.from_string(unsat_script);
    match solver.check() {
        SatResult::Unsat => println!("unsat script: correctly Unsat"),
        other => {
            println!("unsat script: FAIL — expected Unsat, got {other:?}");
            pass = false;
        }
    }

    // 3. Malformed script → detectable via the assertion-count check.
    let bad_script = "\
        (declare-const z Int)\n\
        (assert (> z 0))\n\
        (assert (this is not smtlib\n";
    let solver = solver_with_timeout(10_000);
    solver.from_string(bad_script);
    let parsed = solver.get_assertions().len();
    let expected = count_asserts(bad_script);
    println!("malformed script: {parsed} assertions parsed vs {expected} expected");
    if parsed >= expected {
        println!("   FAIL: parse failure NOT detectable by assertion count");
        pass = false;
    }

    // 4. Interior NUL would panic the crate's CString::new — the wrapper
    //    must reject it first. Here we only document the requirement.
    println!("nul-byte guard: wrapper must reject \\0 before from_string (crate would panic)");

    // 5. Timeout parameter takes effect: a hard instance returns Unknown
    //    promptly instead of hanging. (Large pigeonhole-style instance.)
    let mut hard = String::new();
    for i in 0..18 {
        hard.push_str(&format!("(declare-const p{i} Int)\n"));
        hard.push_str(&format!("(assert (and (>= p{i} 1) (<= p{i} 17)))\n"));
    }
    // all-different over 18 vars in 17 slots, written as pairwise ≠ — unsat
    // but expensive for the default tactic at small timeout.
    for i in 0..18 {
        for j in (i + 1)..18 {
            hard.push_str(&format!("(assert (not (= p{i} p{j})))\n"));
        }
    }
    let solver = solver_with_timeout(50); // 50 ms — intentionally tiny
    solver.from_string(hard.as_str());
    let start = std::time::Instant::now();
    let outcome = solver.check();
    let elapsed = start.elapsed();
    println!(
        "hard instance at 50 ms timeout: {outcome:?} in {} ms",
        elapsed.as_millis()
    );
    if elapsed.as_secs() > 5 {
        println!("   FAIL: timeout parameter did not bound the solve");
        pass = false;
    }

    // 6. Determinism: identical script twice → identical outcome.
    let s1 = solver_with_timeout(10_000);
    s1.from_string(sat_script);
    let s2 = solver_with_timeout(10_000);
    s2.from_string(sat_script);
    if s1.check() == s2.check() {
        println!("determinism: identical outcomes on identical scripts");
    } else {
        println!("determinism: FAIL — same script, different outcomes");
        pass = false;
    }

    println!(
        "\nSPIKE S1 (z3 bundled): {}",
        if pass { "PASS" } else { "FAIL" }
    );
    assert!(pass);
}
