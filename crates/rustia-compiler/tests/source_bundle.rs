use rustia_compiler::source_bundle::{
    resolve_relative_module, SourceBundle, MAX_STATIC_MODULE_DEPTH,
};

fn bundle(sources: &[(&str, &str)]) -> SourceBundle {
    SourceBundle::new(
        sources
            .iter()
            .map(|(name, source)| (*name, source.as_bytes())),
    )
    .expect("valid source bundle")
}

#[test]
fn bundle_and_graph_identity_ignore_input_and_entry_order() {
    let left = bundle(&[
        ("root.luau", "local leaf = require('./leaf')\nreturn leaf"),
        ("leaf.luau", "return 1"),
    ]);
    let right = bundle(&[
        ("leaf.luau", "return 1"),
        ("root.luau", "local leaf = require('./leaf')\nreturn leaf"),
    ]);
    assert_eq!(left.digest(), right.digest());
    assert_eq!(
        left.static_module_graph(&["root.luau"]).unwrap(),
        right
            .static_module_graph(&["root.luau", "root.luau"])
            .unwrap()
    );
    assert_eq!(left.digest().to_string().len(), 64);
}

#[test]
fn bundle_digest_preserves_the_existing_live_source_identity() {
    let sources = bundle(&[("b.luau", "return 22"), ("a.luau", "return 1")]);
    assert_eq!(
        sources.digest().to_string(),
        "3f5c1f81d4e57fe5a2cd6a79b20499aa5a5d8bb63bfc81f444683f80fd4eca88"
    );
}

#[test]
fn unreachable_invalid_source_is_fingerprinted_but_not_parsed() {
    let valid = bundle(&[("main.luau", "return 1")]);
    let with_unrelated = bundle(&[
        ("main.luau", "return 1"),
        (
            "unrelated.luau",
            "invalid Luau does not block this callback",
        ),
    ]);
    assert_ne!(valid.digest(), with_unrelated.digest());
    assert!(with_unrelated.static_module_graph(&["main.luau"]).is_ok());

    let imported = bundle(&[
        (
            "main.luau",
            "local unrelated = require('./unrelated')\nreturn unrelated",
        ),
        (
            "unrelated.luau",
            "invalid Luau does not block this callback",
        ),
    ]);
    let error = imported.static_module_graph(&["main.luau"]).unwrap_err();
    assert_eq!(error.module.as_deref(), Some("unrelated.luau"));
    assert!(error.span.is_some());
}

#[test]
fn graph_is_dependency_first_with_nested_and_shared_imports() {
    let sources = bundle(&[
        (
            "game/main.luau",
            "local left = require('./left')\nlocal right = require('./right')\nreturn left + right",
        ),
        (
            "game/left.luau",
            "local shared = require('./shared')\nreturn shared",
        ),
        (
            "game/right.luau",
            "local shared = require('./shared')\nreturn shared",
        ),
        ("game/shared.luau", "return 1"),
    ]);
    let graph = sources.static_module_graph(&["game/main.luau"]).unwrap();
    assert_eq!(
        graph.module_names().collect::<Vec<_>>(),
        [
            "game/shared.luau",
            "game/left.luau",
            "game/right.luau",
            "game/main.luau"
        ]
    );
    let main = graph.module("game/main.luau").unwrap();
    assert_eq!(main.requires[0].alias.as_deref(), Some("left"));
    assert_eq!(main.requires[0].request, "./left");
    assert_eq!(main.requires[0].target, "game/left.luau");
    assert_eq!(main.requires[1].target, "game/right.luau");
}

#[test]
fn shadowed_require_is_not_a_static_dependency() {
    let sources = bundle(&[
        (
            "variable.luau",
            "local require = require('./real')\nlocal missing = require('./missing')\nreturn require",
        ),
        (
            "function.luau",
            "local function require(_) return 1 end\nrequire('./missing')\nreturn require",
        ),
        ("real.luau", "return 1"),
    ]);

    let variable = sources.static_module_graph(&["variable.luau"]).unwrap();
    assert_eq!(
        variable.module_names().collect::<Vec<_>>(),
        ["real.luau", "variable.luau"]
    );
    assert_eq!(variable.module("variable.luau").unwrap().requires.len(), 1);

    let function = sources.static_module_graph(&["function.luau"]).unwrap();
    assert_eq!(
        function.module_names().collect::<Vec<_>>(),
        ["function.luau"]
    );
}

