#[cfg(not(target_family = "wasm"))]
use rustia_compiler::compile;
use rustia_compiler::compiled_call::generate;
#[cfg(not(target_family = "wasm"))]
use rustia_parser::parse_source;
#[cfg(not(target_family = "wasm"))]
use rustia_vm::{Profile, Value, Vm};
#[cfg(not(target_family = "wasm"))]
use std::{fs, process::Command};

#[cfg(not(target_family = "wasm"))]
const SOURCE: &str =
    "local score = 0\nreturn function(step)\n score += step\n return score * 2\nend";

#[test]
fn unsupported_source_reports_a_span() {
    let source = "local score = 0\nreturn function(step) return unknown(step) end";
    let error = generate(source).unwrap_err();
    let span = error.span.unwrap();
    assert_eq!(&source[span.start..span.end], "unknown(step)");
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
    let call = generate(SOURCE).unwrap();
    assert_eq!(call.capture_name, "score");
    let program = compile(&parse_source(SOURCE).unwrap()).unwrap();
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let functions = [vm.run(&program).unwrap(), vm.run(&program).unwrap()];
    let calls = [
        (0, 1.0),
        (0, 3.0),
        (1, 9.0),
        (0, -2.0),
        (1, -0.0),
        (1, f64::INFINITY),
        (1, f64::NEG_INFINITY),
    ];
    let mut expected = String::new();
    let mut main = format!(
        "{}\nfn main() {{\nlet mut captures = [f64::from_bits({}); 2];\n",
        call.rust,
        call.initial_capture.to_bits()
    );
    use std::fmt::Write;
    for (slot, step) in calls {
        let interpreted = vm
            .call_function(&program, functions[slot], &[Value::number(step)])
            .unwrap()
            .as_number()
            .unwrap();
        writeln!(expected, "{:016x}", interpreted.to_bits()).unwrap();
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
    assert_eq!(String::from_utf8(run.stdout).unwrap(), expected);
}
