use rustia_conformance::{
    list_repository_pending, repository_from_crate_manifest, run_repository, run_repository_case,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::thread;

#[test]
fn repository_conformance_manifest_passes() {
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let report = thread::Builder::new()
        .name("rustia-conformance-manifest".to_owned())
        .stack_size(128 * 1024 * 1024)
        .spawn(move || run_repository(repo).unwrap())
        .unwrap()
        .join()
        .unwrap();
    assert!(report.refs > 0);
    assert!(report.fixtures > 0);
}

#[test]
fn upstream_luau_0709_inventory_is_complete() {
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let upstream_root = repo.join("conformance/upstream/luau-0.709/tests/conformance");
    let mut copied = BTreeSet::new();
    for entry in fs::read_dir(upstream_root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "luau")
        {
            copied.insert(repo_relative(&repo, &path));
        }
    }
    assert_eq!(copied.len(), 49);

    let manifest = fs::read_to_string(repo.join("conformance/MANIFEST.tsv")).unwrap();
    let manifest_paths = manifest
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split('\t').nth(6))
        .filter(|path| path.starts_with("conformance/upstream/luau-0.709/"))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    assert_eq!(manifest_paths, copied);

    let provenance =
        fs::read_to_string(repo.join("conformance/upstream/luau-0.709/PROVENANCE.tsv")).unwrap();
    let mut provenance_paths = BTreeSet::new();
    for (line_number, line) in provenance.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 7, "bad provenance line {}", line_number + 1);
        assert_luau_0709_provenance_common(&fields, line_number + 1);
        assert_valid_upstream_conformance_source(fields[2], line_number + 1);
        assert_valid_local_provenance_path(
            &repo,
            fields[3],
            "conformance/upstream/luau-0.709/tests/conformance/",
            line_number + 1,
        );
        assert_valid_oracle_command(fields[6], line_number + 1);
        provenance_paths.insert(fields[3].to_owned());
    }
    assert_eq!(provenance_paths, copied);
}

#[test]
fn p1_smoke_inventory_is_manifested() {
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let p1_root = repo.join("conformance/fixtures/p1-smoke");
    let mut copied = BTreeSet::new();
    for entry in fs::read_dir(p1_root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "luau")
        {
            copied.insert(repo_relative(&repo, &path));
        }
    }
    assert!(!copied.is_empty());

    let manifest = fs::read_to_string(repo.join("conformance/MANIFEST.tsv")).unwrap();
    let manifest_paths = manifest
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split('\t').nth(6))
        .filter(|path| path.starts_with("conformance/fixtures/p1-smoke/"))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    assert_eq!(manifest_paths, copied);
}

#[test]
fn coverage_current_status_counts_match_manifest() {
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let counts = coverage_counts(&repo);
    let coverage = fs::read_to_string(repo.join("conformance/COVERAGE.md")).unwrap();
    let normalized_coverage = coverage.split_whitespace().collect::<Vec<_>>().join(" ");
    let expected_status = format!(
        "The current manifest contains {} fixtures and {} refs: {} executable `luau-0.709` fixtures, {} `pending-oracle` P1 smoke fixtures, and {} whole upstream Luau 0.709 conformance scripts as `pending-impl` inventory.",
        counts.fixtures,
        counts.refs,
        counts.luau_0709,
        counts.pending_oracle,
        counts.pending_impl
    );

    assert!(
        normalized_coverage.contains(&expected_status),
        "conformance/COVERAGE.md Current Status count summary is stale; expected to find: {expected_status}"
    );
    assert_eq!(
        counts.rustia_only, 0,
        "conformance/COVERAGE.md Current Status must describe rustia-only rows before adding them"
    );
}

#[test]
fn luau_ref_catalog_rows_are_backed_by_luau_oracle_fixtures() {
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let complete_refs = luau_0709_ref_catalog_refs(&repo);
    let covered_refs = luau_0709_manifest_refs(&repo);
    let missing = complete_refs
        .difference(&covered_refs)
        .map(String::as_str)
        .collect::<Vec<_>>();

    assert!(
        missing.is_empty(),
        "luau-0.709 refs must have at least one luau-0.709 manifest fixture; missing: {missing:?}"
    );
}

#[test]
fn ref_catalog_rows_are_used_by_manifest() {
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let catalog_refs = all_ref_catalog_refs(&repo);
    let manifest_refs = all_manifest_refs(&repo);
    let unused = catalog_refs
        .difference(&manifest_refs)
        .map(String::as_str)
        .collect::<Vec<_>>();

    assert!(
        unused.is_empty(),
        "conformance/REFS.tsv rows must be used by at least one manifest fixture; unused: {unused:?}"
    );
}

