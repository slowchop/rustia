#[cfg(not(target_family = "wasm"))]
use rustia_compiler::compile;
use rustia_compiler::{
    compiled_call::{generate, generate_from_bundle, COMPILED_CALL_ABI},
    source_bundle::SourceBundle,
};
#[cfg(not(target_family = "wasm"))]
use rustia_parser::parse_source;
#[cfg(not(target_family = "wasm"))]
use rustia_vm::{Profile, Value, Vm};
#[cfg(not(target_family = "wasm"))]
use std::{fs, process::Command};

const SOURCE: &str =
    "local score = 0\nreturn function(step)\n score += step\n return score * 2\nend";

#[cfg(not(target_family = "wasm"))]
const CALLS: [(usize, f64); 7] = [
    (0, 1.0),
    (0, 3.0),
    (1, 9.0),
    (0, -2.0),
    (1, -0.0),
    (1, f64::INFINITY),
    (1, f64::NEG_INFINITY),
];

#[cfg(not(target_family = "wasm"))]
fn interpreted_results() -> String {
    use std::fmt::Write;
    let program = compile(&parse_source(SOURCE).unwrap()).unwrap();
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let functions = [vm.run(&program).unwrap(), vm.run(&program).unwrap()];
    let mut expected = String::new();
    for (slot, step) in CALLS {
        let value = vm
            .call_function(&program, functions[slot], &[Value::number(step)])
            .unwrap()
            .as_number()
            .unwrap();
        writeln!(expected, "{:016x}", value.to_bits()).unwrap();
    }
    expected
}

#[test]
fn unsupported_source_reports_a_span() {
    let source = "local score = 0\nreturn function(step) return unknown(step) end";
    let bundle = SourceBundle::new([("entry.luau", source)]).unwrap();
    let error = generate_from_bundle(&bundle, "entry.luau").unwrap_err();
    let span = error.span.unwrap();
    assert_eq!(&source[span.start..span.end], "unknown(step)");
}

#[test]
fn compiled_call_receipt_identifies_bundle_entry_abi_and_generated_bytes() {
    let bundle = SourceBundle::new([("first.luau", SOURCE), ("second.luau", SOURCE)]).unwrap();
    let (first_call, first) = generate_from_bundle(&bundle, "first.luau").unwrap();
    let (second_call, second) = generate_from_bundle(&bundle, "second.luau").unwrap();
    assert_eq!(first.compiler_abi, COMPILED_CALL_ABI);
    assert_eq!(first.backend, "generated-rust");
    assert_eq!(first.source_digest, bundle.digest());
    assert_eq!(first.source_digest, second.source_digest);
    assert_eq!(first_call.rust, second_call.rust);
    assert_eq!(first.artifact_digest, second.artifact_digest);
    assert_ne!(first.graph_digest, second.graph_digest);
    assert_eq!(
        first.graph_digest,
        bundle.static_module_graph(&["first.luau"]).unwrap().digest
    );

    let edited = SOURCE.replace("score * 2", "score * 3");
    let edited =
        SourceBundle::new([("first.luau", edited.as_str()), ("second.luau", SOURCE)]).unwrap();
    let (edited_call, edited) = generate_from_bundle(&edited, "first.luau").unwrap();
    assert_ne!(first_call.rust, edited_call.rust);
    assert_ne!(first.artifact_digest, edited.artifact_digest);
    assert_ne!(first.source_digest, edited.source_digest);
}