#[test]
fn depth_limit_checks_later_paths_through_shared_subgraphs() {
    let shared_depth = 3;
    let chain_depth = MAX_STATIC_MODULE_DEPTH - shared_depth;
    let mut sources = Vec::new();
    sources.push((
        "root.luau".to_owned(),
        b"require('./shared-0')\nrequire('./chain-0')".to_vec(),
    ));
    for index in 0..shared_depth {
        let source = if index + 1 == shared_depth {
            b"return 1".to_vec()
        } else {
            format!("require('./shared-{}')", index + 1).into_bytes()
        };
        sources.push((format!("shared-{index}.luau"), source));
    }
    for index in 0..chain_depth {
        let target = if index + 1 == chain_depth {
            "shared-0".to_owned()
        } else {
            format!("chain-{}", index + 1)
        };
        sources.push((
            format!("chain-{index}.luau"),
            format!("require('./{target}')").into_bytes(),
        ));
    }

    let sources = SourceBundle::new(sources).unwrap();
    let error = sources.static_module_graph(&["root.luau"]).unwrap_err();
    assert_eq!(
        error.module.as_deref(),
        Some(format!("chain-{}.luau", chain_depth - 1).as_str())
    );
    assert!(error.message.contains(&MAX_STATIC_MODULE_DEPTH.to_string()));
}

#[test]
fn missing_module_reports_the_importing_source_location() {
    let source = "local missing = require('./missing')\nreturn missing";
    let sources = bundle(&[("main.luau", source)]);
    let error = sources.static_module_graph(&["main.luau"]).unwrap_err();
    assert_eq!(error.module.as_deref(), Some("main.luau"));
    let span = error.span.unwrap();
    assert_eq!(&source[span.start..span.end], "'./missing'");
    assert!(error
        .message
        .contains("missing static module \"missing.luau\""));
}

#[test]
fn cycle_reports_the_closing_edge_and_deterministic_chain() {
    let b = "local a = require('./a')\nreturn a";
    let sources = bundle(&[
        ("a.luau", "local b = require('./b')\nreturn b"),
        ("b.luau", b),
    ]);
    let error = sources.static_module_graph(&["a.luau"]).unwrap_err();
    assert_eq!(error.module.as_deref(), Some("b.luau"));
    let span = error.span.unwrap();
    assert_eq!(&b[span.start..span.end], "'./a'");
    assert_eq!(
        error.message,
        "cyclic static require: a.luau -> b.luau -> a.luau"
    );
}

#[test]
fn parse_errors_keep_the_module_and_byte_span() {
    let sources = bundle(&[("broken.luau", "local =")]);
    let error = sources.static_module_graph(&["broken.luau"]).unwrap_err();
    assert_eq!(error.module.as_deref(), Some("broken.luau"));
    assert!(error.span.is_some());
}

#[test]
fn custom_resolver_keeps_host_policy_explicit() {
    let sources = bundle(&[
        (
            "main.luau",
            "local shared = require('@shared')\nreturn shared",
        ),
        ("vendor/shared.luau", "return 1"),
    ]);
    assert!(sources.static_module_graph(&["main.luau"]).is_err());
    let graph = sources
        .static_module_graph_with(&["main.luau"], |_, request| match request {
            "@shared" => Ok("vendor/shared.luau".to_owned()),
            _ => Err("unknown package import"),
        })
        .unwrap();
    assert_eq!(
        graph.module_names().collect::<Vec<_>>(),
        ["vendor/shared.luau", "main.luau"]
    );
}

#[test]
fn graph_digest_changes_when_the_resolved_edge_changes() {
    let sources = bundle(&[
        ("main.luau", "local value = require('./selected')"),
        ("one.luau", "return 1"),
        ("two.luau", "return 2"),
    ]);
    let one = sources
        .static_module_graph_with(&["main.luau"], |_, _| {
            Ok::<_, &'static str>("one.luau".to_owned())
        })
        .unwrap();
    let two = sources
        .static_module_graph_with(&["main.luau"], |_, _| {
            Ok::<_, &'static str>("two.luau".to_owned())
        })
        .unwrap();
    assert_ne!(one.digest, two.digest);
    assert_eq!(one.source_digest, two.source_digest);
}

#[test]
fn default_resolver_rejects_escape_and_normalizes_extension() {
    assert_eq!(
        resolve_relative_module("folder/main.luau", "../shared").unwrap(),
        "shared.luau"
    );
    assert!(resolve_relative_module("main.luau", "../escape").is_err());
}

#[test]
fn bundle_rejects_duplicate_and_noncanonical_keys() {
    assert!(SourceBundle::new([("a.luau", b"a"), ("a.luau", b"b")]).is_err());
    for name in [
        "../a.luau",
        "a//b.luau",
        "./b.luau",
        "a\\b.luau",
        ".luau",
        "a.lua",
    ] {
        assert!(SourceBundle::new([(name, Vec::new())]).is_err(), "{name}");
    }
}