#[test]
fn refs_catalog_sources_are_local_or_explicit_upstream() {
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let refs = fs::read_to_string(repo.join("conformance/REFS.tsv")).unwrap();

    for (line_number, line) in refs.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 5, "bad refs line {}", line_number + 1);
        let source = fields[2];
        if let Some(path) = source.strip_prefix("upstream-luau-0.709:") {
            assert_safe_slash_path(path, line_number + 1);
        } else if source.starts_with("conformance/") || source.starts_with("design/") {
            let path = repo.join(source);
            assert!(
                path.exists(),
                "refs line {} points at missing local source {:?}",
                line_number + 1,
                source
            );
        } else {
            panic!(
                "refs line {} has unclassified source {:?}",
                line_number + 1,
                source
            );
        }
    }
}

#[test]
fn ref_catalog_status_matches_source_evidence() {
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let refs = fs::read_to_string(repo.join("conformance/REFS.tsv")).unwrap();

    for (line_number, line) in refs.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 5, "bad refs line {}", line_number + 1);
        let reference = fields[0];
        let source = fields[2];
        let status = fields[3];

        match status {
            "luau-0.709" => assert!(
                source.starts_with("conformance/upstream/luau-0.709/")
                    || source.starts_with("conformance/fixtures/upstream-luau-0.709/")
                    || source.starts_with("upstream-luau-0.709:"),
                "luau-0.709 ref {reference:?} must cite pinned upstream evidence, got {source:?}"
            ),
            "pending-oracle" => assert_eq!(
                source, "design/50-conformance.md",
                "pending-oracle ref {reference:?} must remain design-sourced until a pinned oracle is recorded"
            ),
            "pending-impl" => assert!(
                source.starts_with("conformance/upstream/luau-0.709/tests/conformance/")
                    && source.ends_with(".luau"),
                "pending-impl ref {reference:?} must cite the unexecuted upstream script, got {source:?}"
            ),
            "pending-fixture" => assert!(
                source.starts_with("design/"),
                "pending-fixture ref {reference:?} must cite design scope until a fixture exists"
            ),
            "rustia-only" => assert!(
                source.starts_with("conformance/fixtures/rustia-only/")
                    || source.starts_with("design/"),
                "rustia-only ref {reference:?} must cite a rustia-only fixture or design source, got {source:?}"
            ),
            status => panic!("refs line {} has unknown status {status:?}", line_number + 1),
        }
    }
}

#[test]
fn pending_manifest_rows_match_explicit_allowlist() {
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let pending = list_repository_pending(&repo).unwrap();
    let actual = pending
        .iter()
        .map(|case| {
            format!(
                "{}\t{}\t{}\t{}",
                case.oracle,
                case.id,
                case.path,
                case.refs.join(",")
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        actual,
        Vec::<String>::new()
    );
}

#[test]
fn pending_ref_catalog_rows_match_explicit_allowlist() {
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let refs = fs::read_to_string(repo.join("conformance/REFS.tsv")).unwrap();
    let mut actual = Vec::new();

    for (line_number, line) in refs.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 5, "bad refs line {}", line_number + 1);
        if fields[3].starts_with("pending-") {
            actual.push(format!("{}\t{}\t{}", fields[3], fields[0], fields[2]));
        }
    }

    assert_eq!(
        actual,
        Vec::<String>::new()
    );
}

#[test]
fn pending_impl_inventory_is_empty() {
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let pending_impl = list_repository_pending(&repo)
        .unwrap()
        .into_iter()
        .filter(|case| case.oracle == "pending-impl")
        .map(|case| case.id)
        .collect::<Vec<_>>();

    assert!(
        pending_impl.is_empty(),
        "new pending-impl rows need an explicit forced-failure allowlist: {pending_impl:?}"
    );
}

