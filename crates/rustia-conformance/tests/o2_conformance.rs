//! Full-corpus conformance under the default-disabled O2 families.
//!
//! The C2.3 enablement gate from `design/58-vm-core2.md`: every fixture —
//! including fuel-limit, coverage, and deterministic-collection expectations
//! — must hold its recorded behavior when fixtures are compiled with O2
//! families enabled, because the attached logical-unit schedule restores the
//! unoptimized fuel/deterministic-GC accounting.
//!
//! The all-families run executes on every test pass (it is the strictest
//! single configuration). The per-family runs are `#[ignore]`d so targeted
//! enablement validation stays cheap to invoke without quintupling routine
//! CI time:
//!
//! ```text
//! cargo test -p rustia-conformance --test o2_conformance -- --ignored
//! ```

use rustia_compiler::ExperimentalO2Options;
use rustia_conformance::{repository_from_crate_manifest, run_repository_with_experimental_o2};
use std::path::PathBuf;
use std::thread;

fn run_with(o2: ExperimentalO2Options) {
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let report = thread::Builder::new()
        .name("rustia-conformance-o2".to_owned())
        .stack_size(128 * 1024 * 1024)
        .spawn(move || run_repository_with_experimental_o2(repo, o2).unwrap())
        .unwrap()
        .join()
        .unwrap();
    assert!(report.refs > 0);
    assert!(report.fixtures > 0);
    assert_eq!(report.pending_oracle, 0);
    assert_eq!(report.pending_impl, 0);
}

#[test]
fn corpus_passes_with_the_reachability_exact_families() {
    // The enablement track: constant-branch folding, commit-late compound
    // locals, reachability-exact tail folds, and slot tightening together.
    run_with(ExperimentalO2Options::reachability_exact());
}

#[test]
#[ignore = "documents the C2.4 dependency; run with -- --ignored"]
fn clear_elision_diverges_on_weak_root_liveness_until_c24_masks_roots() {
    // Deleting a standalone dead-value clear keeps the stale value
    // register-rooted through later deterministic collection boundaries; the
    // upstream weak key/value root-liveness fixture observes exactly one
    // extra surviving weak entry. This stays a hard failure until C2.4's
    // frame root scanning masks registers the unoptimized program would have
    // cleared. When that lands and this test starts failing, flip it into a
    // passing corpus gate like the other families.
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let error = thread::Builder::new()
        .name("rustia-conformance-o2-elide".to_owned())
        .stack_size(128 * 1024 * 1024)
        .spawn(move || {
            run_repository_with_experimental_o2(
                repo,
                ExperimentalO2Options {
                    elide_dead_cleanup_clears: true,
                    ..ExperimentalO2Options::default()
                },
            )
        })
        .unwrap()
        .join()
        .unwrap()
        .expect_err("clear elision must fail weak-root liveness until C2.4");
    assert!(
        format!("{error}").contains("weak_key_value_root_liveness"),
        "unexpected failure shape: {error}"
    );
}

#[test]
#[ignore = "per-family enablement gate; run with -- --ignored"]
fn corpus_passes_with_fold_constant_branches() {
    run_with(ExperimentalO2Options {
        fold_constant_branches: true,
        ..ExperimentalO2Options::default()
    });
}

#[test]
#[ignore = "per-family enablement gate; run with -- --ignored"]
fn corpus_passes_with_commit_late_compound_locals() {
    run_with(ExperimentalO2Options {
        commit_late_compound_locals: true,
        ..ExperimentalO2Options::default()
    });
}

#[test]
#[ignore = "per-family enablement gate; run with -- --ignored"]
fn corpus_passes_with_remove_dead_register_tails() {
    run_with(ExperimentalO2Options {
        remove_dead_register_tails: true,
        ..ExperimentalO2Options::default()
    });
}

#[test]
#[ignore = "per-family enablement gate; run with -- --ignored"]
fn corpus_passes_with_tighten_local_slots() {
    run_with(ExperimentalO2Options {
        tighten_local_slots: true,
        ..ExperimentalO2Options::default()
    });
}