#[test]
fn closure_must_really_capture_the_numeric_local() {
    let unused = "local score = 0\nreturn function(step) return step end";
    assert!(generate(unused)
        .unwrap_err()
        .message
        .contains("must capture"));
    assert!(generate("local score = 0\nreturn function(step) return score + step end").is_ok());
    assert!(
        generate("local score = 0\nreturn function(step) score = step return step end").is_ok()
    );
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn compiler_rejects_over_budget_syntax_on_a_worker_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            for expression in [
                format!("{}1", "score+".repeat(500)),
                format!("{}score{}", "(".repeat(350), ")".repeat(350)),
            ] {
                let source = format!("local score=0 return function(step) return {expression} end");
                let error = generate(&source).unwrap_err();
                assert!(error.message.contains("token budget"));
                assert!(error.span.unwrap().end <= source.len());
            }
            let near_budget = format!(
                "local score=0 return function(step) return {}score{} end",
                "(".repeat(55),
                ")".repeat(55)
            );
            assert!(generate(&near_budget).is_ok());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn generated_rust_matches_the_interpreter_with_independent_captures() {
    let bundle = SourceBundle::new([("entry.luau", SOURCE)]).unwrap();
    let (call, _) = generate_from_bundle(&bundle, "entry.luau").unwrap();
    assert_eq!(call.capture_name, "score");
    let mut main = format!(
        "{}\nfn main() {{\nlet mut captures = [f64::from_bits({}); 2];\n",
        call.rust,
        call.initial_capture.to_bits()
    );
    use std::fmt::Write;
    for (slot, step) in CALLS {
        writeln!(
            main,
            "let (next, value) = compiled_call(captures[{slot}], f64::from_bits({}));\ncaptures[{slot}] = next;\nprintln!(\"{{:016x}}\", value.to_bits());",
            step.to_bits()
        )
        .unwrap();
    }
    main.push_str("}\n");
    let directory = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("rustia-compiled-call-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("generated.rs");
    let exe = directory.join(format!("generated{}", std::env::consts::EXE_SUFFIX));
    fs::write(&source, main).unwrap();
    let output = Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    let run = output
        .status
        .success()
        .then(|| Command::new(&exe).output().unwrap());
    fs::remove_dir_all(directory).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = run.unwrap();
    assert!(run.status.success());
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        interpreted_results()
    );
}

#[cfg(not(target_family = "wasm"))]
#[test]
#[ignore = "requires the wasm32-unknown-unknown target and Node"]
fn generated_wasm_matches_the_interpreter_with_independent_captures() {
    let bundle = SourceBundle::new([("entry.luau", SOURCE)]).unwrap();
    let (call, _) = generate_from_bundle(&bundle, "entry.luau").unwrap();
    let directory = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("rustia-compiled-wasm-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("generated.rs");
    let wasm = directory.join("generated.wasm");
    // Represent floats as i64 bits across the JS boundary to preserve NaNs.
    let adapter = r#"
#[no_mangle]
pub extern "C" fn step(capture: u64, parameter: u64, return_capture: i32) -> u64 {
    let (next, result) = compiled_call(f64::from_bits(capture), f64::from_bits(parameter));
    if return_capture == 0 { result.to_bits() } else { next.to_bits() }
}
"#;
    fs::write(&source, format!("{}\n{adapter}", call.rust)).unwrap();
    let output = Command::new("rustc")
        .args([
            "--edition=2021",
            "--crate-type=cdylib",
            "--target=wasm32-unknown-unknown",
        ])
        .arg(&source)
        .arg("-o")
        .arg(&wasm)
        .output()
        .unwrap();
    let driver = r#"
const [path, initial, ...calls] = process.argv.slice(1);
const binary = new WebAssembly.Module(require('node:fs').readFileSync(path));
const { step } = new WebAssembly.Instance(binary).exports;
const captures = [BigInt(initial), BigInt(initial)];
for (const call of calls) {
    const [slot, parameter] = call.split(':');
    const capture = captures[slot];
    const value = step(capture, BigInt(parameter), 0);
    captures[slot] = step(capture, BigInt(parameter), 1);
    console.log(BigInt.asUintN(64, value).toString(16).padStart(16, '0'));
}
"#;
    let run = output.status.success().then(|| {
        Command::new("node")
            .args(["-e", driver])
            .arg(&wasm)
            .arg(call.initial_capture.to_bits().to_string())
            .args(CALLS.map(|(slot, step)| format!("{slot}:{}", step.to_bits())))
            .output()
            .unwrap()
    });
    fs::remove_dir_all(directory).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = run.unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        interpreted_results()
    );
}