#[test]
fn upstream_luau_0709_derived_fixtures_have_provenance() {
    let repo = repository_from_crate_manifest(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let derived_root = repo.join("conformance/fixtures/upstream-luau-0.709");
    let mut copied = BTreeSet::new();
    for entry in fs::read_dir(derived_root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "luau")
        {
            copied.insert(repo_relative(&repo, &path));
        }
    }
    assert!(!copied.is_empty());

    let manifest = fs::read_to_string(repo.join("conformance/MANIFEST.tsv")).unwrap();
    let manifest_paths = manifest
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split('\t').nth(6))
        .filter(|path| path.starts_with("conformance/fixtures/upstream-luau-0.709/"))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    assert_eq!(manifest_paths, copied);

    let provenance =
        fs::read_to_string(repo.join("conformance/fixtures/upstream-luau-0.709/PROVENANCE.tsv"))
            .unwrap();
    let mut provenance_paths = BTreeSet::new();
    for (line_number, line) in provenance.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 7, "bad provenance line {}", line_number + 1);
        assert_luau_0709_provenance_common(&fields, line_number + 1);
        assert_valid_upstream_source_path(fields[2], line_number + 1);
        assert_valid_local_provenance_path(
            &repo,
            fields[3],
            "conformance/fixtures/upstream-luau-0.709/",
            line_number + 1,
        );
        assert_valid_oracle_command(fields[6], line_number + 1);
        if fields[6].starts_with("source-inspection:") {
            assert!(
                fields[2].contains("VM/src/") || fields[2] == "tests/Conformance.test.cpp",
                "source-inspection provenance line {} must cite upstream source",
                line_number + 1
            );
        }
        provenance_paths.insert(fields[3].to_owned());
    }
    assert_eq!(provenance_paths, copied);
}

#[test]
fn pass_expectation_accepts_successful_scripts() {
    let repo = FixtureRepo::new("pass");
    repo.write_ref("upstream.basic\truntime\tupstream\tluau-0.709\tBasic upstream fixture.");
    repo.write_fixture("ok.luau", "return 42");
    repo.write_manifest(
        "upstream.basic\truntime\tupstream.basic\tDeterministicSim\tluau-0.709\tpass\tconformance/fixtures/ok.luau",
    );

    let report = run_repository(repo.path()).unwrap();
    assert_eq!(report.fixtures, 1);
    assert_eq!(report.luau_oracle, 1);
}

#[test]
fn string_exact_expectation_checks_returned_bytes() {
    let repo = FixtureRepo::new("string-exact");
    repo.write_ref("upstream.string\truntime\tupstream\tluau-0.709\tString upstream fixture.");
    repo.write_fixture("ok.luau", "return 'ok'");
    repo.write_manifest(
        "upstream.string\truntime\tupstream.string\tDeterministicSim\tluau-0.709\tstring:ok\tconformance/fixtures/ok.luau",
    );

    let report = run_repository(repo.path()).unwrap();
    assert_eq!(report.fixtures, 1);
    assert_eq!(report.luau_oracle, 1);
}

