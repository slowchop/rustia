use rustia_compiler::{compile, compiled_call::generate};
use rustia_parser::parse_source;
use rustia_vm::{Profile, Value, Vm};
use std::{fs, process::Command};

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
    let directory =
        std::env::temp_dir().join(format!("rustia-compiled-call-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
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
