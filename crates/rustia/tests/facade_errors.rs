//! Error contracts exercised only through the embedding facade.

use rustia::{
    DiagnosticLimits, Error, ErrorKind, Profile, Rustia, RustiaBuilder, SourceError, SourceSpan,
    VmError, VmErrorKind,
};
use std::error::Error as _;

#[test]
fn frontend_errors_keep_phase_message_and_source_range() {
    let cases = [
        ("lex.luau", "return 'unfinished", "lex"),
        ("parse.luau", "local = 1", "parse"),
        ("compile.luau", "break", "compile"),
    ];
    for (name, source, phase) in cases {
        let error = Rustia::compile_module(name, source).err().unwrap();
        let span = error.source_span().expect("frontend byte range");
        assert!(span.start <= span.end && span.end <= source.len());
        let (actual_phase, payload_span, message) = match error.kind() {
            ErrorKind::Source(SourceError::Lex(error)) => ("lex", Some(error.span), &error.message),
            ErrorKind::Source(SourceError::Parse(error)) => {
                ("parse", Some(error.span), &error.message)
            }
            ErrorKind::Compile(error) => ("compile", error.span, &error.message),
            other => panic!("unexpected frontend failure: {other:?}"),
        };
        assert_eq!(actual_phase, phase);
        assert_eq!(payload_span, Some(span));
        assert!(error.to_string().ends_with(message));
        assert_eq!(error.source_location().unwrap().source, name);
        assert!(error.report().to_string().contains(source));
        assert!(error.source().is_some());
    }
}

#[test]
fn runtime_errors_keep_exact_message_and_suppressed_location() {
    let mut vm = RustiaBuilder::standard().build().unwrap();
    let error = vm
        .load("error('unchanged message', 0)")
        .source_name("hidden.luau")
        .exec()
        .unwrap_err();
    let runtime = error.as_vm_error().unwrap();
    assert!(
        matches!(runtime.kind(), VmErrorKind::RuntimeError { message } if message == "unchanged message")
    );
    assert_eq!(runtime.message().to_string(), "unchanged message");
    assert_eq!(error.to_string(), "unchanged message");
    assert!(runtime.source_location().is_none());
    assert!(runtime.source_span().is_none());
    assert!(error.diagnostic().is_none());
}

#[test]
fn source_snapshots_remain_budgeted_until_outstanding_error_drops() {
    let mut vm = RustiaBuilder::standard().build().unwrap();
    let source = "return missing.health";
    let error = vm
        .load(source)
        .source_name("health.luau")
        .exec()
        .unwrap_err();
    let snapshot = error.diagnostic().unwrap().snapshot.clone().unwrap();
    assert_eq!(snapshot.bytes(), source.as_bytes());
    assert_eq!(error.source_span(), Some(SourceSpan { start: 7, end: 21 }));
    assert_eq!(
        error.as_vm_error().unwrap().source_span(),
        error.source_span()
    );
    let report = error.report().to_string();
    let charged = vm.diagnostic_source_bytes();
    assert!(charged >= source.len());
    vm.reset_decision().unwrap();
    assert_eq!(vm.diagnostic_source_bytes(), charged);
    assert_eq!(error.report().to_string(), report);
    drop(error);
    assert_eq!(vm.diagnostic_source_bytes(), charged);
    drop(snapshot);
    assert_eq!(vm.diagnostic_source_bytes(), 0);
}

#[test]
fn diagnostic_budgets_do_not_change_protected_errors() {
    fn run(limits: DiagnosticLimits) -> (bool, String) {
        let mut vm = RustiaBuilder::standard()
            .diagnostic_limits(limits)
            .build()
            .unwrap();
        vm.load("local ok, message = pcall(function() error('same', 0) end); return {ok, message}")
            .eval_serde()
            .unwrap()
    }
    assert_eq!(
        run(DiagnosticLimits::default()),
        run(DiagnosticLimits::new()
            .with_retained_source_bytes(0)
            .with_source_bytes(0)
            .with_frames(0)
            .with_report_bytes(0)),
    );
}

#[test]
fn typed_host_failures_keep_hard_abort_and_original_cause() {
    let mut vm = RustiaBuilder::standard().build().unwrap();
    vm.register_function("stop", || -> rustia::Result<()> {
        Err(Error::from(VmError::new(VmErrorKind::FuelExhausted {
            limit: 17,
        })))
    })
    .unwrap();
    let error = vm.exec("pcall(stop); error('must not run')").unwrap_err();
    assert!(matches!(
        error.as_vm_error().unwrap().kind(),
        VmErrorKind::FuelExhausted { limit: 17 }
    ));

    let mut vm = RustiaBuilder::standard().build().unwrap();
    vm.register_function("asset", || -> rustia::Result<()> {
        Err(Error::external(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "missing asset",
        )))
    })
    .unwrap();
    let error = vm
        .load("asset()")
        .source_name("asset.luau")
        .exec()
        .unwrap_err();
    let mut source = error.source();
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            assert_eq!(io.kind(), std::io::ErrorKind::NotFound);
            return;
        }
        source = cause.source();
    }
    panic!("original host error was lost");
}

#[test]
fn profile_rejection_payload_is_a_facade_profile() {
    let error = Error::from(VmError::new(VmErrorKind::CapabilityRemoved {
        feature: "test operation",
        operation: Some("test".to_owned()),
        profile: Profile::DeterministicSim,
    }));
    assert!(matches!(
        error.as_vm_error().unwrap().kind(),
        VmErrorKind::CapabilityRemoved {
            profile: Profile::DeterministicSim,
            ..
        }
    ));
    assert_eq!(
        error.to_string(),
        "test operation `test` is not available in DeterministicSim"
    );
}

#[test]
fn captured_guest_errors_cross_host_boundaries_as_messages() {
    let mut original = RustiaBuilder::standard().build().unwrap();
    let error = original.exec("token = {}; error(token, 0)").unwrap_err();
    let captured = error.as_vm_error().unwrap().clone();
    assert!(matches!(captured.kind(), VmErrorKind::GuestError { message } if message == "table"));
    let error_debug = format!("{captured:?}");
    assert!(!error_debug.contains("value: Value"));
    original.reset_decision().unwrap();

    for mut vm in [original, RustiaBuilder::standard().build().unwrap()] {
        let captured = captured.clone();
        vm.register_function("rethrow", move || -> rustia::Result<()> {
            Err(Error::from(captured.clone()))
        })
        .unwrap();
        assert!(vm.eval::<bool>(
            "local ok, message = pcall(rethrow); return not ok and type(message) == 'string' and message == 'table'"
        ).unwrap());
        assert!(vm.eval::<bool>(
            "local token = {}; local ok, caught = pcall(function() error(token) end); return not ok and rawequal(token, caught)"
        ).unwrap());
    }
}