#[test]
fn string_hex_expectation_checks_returned_bytes() {
    let repo = FixtureRepo::new("string-hex");
    repo.write_ref("upstream.string\truntime\tupstream\tluau-0.709\tString upstream fixture.");
    repo.write_fixture("nul.luau", r#"return "a\0b""#);
    repo.write_manifest(
        "upstream.string\truntime\tupstream.string\tDeterministicSim\tluau-0.709\tstring_hex:610062\tconformance/fixtures/nul.luau",
    );

    let report = run_repository(repo.path()).unwrap();
    assert_eq!(report.fixtures, 1);
    assert_eq!(report.luau_oracle, 1);
}

#[test]
fn non_utf8_source_fixtures_execute_as_luau_bytes() {
    let repo = FixtureRepo::new("non-utf8-source");
    repo.write_ref("upstream.string\truntime\tupstream\tluau-0.709\tString upstream fixture.");
    repo.write_fixture_bytes("bytes.luau", b"return 'a\xffb'");
    repo.write_manifest(
        "upstream.string\truntime\tupstream.string\tReproducibleStandard\tluau-0.709\tstring_hex:61ff62\tconformance/fixtures/bytes.luau",
    );

    let report = run_repository(repo.path()).unwrap();
    assert_eq!(report.fixtures, 1);
    assert_eq!(report.luau_oracle, 1);
}

#[test]
fn luau_oracle_entries_reject_pending_refs() {
    let repo = FixtureRepo::new("luau-row-pending-ref");
    repo.write_ref("topic.pending\truntime\tupstream\tpending-oracle\tPending topic.");
    repo.write_fixture("ok.luau", "return 42");
    repo.write_manifest(
        "case.pending-ref\truntime\ttopic.pending\tDeterministicSim\tluau-0.709\tnumber:42\tconformance/fixtures/ok.luau",
    );

    let error = run_repository(repo.path()).unwrap_err().to_string();
    assert!(error.contains("luau-0.709 manifest row references"));
    assert!(error.contains("pending-oracle status"));
}

#[test]
fn pending_oracle_entries_may_reference_pending_refs() {
    let repo = FixtureRepo::new("pending-row-pending-ref");
    repo.write_ref("topic.pending\truntime\tupstream\tpending-oracle\tPending topic.");
    repo.write_fixture("ok.luau", "return 42");
    repo.write_manifest(
        "case.pending-ref\truntime\ttopic.pending\tDeterministicSim\tpending-oracle\tnumber:42\tconformance/fixtures/ok.luau",
    );

    let report = run_repository(repo.path()).unwrap();
    assert_eq!(report.fixtures, 1);
    assert_eq!(report.pending_oracle, 1);
}

#[test]
fn pending_oracle_entries_reject_fully_oracled_refs() {
    let repo = FixtureRepo::new("pending-row-complete-ref");
    repo.write_ref("topic.complete\truntime\tupstream\tluau-0.709\tComplete topic.");
    repo.write_fixture("ok.luau", "return 42");
    repo.write_manifest(
        "case.stale-pending\truntime\ttopic.complete\tDeterministicSim\tpending-oracle\tnumber:42\tconformance/fixtures/ok.luau",
    );

    let error = run_repository(repo.path()).unwrap_err().to_string();
    assert!(error.contains("pending-oracle manifest rows must reference"));
}

#[test]
fn pending_oracle_entries_reject_mixed_pending_status_refs() {
    let repo = FixtureRepo::new("pending-row-mixed-ref");
    repo.write_refs(&[
        "topic.oracle\truntime\tupstream\tpending-oracle\tPending oracle topic.",
        "topic.impl\truntime\tupstream\tpending-impl\tPending implementation topic.",
    ]);
    repo.write_fixture("ok.luau", "return 42");
    repo.write_manifest(
        "case.pending-mixed\truntime\ttopic.oracle,topic.impl\tDeterministicSim\tpending-oracle\tnumber:42\tconformance/fixtures/ok.luau",
    );

    let error = run_repository(repo.path()).unwrap_err().to_string();
    assert!(error.contains("pending-oracle manifest rows must reference"));
    assert!(error.contains("pending-impl status"));
}

#[test]
fn rustia_only_entries_accept_rustia_only_refs() {
    let repo = FixtureRepo::new("rustia-row-rustia-ref");
    repo.write_ref(
        "rustia.determinism\tdeterminism\trustia\trustia-only\tRustia-specific deterministic invariant.",
    );
    repo.write_fixture("ok.luau", "return 42");
    repo.write_manifest(
        "case.rustia\tdeterminism\trustia.determinism\tDeterministicSim\trustia-only\tnumber:42\tconformance/fixtures/ok.luau",
    );

    let report = run_repository(repo.path()).unwrap();
    assert_eq!(report.fixtures, 1);
    assert_eq!(report.rustia_only, 1);
}

#[test]
fn rustia_only_entries_reject_non_rustia_only_refs() {
    let repo = FixtureRepo::new("rustia-row-luau-ref");
    repo.write_ref(
        "rustia.determinism\tdeterminism\trustia\tluau-0.709\tIncorrectly complete ref.",
    );
    repo.write_fixture("ok.luau", "return 42");
    repo.write_manifest(
        "case.rustia\tdeterminism\trustia.determinism\tDeterministicSim\trustia-only\tnumber:42\tconformance/fixtures/ok.luau",
    );

    let error = run_repository(repo.path()).unwrap_err().to_string();
    assert!(error.contains("rustia-only manifest rows must reference"));
}

#[test]
fn luau_oracle_entries_reject_rustia_only_refs() {
    let repo = FixtureRepo::new("luau-row-rustia-ref");
    repo.write_ref(
        "rustia.determinism\tdeterminism\trustia\trustia-only\tRustia-specific deterministic invariant.",
    );
    repo.write_fixture("ok.luau", "return 42");
    repo.write_manifest(
        "case.luau\tdeterminism\trustia.determinism\tDeterministicSim\tluau-0.709\tnumber:42\tconformance/fixtures/ok.luau",
    );

    let error = run_repository(repo.path()).unwrap_err().to_string();
    assert!(error.contains("luau-0.709 manifest row references"));
    assert!(error.contains("rustia-only status"));
}

#[test]
fn conformance_loadstring_compiles_dynamic_source_expressions() {
    let repo = FixtureRepo::new("dynamic-loadstring");
    repo.write_ref(
        "stdlib.loadstring-dynamic\tstdlib\tupstream\tluau-0.709\tloadstring compiles dynamic source strings in conformance mode.",
    );
    repo.write_fixture(
        "dynamic-loadstring.luau",
        "local source = 'return 40 + 2'\nreturn assert(loadstring(source))()",
    );
    repo.write_manifest(
        "stdlib.loadstring-dynamic\tstdlib\tstdlib.loadstring-dynamic\tReproducibleStandard\tluau-0.709\tnumber:42\tconformance/fixtures/dynamic-loadstring.luau",
    );

    let report = run_repository(repo.path()).unwrap();
    assert_eq!(report.fixtures, 1);
    assert_eq!(report.luau_oracle, 1);
}

#[test]
fn pending_impl_entries_are_counted_without_execution() {
    let repo = FixtureRepo::new("pending-impl");
    repo.write_ref("upstream.basic\truntime\tupstream\tpending-impl\tBasic upstream fixture.");
    repo.write_fixture_bytes(
        "pending.luau",
        b"\xff this is intentionally not valid UTF-8 Luau",
    );
    repo.write_manifest(
        "upstream.basic\truntime\tupstream.basic\tDeterministicSim\tpending-impl\tpass\tconformance/fixtures/pending.luau",
    );

    let report = run_repository(repo.path()).unwrap();
    assert_eq!(report.fixtures, 1);
    assert_eq!(report.pending_impl, 1);
}

#[test]
fn pending_impl_entries_reject_fully_oracled_refs() {
    let repo = FixtureRepo::new("pending-impl-complete-ref");
    repo.write_ref("upstream.basic\truntime\tupstream\tluau-0.709\tBasic upstream fixture.");
    repo.write_fixture_bytes(
        "pending.luau",
        b"\xff this is intentionally not valid UTF-8 Luau",
    );
    repo.write_manifest(
        "upstream.basic\truntime\tupstream.basic\tDeterministicSim\tpending-impl\tpass\tconformance/fixtures/pending.luau",
    );

    let error = run_repository(repo.path()).unwrap_err().to_string();
    assert!(error.contains("pending-impl manifest rows must reference"));
}

#[test]
fn pending_impl_entries_reject_pending_fixture_refs() {
    let repo = FixtureRepo::new("pending-impl-pending-fixture-ref");
    repo.write_refs(&[
        "topic.impl\truntime\tupstream\tpending-impl\tPending implementation topic.",
        "topic.fixture\truntime\tupstream\tpending-fixture\tDeclared topic without fixture.",
    ]);
    repo.write_fixture_bytes(
        "pending.luau",
        b"\xff this is intentionally not valid UTF-8 Luau",
    );
    repo.write_manifest(
        "case.pending-fixture-ref\truntime\ttopic.impl,topic.fixture\tDeterministicSim\tpending-impl\tpass\tconformance/fixtures/pending.luau",
    );

    let error = run_repository(repo.path()).unwrap_err().to_string();
    assert!(error.contains("pending-impl manifest rows must reference"));
    assert!(error.contains("pending-fixture status"));
}

#[test]
fn pending_listing_reports_pending_without_execution() {
    let repo = FixtureRepo::new("pending-listing");
    repo.write_refs(&[
        "topic.complete\truntime\tupstream\tluau-0.709\tComplete topic.",
        "topic.oracle\truntime\tupstream\tpending-oracle\tPending oracle topic.",
        "topic.impl\truntime\tupstream\tpending-impl\tPending implementation topic.",
    ]);
    repo.write_fixture("complete.luau", "return 42");
    repo.write_fixture_bytes("pending-oracle.luau", b"\xff not valid Luau");
    repo.write_fixture_bytes("pending-impl.luau", b"\xff not valid Luau");
    repo.write_manifest_rows(&[
        "case.complete\truntime\ttopic.complete\tDeterministicSim\tluau-0.709\tnumber:42\tconformance/fixtures/complete.luau",
        "case.pending-oracle\truntime\ttopic.oracle\tDeterministicSim\tpending-oracle\tpass\tconformance/fixtures/pending-oracle.luau",
        "case.pending-impl\truntime\ttopic.impl\tDeterministicSim\tpending-impl\tpass\tconformance/fixtures/pending-impl.luau",
    ]);

    let pending = list_repository_pending(repo.path()).unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].id, "case.pending-oracle");
    assert_eq!(pending[0].oracle, "pending-oracle");
    assert_eq!(pending[0].refs, vec!["topic.oracle".to_owned()]);
    assert_eq!(pending[1].id, "case.pending-impl");
    assert_eq!(pending[1].oracle, "pending-impl");
    assert_eq!(pending[1].refs, vec!["topic.impl".to_owned()]);
}

