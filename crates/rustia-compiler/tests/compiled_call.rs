#[cfg(not(target_family = "wasm"))]
use rustia_compiler::compile;
#[cfg(not(target_family = "wasm"))]
use rustia_compiler::compiled_call::{prepare_runtime, CompiledCallReceipt};
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
    let source = "local score = 0\nreturn function(step) return score + math.abs(step) end";
    let bundle = SourceBundle::new([("entry.luau", source)]).unwrap();
    let error = generate_from_bundle(&bundle, "entry.luau").unwrap_err();
    let span = error.span.unwrap();
    assert_eq!(&source[span.start..span.end], "math.abs(step)");

    #[cfg(not(target_family = "wasm"))]
    {
        let program = compile(&parse_source(source).unwrap()).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let function = vm.run(&program).unwrap();
        let result = vm
            .call_function(&program, function, &[Value::number(-2.0)])
            .unwrap();
        assert_eq!(result.as_number(), Some(2.0));
    }
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
fn compiled_and_interpreted_calls_share_the_vm_capture() {
    use rustia_vm::VmSettings;
    let bundle = SourceBundle::new([("entry.luau", SOURCE)]).unwrap();
    let (_, receipt) = generate_from_bundle(&bundle, "entry.luau").unwrap();
    let prepared = prepare_runtime(&bundle, "entry.luau").unwrap();
    let edited =
        SourceBundle::new([("entry.luau", SOURCE.replace("score * 2", "score * 3"))]).unwrap();
    let (_, stale) = generate_from_bundle(&edited, "entry.luau").unwrap();
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let call = prepared.instantiate(&mut vm).unwrap();
    let value = call.call_interpreted(&mut vm, Value::number(1.0)).unwrap();
    assert_eq!(value.as_number(), Some(2.0));
    let value = call
        .call(
            &mut vm,
            Value::number(3.0),
            &receipt,
            |capture, parameter| {
                assert_eq!(capture, 1.0);
                let next = capture + parameter;
                Ok((next, next * 2.0))
            },
        )
        .unwrap();
    assert_eq!(value.as_number(), Some(8.0));
    let value = call
        .call(&mut vm, Value::number(-2.0), &stale, |_, _| {
            panic!("stale artifact ran")
        })
        .unwrap();
    assert_eq!(value.as_number(), Some(4.0));

    for settings in [
        VmSettings {
            initial_fuel: Some(128),
            ..VmSettings::default()
        },
        VmSettings {
            memory_limit_bytes: Some(4 * 1024 * 1024),
            ..VmSettings::default()
        },
    ] {
        let mut metered = Vm::with_settings(Profile::ReproducibleStandard, settings);
        let call = prepared.instantiate(&mut metered).unwrap();
        let value = call
            .call(&mut metered, Value::number(1.0), &receipt, |_, _| {
                panic!("metered backend ran")
            })
            .unwrap();
        assert_eq!(value.as_number(), Some(2.0));
    }
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn compiled_call_matches_instruction_clock_and_defers_collection() {
    use rustia_vm::VmSettings;
    use std::num::NonZeroU64;

    let bundle = SourceBundle::new([("entry.luau", SOURCE)]).unwrap();
    let (_, receipt) = generate_from_bundle(&bundle, "entry.luau").unwrap();
    let prepared = prepare_runtime(&bundle, "entry.luau").unwrap();
    let clock = compile(&parse_source("return os.clock()").unwrap()).unwrap();
    for (interval, should_compile) in [(1, false), (10_000, true)] {
        let settings = VmSettings {
            deterministic_gc_interval: NonZeroU64::new(interval).unwrap(),
            ..VmSettings::default()
        };
        let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings.clone());
        let mut reference = Vm::with_settings(Profile::ReproducibleStandard, settings);
        let call = prepared.instantiate(&mut vm).unwrap();
        let oracle = prepared.instantiate(&mut reference).unwrap();
        for step in [1.0, 3.0, -2.0] {
            let mut ran_backend = false;
            let result = call
                .call(
                    &mut vm,
                    Value::number(step),
                    &receipt,
                    |capture, parameter| {
                        ran_backend = true;
                        let next = capture + parameter;
                        Ok((next, next * 2.0))
                    },
                )
                .unwrap();
            let expected = oracle
                .call_interpreted(&mut reference, Value::number(step))
                .unwrap();
            assert_eq!(ran_backend, should_compile);
            assert_eq!(result.as_number(), expected.as_number());
            assert_eq!(
                vm.executed_instructions(),
                reference.executed_instructions()
            );
            assert_eq!(
                vm.next_deterministic_collection_instruction(),
                reference.next_deterministic_collection_instruction()
            );
        }
        assert_eq!(
            vm.run(&clock).unwrap().as_number(),
            reference.run(&clock).unwrap().as_number()
        );
    }
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn compiled_call_preserves_buffer_nan_payloads_via_the_interpreter() {
    let source = "local score = 0 return function(step) score = step return score end";
    let bundle = SourceBundle::new([("entry.luau", source)]).unwrap();
    let (_, receipt) = generate_from_bundle(&bundle, "entry.luau").unwrap();
    let prepared = prepare_runtime(&bundle, "entry.luau").unwrap();
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let call = prepared.instantiate(&mut vm).unwrap();
    let read_bits = compile(&parse_source(
        "return buffer.readf64(buffer.fromstring(string.char(52, 18, 0, 0, 0, 0, 248, 127)), 0)",
    ).unwrap()).unwrap();
    let payload = vm.run(&read_bits).unwrap();
    let value = call
        .call(&mut vm, payload, &receipt, |_, _| {
            panic!("noncanonical argument entered backend")
        })
        .unwrap();
    assert_eq!(value.as_number().unwrap().to_bits(), 0x7ff8_0000_0000_1234);
    let value = call
        .call(&mut vm, Value::number(7.0), &receipt, |_, _| {
            panic!("noncanonical capture entered backend")
        })
        .unwrap();
    assert_eq!(value.as_number(), Some(7.0));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn compiled_call_rejects_poisoned_vm_before_entering_backend() {
    use rustia_vm::{ErrorKind, ModuleResolver, VmSettings};
    let bundle = SourceBundle::new([("entry.luau", SOURCE)]).unwrap();
    let (_, receipt) = generate_from_bundle(&bundle, "entry.luau").unwrap();
    let prepared = prepare_runtime(&bundle, "entry.luau").unwrap();
    let mut vm = Vm::with_settings(
        Profile::ReproducibleStandard,
        VmSettings {
            module_resolver: Some(ModuleResolver::new(
                |_, _| panic!("resolver poisoned VM"),
                true,
            )),
            ..VmSettings::default()
        },
    );
    let call = prepared.instantiate(&mut vm).unwrap();
    let require = compile(&parse_source("return require('./module')").unwrap()).unwrap();
    assert!(matches!(
        vm.run(&require).unwrap_err().kind,
        ErrorKind::HostPanicked { .. }
    ));
    let error = call
        .call(&mut vm, Value::number(1.0), &receipt, |_, _| {
            panic!("poisoned VM entered backend")
        })
        .unwrap_err();
    assert!(matches!(error.kind, ErrorKind::VmPoisoned));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn generated_rust_matches_the_interpreter_with_independent_captures() {
    let bundle = SourceBundle::new([("entry.luau", SOURCE)]).unwrap();
    let (call, receipt) = generate_from_bundle(&bundle, "entry.luau").unwrap();
    assert_eq!(call.capture_name, "score");
    let driver = r#"
fn main() {
    let mut inputs = std::env::args().skip(1).map(|arg| f64::from_bits(arg.parse().unwrap()));
    let (next, value) = compiled_call(inputs.next().unwrap(), inputs.next().unwrap());
    println!("{:016x}\n{:016x}", next.to_bits(), value.to_bits());
}
"#;
    let directory = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("rustia-compiled-call-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("generated.rs");
    let exe = directory.join(format!("generated{}", std::env::consts::EXE_SUFFIX));
    fs::write(&source, format!("{}\n{driver}", call.rust)).unwrap();
    let output = Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    verify_backend(&bundle, &receipt, || Command::new(&exe));
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(not(target_family = "wasm"))]
#[test]
#[ignore = "requires the wasm32-unknown-unknown target and Node"]
fn generated_wasm_matches_the_interpreter_with_independent_captures() {
    let bundle = SourceBundle::new([("entry.luau", SOURCE)]).unwrap();
    let (call, receipt) = generate_from_bundle(&bundle, "entry.luau").unwrap();
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
const [path, capture, parameter] = process.argv.slice(1);
const binary = new WebAssembly.Module(require('node:fs').readFileSync(path));
const { step } = new WebAssembly.Instance(binary).exports;
for (const field of [1, 0]) {
    console.log(BigInt.asUintN(64, step(BigInt(capture), BigInt(parameter), field)).toString(16));
}
"#;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    verify_backend(&bundle, &receipt, || {
        let mut node = Command::new("node");
        node.args(["-e", driver]).arg(&wasm);
        node
    });
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(not(target_family = "wasm"))]
fn verify_backend(
    bundle: &SourceBundle,
    receipt: &CompiledCallReceipt,
    command: impl Fn() -> Command,
) {
    let prepared = prepare_runtime(bundle, "entry.luau").unwrap();
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let calls = [
        prepared.instantiate(&mut vm).unwrap(),
        prepared.instantiate(&mut vm).unwrap(),
    ];
    let mut results = String::new();
    let mut executed = 0;
    use std::fmt::Write;
    for (slot, parameter) in CALLS {
        let value = calls[slot]
            .call(
                &mut vm,
                Value::number(parameter),
                receipt,
                |capture, parameter| {
                    executed += 1;
                    let run = command()
                        .arg(capture.to_bits().to_string())
                        .arg(parameter.to_bits().to_string())
                        .output()
                        .unwrap();
                    assert!(
                        run.status.success(),
                        "{}",
                        String::from_utf8_lossy(&run.stderr)
                    );
                    let stdout = String::from_utf8(run.stdout).unwrap();
                    let mut numbers = stdout
                        .lines()
                        .map(|line| f64::from_bits(u64::from_str_radix(line, 16).unwrap()));
                    let values = (numbers.next().unwrap(), numbers.next().unwrap());
                    assert!(numbers.next().is_none());
                    Ok(values)
                },
            )
            .unwrap();
        writeln!(
            &mut results,
            "{:016x}",
            value.as_number().unwrap().to_bits()
        )
        .unwrap();
    }
    assert_eq!(executed, CALLS.len());
    assert_eq!(results, interpreted_results());
}
