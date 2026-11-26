use rustia_compiler::source_bundle::SourceBundle;

fn bundle(sources: &[(&str, &str)]) -> SourceBundle {
    SourceBundle::new(
        sources
            .iter()
            .map(|(name, source)| (*name, source.as_bytes())),
    )
    .expect("valid source bundle")
}

#[test]
fn admits_nested_stateless_functions_in_dependency_order() {
    let sources = bundle(&[
        (
            "helpers/adjust.luau",
            "local scale = require('./scale')\nlocal offset = 2\nreturn function(n) return scale(n) + offset end",
        ),
        (
            "helpers/scale.luau",
            "return function(n) return math.abs(n) * 3 end",
        ),
    ]);
    let graph = sources
        .stateless_function_graph(&["helpers/adjust.luau"])
        .unwrap();
    assert_eq!(
        graph.module_names().collect::<Vec<_>>(),
        ["helpers/scale.luau", "helpers/adjust.luau"]
    );
}

#[test]
fn rejects_module_state_and_host_effects_with_module_identity() {
    for source in [
        "local counter = 0\nreturn function() counter += 1 return counter end",
        "return function(value) return value:host_effect() end",
        "local state = {}\nreturn function() return state end",
    ] {
        let sources = bundle(&[("stateful.luau", source)]);
        let error = sources
            .stateless_function_graph(&["stateful.luau"])
            .unwrap_err();
        assert_eq!(error.module.as_deref(), Some("stateful.luau"));
        assert!(
            error.message.contains("stateless helper"),
            "{source}: {error}"
        );
    }
}

#[test]
fn rejects_shadowed_builtins_and_indirect_calls() {
    for source in [
        "local math = require('./pure')\nreturn function(n) return math(n) end",
        "return function(call) return call() end",
    ] {
        let sources = bundle(&[
            ("helper.luau", source),
            ("pure.luau", "return function(n) return n end"),
        ]);
        assert!(
            sources.stateless_function_graph(&["helper.luau"]).is_err(),
            "{source}"
        );
    }
}

#[test]
fn custom_resolution_and_graph_errors_still_precede_admission() {
    let sources = bundle(&[
        (
            "helper.luau",
            "local pure = require('@pure')\nreturn function(n) return pure(n) end",
        ),
        ("vendor/pure.luau", "return function(n) return n end"),
    ]);
    assert!(sources.stateless_function_graph(&["helper.luau"]).is_err());
    let graph = sources
        .stateless_function_graph_with(&["helper.luau"], |_, request| match request {
            "@pure" => Ok("vendor/pure.luau".to_owned()),
            _ => Err("unknown helper import"),
        })
        .unwrap();
    assert_eq!(graph.entries, ["helper.luau"]);
}

#[test]
fn admits_explicitly_typed_import_calls() {
    let sources = bundle(&[
        (
            "helper.luau",
            "local id = require<<any>>('./id')\nreturn function(n) return id<<number>>(n) end",
        ),
        ("id.luau", "return function(value) return value end"),
    ]);

    let graph = sources.stateless_function_graph(&["helper.luau"]).unwrap();
    assert_eq!(
        graph.module_names().collect::<Vec<_>>(),
        ["id.luau", "helper.luau"]
    );
}