#[test]
fn pending_listing_validates_pending_metadata() {
    let repo = FixtureRepo::new("pending-listing-invalid-metadata");
    repo.write_refs(&[
        "topic.oracle\truntime\tupstream\tpending-oracle\tPending oracle topic.",
        "topic.fixture\truntime\tupstream\tpending-fixture\tDeclared topic without fixture.",
    ]);
    repo.write_fixture_bytes("pending-oracle.luau", b"\xff not valid Luau");
    repo.write_manifest(
        "case.pending-oracle\truntime\ttopic.oracle,topic.fixture\tDeterministicSim\tpending-oracle\tpass\tconformance/fixtures/pending-oracle.luau",
    );

    let error = list_repository_pending(repo.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("pending-oracle manifest rows must reference"));
    assert!(error.contains("pending-fixture status"));
}

#[test]
fn single_case_runner_force_executes_pending_impl_entries() {
    let repo = FixtureRepo::new("single-case-pending");
    repo.write_ref("upstream.basic\truntime\tupstream\tpending-impl\tBasic upstream fixture.");
    repo.write_fixture("pending.luau", "return 42");
    repo.write_manifest(
        "upstream.basic\truntime\tupstream.basic\tDeterministicSim\tpending-impl\tnumber:42\tconformance/fixtures/pending.luau",
    );

    let report = run_repository(repo.path()).unwrap();
    assert_eq!(report.pending_impl, 1);

    let case = run_repository_case(repo.path(), "upstream.basic").unwrap();
    assert_eq!(case.id, "upstream.basic");
    assert_eq!(case.path, "conformance/fixtures/pending.luau");
}

