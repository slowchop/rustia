//! Corpus-wide compile validation for the analysis and the default-disabled
//! O2 families.
//!
//! Every conformance fixture is compiled twice — default options and every
//! O2 family enabled — and the pair is checked against the structural
//! contracts that do not need the per-fixture execution harness:
//!
//! - the liveness analysis accepts every compiled function,
//! - O2 never grows code or register files, never drops a required
//!   captured-local detach, and
//! - the logical receipts of every optimized function sum to the
//!   unoptimized instruction count, keeping the tape-restorable schedule.
//!
//! Executing fixtures under O2 needs the conformance harness (per-fixture
//! preludes and settings) and stays an explicit enablement-time gate; the
//! execution differentials in `o2_families.rs` and `opcode_census.rs` cover
//! semantics on targeted programs and the tracked workloads.

use std::path::{Path, PathBuf};

use rustia_compiler::{
    compile_with_options, BuiltinEnvironment, CompileOptions, ExperimentalO2Options,
};
use rustia_parser::parse_source_bytes;
use rustia_vm::liveness::{ClearKind, FunctionLiveness};
use rustia_vm::{Function, Program};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/fixtures")
}

fn luau_files(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            luau_files(&path, files);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "luau")
        {
            files.push(path);
        }
    }
}

fn compile_fixture(source: &[u8], name: &str, o2: ExperimentalO2Options) -> Option<Program> {
    let chunk = parse_source_bytes(source).ok()?;
    let options = CompileOptions {
        builtins: BuiltinEnvironment::LuauConformance,
        experimental_o2: o2,
        ..CompileOptions::default()
    }
    .with_source_bytes(name, source);
    compile_with_options(&chunk, options).ok()
}

fn functions(program: &Program) -> impl Iterator<Item = &Function> {
    std::iter::once(&program.main).chain(program.functions.iter())
}

fn required_detaches(program: &Program, function: &Function) -> usize {
    let liveness = FunctionLiveness::analyze(program, function)
        .unwrap_or_else(|error| panic!("analysis failed: {error}"));
    liveness
        .clear_sites()
        .into_iter()
        .filter(|site| site.kind == ClearKind::RequiredDetach)
        .count()
}

#[test]
fn every_fixture_analyzes_and_o2_preserves_structural_contracts() {
    let mut files = Vec::new();
    luau_files(&fixtures_root(), &mut files);
    assert!(
        files.len() > 500,
        "expected the conformance fixture corpus, found {} files",
        files.len()
    );

    let mut compiled = 0usize;
    let mut optimized_smaller = 0usize;
    for path in &files {
        let source =
            std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("fixture");
        let Some(baseline) = compile_fixture(&source, name, ExperimentalO2Options::default())
        else {
            // Fixtures that need runtime compilation context stay with the
            // conformance runner; this corpus check only covers what compiles
            // standalone.
            continue;
        };
        compiled += 1;

        // The analysis accepts every compiled function.
        for function in functions(&baseline) {
            FunctionLiveness::analyze(&baseline, function)
                .unwrap_or_else(|error| panic!("analysis failed on {}: {error}", path.display()));
        }

        let optimized = compile_fixture(&source, name, ExperimentalO2Options::ALL)
            .unwrap_or_else(|| panic!("O2 compile failed on {}", path.display()));
        assert_eq!(baseline.functions.len(), optimized.functions.len());

        let mut baseline_total = 0usize;
        let mut optimized_total = 0usize;
        for (original, rewritten) in functions(&baseline).zip(functions(&optimized)) {
            assert!(
                rewritten.code.len() <= original.code.len(),
                "O2 grew {}",
                path.display()
            );
            assert!(
                rewritten.max_registers <= original.max_registers,
                "O2 grew the register file of {}",
                path.display()
            );
            // Captured-local metadata and detach requirements survive.
            assert_eq!(original.captured_registers, rewritten.captured_registers);
            assert_eq!(
                required_detaches(&baseline, original),
                required_detaches(&optimized, rewritten),
                "O2 changed required detaches in {}",
                path.display()
            );
            baseline_total += original.code.len();
            optimized_total += rewritten.code.len();
        }
        if optimized_total < baseline_total {
            optimized_smaller += 1;
        }
    }

    assert!(
        compiled > 500,
        "expected most fixtures to compile standalone, compiled {compiled}"
    );
    assert!(
        optimized_smaller > compiled / 2,
        "O2 should shrink most real programs ({optimized_smaller}/{compiled} shrank)"
    );
}

#[test]
fn o2_receipts_cover_the_fixture_corpus() {
    let mut files = Vec::new();
    luau_files(&fixtures_root(), &mut files);

    let mut checked = 0usize;
    for path in &files {
        let source =
            std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("fixture");
        let Ok(chunk) = parse_source_bytes(&source) else {
            continue;
        };
        let options = CompileOptions {
            builtins: BuiltinEnvironment::LuauConformance,
            experimental_o2: ExperimentalO2Options::ALL,
            ..CompileOptions::default()
        }
        .with_source_bytes(name, source.as_slice());
        let baseline_options = CompileOptions {
            experimental_o2: ExperimentalO2Options::default(),
            ..options.clone()
        };
        let Ok(baseline) = compile_with_options(&chunk, baseline_options) else {
            continue;
        };
        let Ok((optimized, receipts)) =
            rustia_compiler::compile_with_options_and_receipts(&chunk, options)
        else {
            continue;
        };
        let optimized_receipts = std::iter::once((&optimized.main, &receipts.main))
            .chain(optimized.functions.iter().zip(receipts.functions.iter()));
        let baseline_functions = functions(&baseline);
        for ((function, receipts), original) in optimized_receipts.zip(baseline_functions) {
            assert_eq!(
                receipts.logical_len(0, function.code.len()),
                original.code.len() as u64,
                "receipts lost units in {}",
                path.display()
            );
        }
        checked += 1;
    }
    assert!(
        checked > 500,
        "expected receipt coverage, checked {checked}"
    );
}