#[test]
fn manifest_paths_reject_parent_segments() {
    let repo = FixtureRepo::new("manifest-path-parent");
    repo.write_ref("topic.basic\truntime\tupstream\tluau-0.709\tBasic fixture.");
    repo.write_manifest(
        "case.bad-path\truntime\ttopic.basic\tDeterministicSim\tluau-0.709\tnumber:42\tconformance/fixtures/../escape.luau",
    );

    let error = run_repository(repo.path()).unwrap_err().to_string();
    assert!(error.contains("contains an invalid segment"));
}

#[cfg(unix)]
#[test]
fn manifest_paths_reject_symlink_escape() {
    let repo = FixtureRepo::new("manifest-path-symlink");
    repo.write_ref("topic.basic\truntime\tupstream\tluau-0.709\tBasic fixture.");
    fs::write(repo.path().join("outside.luau"), "return 42").unwrap();
    std::os::unix::fs::symlink(
        repo.path().join("outside.luau"),
        repo.path().join("conformance/fixtures/link.luau"),
    )
    .unwrap();
    repo.write_manifest(
        "case.symlink\truntime\ttopic.basic\tDeterministicSim\tluau-0.709\tnumber:42\tconformance/fixtures/link.luau",
    );

    let error = run_repository(repo.path()).unwrap_err().to_string();
    assert!(error.contains("resolves outside conformance"));
}

#[test]
fn manifest_rows_reject_trailing_columns() {
    let repo = FixtureRepo::new("manifest-trailing-column");
    repo.write_ref("topic.basic\truntime\tupstream\tluau-0.709\tBasic fixture.");
    repo.write_fixture("ok.luau", "return 42");
    repo.write_manifest(
        "case.trailing\truntime\ttopic.basic\tDeterministicSim\tluau-0.709\tnumber:42\tconformance/fixtures/ok.luau\t",
    );

    let error = run_repository(repo.path()).unwrap_err().to_string();
    assert!(error.contains("must have 7 tab-separated columns"));
}

#[test]
fn refs_rows_reject_trailing_columns() {
    let repo = FixtureRepo::new("refs-trailing-column");
    repo.write_ref("topic.basic\truntime\tupstream\tluau-0.709\tBasic fixture.\t");
    repo.write_fixture("ok.luau", "return 42");
    repo.write_manifest(
        "case.trailing\truntime\ttopic.basic\tDeterministicSim\tluau-0.709\tnumber:42\tconformance/fixtures/ok.luau",
    );

    let error = run_repository(repo.path()).unwrap_err().to_string();
    assert!(error.contains("must have 5 tab-separated columns"));
}

#[test]
fn manifest_refs_reject_empty_components() {
    let repo = FixtureRepo::new("manifest-empty-ref-component");
    repo.write_ref("topic.basic\truntime\tupstream\tluau-0.709\tBasic fixture.");
    repo.write_fixture("ok.luau", "return 42");
    repo.write_manifest(
        "case.empty-ref\truntime\ttopic.basic,\tDeterministicSim\tluau-0.709\tnumber:42\tconformance/fixtures/ok.luau",
    );

    let error = run_repository(repo.path()).unwrap_err().to_string();
    assert!(error.contains("empty ref component"));
}

#[test]
fn manifest_ids_must_be_unique() {
    let repo = FixtureRepo::new("manifest-duplicate-id");
    repo.write_ref("topic.basic\truntime\tupstream\tluau-0.709\tBasic fixture.");
    repo.write_fixture("first.luau", "return 1");
    repo.write_fixture("second.luau", "return 2");
    repo.write_manifest_rows(&[
        "case.duplicate\truntime\ttopic.basic\tDeterministicSim\tluau-0.709\tnumber:1\tconformance/fixtures/first.luau",
        "case.duplicate\truntime\ttopic.basic\tDeterministicSim\tluau-0.709\tnumber:2\tconformance/fixtures/second.luau",
    ]);

    let error = run_repository(repo.path()).unwrap_err().to_string();
    assert!(error.contains("duplicates id"));
}

#[test]
fn manifest_paths_must_be_unique() {
    let repo = FixtureRepo::new("manifest-duplicate-path");
    repo.write_ref("topic.basic\truntime\tupstream\tluau-0.709\tBasic fixture.");
    repo.write_fixture("shared.luau", "return 42");
    repo.write_manifest_rows(&[
        "case.first\truntime\ttopic.basic\tDeterministicSim\tluau-0.709\tnumber:42\tconformance/fixtures/shared.luau",
        "case.second\truntime\ttopic.basic\tDeterministicSim\tluau-0.709\tnumber:42\tconformance/fixtures/shared.luau",
    ]);

    let error = run_repository_case(repo.path(), "case.first")
        .unwrap_err()
        .to_string();
    assert!(error.contains("duplicates path"));
}

struct FixtureRepo {
    root: PathBuf,
}

impl FixtureRepo {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("rustia-conformance-{name}-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("conformance/fixtures")).unwrap();
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn write_ref(&self, row: &str) {
        self.write_refs(&[row]);
    }

    fn write_refs(&self, rows: &[&str]) {
        fs::write(
            self.root.join("conformance/REFS.tsv"),
            format!(
                "# ref\tcategory\tsource\tstatus\tsummary\n{}\n",
                rows.join("\n")
            ),
        )
        .unwrap();
    }

    fn write_manifest(&self, row: &str) {
        self.write_manifest_rows(&[row]);
    }

    fn write_manifest_rows(&self, rows: &[&str]) {
        fs::write(
            self.root.join("conformance/MANIFEST.tsv"),
            format!(
                "# id\tcategory\trefs\tprofile\toracle\texpect\tpath\n{}\n",
                rows.join("\n")
            ),
        )
        .unwrap();
    }

    fn write_fixture(&self, name: &str, source: &str) {
        fs::write(self.root.join("conformance/fixtures").join(name), source).unwrap();
    }

    fn write_fixture_bytes(&self, name: &str, source: &[u8]) {
        fs::write(self.root.join("conformance/fixtures").join(name), source).unwrap();
    }
}

impl Drop for FixtureRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn repo_relative(repo: &Path, path: &Path) -> String {
    path.strip_prefix(repo)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

fn assert_luau_0709_provenance_common(fields: &[&str], line_number: usize) {
    assert_eq!(fields[0], "https://github.com/luau-lang/luau");
    assert_eq!(fields[1], "b968ef742741bb2b703afc3b3c53f06608c87481");
    assert!(fields[4].contains("MIT"));
    assert_valid_classification(fields[5], line_number);
    assert!(
        !fields[6].is_empty(),
        "provenance line {line_number} has empty oracle command"
    );
}

fn assert_valid_classification(classification: &str, line_number: usize) {
    assert!(
        matches!(
            classification,
            "api-native"
                | "core-language"
                | "parser"
                | "parser-runtime"
                | "runtime"
                | "stdlib"
                | "error-message"
                | "determinism"
                | "excluded"
        ),
        "provenance line {line_number} has unclassified test classification {classification:?}"
    );
}

fn assert_valid_upstream_conformance_source(path: &str, line_number: usize) {
    assert_valid_upstream_source_path(path, line_number);
    assert!(
        path.starts_with("tests/conformance/") && path.ends_with(".luau"),
        "provenance line {line_number} must cite an upstream conformance .luau file; got {path:?}"
    );
}

fn assert_valid_upstream_source_path(path: &str, line_number: usize) {
    assert_safe_slash_path(path, line_number);
    assert!(
        path.starts_with("tests/conformance/")
            || path.starts_with("VM/src/")
            || path == "tests/Conformance.test.cpp",
        "provenance line {line_number} has unclassified upstream source path {path:?}"
    );
}

fn assert_valid_local_provenance_path(
    repo: &Path,
    path: &str,
    expected_prefix: &str,
    line_number: usize,
) {
    assert_safe_slash_path(path, line_number);
    assert!(
        path.starts_with(expected_prefix),
        "provenance line {line_number} local path {path:?} must start with {expected_prefix:?}"
    );
    assert!(
        repo.join(path).exists(),
        "provenance line {line_number} points at missing local path {path:?}"
    );
}

#[derive(Default)]
struct CoverageCounts {
    refs: usize,
    fixtures: usize,
    luau_0709: usize,
    pending_oracle: usize,
    pending_impl: usize,
    rustia_only: usize,
}

fn coverage_counts(repo: &Path) -> CoverageCounts {
    let mut counts = CoverageCounts::default();
    let refs = fs::read_to_string(repo.join("conformance/REFS.tsv")).unwrap();
    counts.refs = refs
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .count();

    let manifest = fs::read_to_string(repo.join("conformance/MANIFEST.tsv")).unwrap();
    for (line_number, line) in manifest.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 7, "bad manifest line {}", line_number + 1);
        counts.fixtures += 1;
        match fields[4] {
            "luau-0.709" => counts.luau_0709 += 1,
            "pending-oracle" => counts.pending_oracle += 1,
            "pending-impl" => counts.pending_impl += 1,
            "rustia-only" => counts.rustia_only += 1,
            status => panic!(
                "manifest line {} has unknown oracle status {status:?}",
                line_number + 1
            ),
        }
    }
    counts
}

fn luau_0709_ref_catalog_refs(repo: &Path) -> BTreeSet<String> {
    ref_catalog_refs(repo, |status| status == "luau-0.709")
}

fn all_ref_catalog_refs(repo: &Path) -> BTreeSet<String> {
    ref_catalog_refs(repo, |_| true)
}

fn ref_catalog_refs(repo: &Path, mut include_status: impl FnMut(&str) -> bool) -> BTreeSet<String> {
    let refs = fs::read_to_string(repo.join("conformance/REFS.tsv")).unwrap();
    refs.lines()
        .enumerate()
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|(line_number, line)| {
            let fields = line.split('\t').collect::<Vec<_>>();
            assert_eq!(fields.len(), 5, "bad refs line {}", line_number + 1);
            include_status(fields[3]).then(|| fields[0].to_owned())
        })
        .collect()
}

fn luau_0709_manifest_refs(repo: &Path) -> BTreeSet<String> {
    manifest_refs(repo, |oracle| oracle == "luau-0.709")
}

fn all_manifest_refs(repo: &Path) -> BTreeSet<String> {
    manifest_refs(repo, |_| true)
}

fn manifest_refs(repo: &Path, mut include_oracle: impl FnMut(&str) -> bool) -> BTreeSet<String> {
    let manifest = fs::read_to_string(repo.join("conformance/MANIFEST.tsv")).unwrap();
    let mut covered_refs = BTreeSet::new();
    for (line_number, line) in manifest.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 7, "bad manifest line {}", line_number + 1);
        if !include_oracle(fields[4]) {
            continue;
        }
        for reference in fields[2].split(',') {
            assert!(
                !reference.is_empty(),
                "bad empty manifest ref on line {}",
                line_number + 1
            );
            covered_refs.insert(reference.to_owned());
        }
    }
    covered_refs
}

fn assert_safe_slash_path(path: &str, line_number: usize) {
    assert!(
        !path.is_empty(),
        "refs line {line_number} has empty upstream source path"
    );
    assert!(
        !path.starts_with('/'),
        "refs line {line_number} has absolute upstream source path {path:?}"
    );
    assert!(
        !path.contains('\\'),
        "refs line {line_number} has non-portable upstream source path {path:?}"
    );
    assert!(
        !path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == ".."),
        "refs line {line_number} has unsafe upstream source path {path:?}"
    );
}

fn assert_valid_oracle_command(command: &str, line_number: usize) {
    let valid = command.starts_with(
        "LUAU_CONFORMANCE_SOURCE_DIR=<upstream>/tests/conformance \
         <build>/Luau.Conformance --test-case=Conformance/",
    ) || command
        .starts_with("<build>/Luau.Repl conformance/fixtures/upstream-luau-0.709/")
        || command.starts_with("source-inspection: ");
    assert!(
        valid,
        "provenance line {line_number} has unclassified oracle command {command:?}"
    );
}
