use rustia_compiler::{compile_with_options, BuiltinEnvironment, CompileOptions};
use rustia_parser::{parse_source_bytes, SourceError};
use rustia_vm::{Profile, Program, Value, ValueKind, Vm, VmSettings};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Error {
    message: String,
}

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn io(path: &Path, error: std::io::Error) -> Self {
        Self::new(format!("{}: {error}", path.display()))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Report {
    pub refs: usize,
    pub fixtures: usize,
    pub luau_oracle: usize,
    pub pending_oracle: usize,
    pub pending_impl: usize,
    pub rustia_only: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaseRun {
    pub id: String,
    pub path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingCase {
    pub id: String,
    pub oracle: String,
    pub path: String,
    pub refs: Vec<String>,
}

pub fn run_repository(repo: impl AsRef<Path>) -> Result<Report> {
    let repo = repo.as_ref();
    let known_refs = load_ref_catalog(repo)?;
    let manifest_text = load_manifest_text(repo)?;
    let cases = parse_manifest_cases(&manifest_text)?;

    let mut report = Report {
        refs: known_refs.len(),
        fixtures: 0,
        luau_oracle: 0,
        pending_oracle: 0,
        pending_impl: 0,
        rustia_only: 0,
    };

    for case in &cases {
        report.fixtures += 1;
        match case.oracle {
            OracleStatus::Luau0709 => report.luau_oracle += 1,
            OracleStatus::PendingOracle => report.pending_oracle += 1,
            OracleStatus::PendingImpl => report.pending_impl += 1,
            OracleStatus::RustiaOnly => report.rustia_only += 1,
        }
        case.run(repo, &known_refs)?;
    }

    if report.fixtures == 0 {
        return Err(Error::new("conformance manifest must not be empty"));
    }
    Ok(report)
}

pub fn run_repository_case(repo: impl AsRef<Path>, selector: &str) -> Result<CaseRun> {
    let repo = repo.as_ref();
    let known_refs = load_ref_catalog(repo)?;
    let manifest_text = load_manifest_text(repo)?;
    let cases = parse_manifest_cases(&manifest_text)?;

    let matches = cases
        .iter()
        .filter(|case| case.id == selector || case.path == selector)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [case] => {
            case.run_forced(repo, &known_refs)?;
            Ok(CaseRun {
                id: case.id.to_owned(),
                path: case.path.to_owned(),
            })
        }
        [] => Err(Error::new(format!(
            "no conformance manifest case matched {selector:?}"
        ))),
        _ => Err(Error::new(format!(
            "conformance manifest selector {selector:?} is ambiguous"
        ))),
    }
}

pub fn list_repository_pending(repo: impl AsRef<Path>) -> Result<Vec<PendingCase>> {
    let repo = repo.as_ref();
    let known_refs = load_ref_catalog(repo)?;
    let manifest_text = load_manifest_text(repo)?;
    let cases = parse_manifest_cases(&manifest_text)?;

    let mut pending = Vec::new();
    for case in &cases {
        case.validate_metadata(&known_refs)?;
        let Some(oracle) = case.oracle.pending_label() else {
            continue;
        };
        let source_path = resolve_manifest_source_path(repo, case.path)?;
        fs::read(&source_path).map_err(|error| Error::io(&source_path, error))?;
        pending.push(PendingCase {
            id: case.id.to_owned(),
            oracle: oracle.to_owned(),
            path: case.path.to_owned(),
            refs: case
                .refs
                .iter()
                .map(|reference| (*reference).to_owned())
                .collect(),
        });
    }
    Ok(pending)
}

pub fn repository_from_crate_manifest(crate_manifest_dir: impl AsRef<Path>) -> Result<PathBuf> {
    crate_manifest_dir
        .as_ref()
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| Error::new("crate must live under <repo>/crates/<crate>"))
}

fn load_manifest_text(repo: &Path) -> Result<String> {
    let manifest = repo.join("conformance/MANIFEST.tsv");
    fs::read_to_string(&manifest).map_err(|error| Error::io(&manifest, error))
}

fn parse_manifest_cases(manifest_text: &str) -> Result<Vec<Case<'_>>> {
    let mut cases = Vec::new();
    let mut seen = ManifestSeen::default();
    for (line_number, raw_line) in manifest_text.lines().enumerate() {
        if raw_line.is_empty() || raw_line.starts_with('#') {
            continue;
        }
        let case = Case::parse(raw_line, line_number + 1)?;
        case.validate_manifest_identity(&mut seen)?;
        cases.push(case);
    }

    if cases.is_empty() {
        return Err(Error::new("conformance manifest must not be empty"));
    }
    Ok(cases)
}

#[derive(Default)]
struct ManifestSeen {
    ids: BTreeSet<String>,
    paths: BTreeSet<String>,
}

struct Case<'a> {
    line_number: usize,
    id: &'a str,
    category: &'a str,
    refs: Vec<&'a str>,
    profile: Profile,
    oracle: OracleStatus,
    expect: Expect<'a>,
    path: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OracleStatus {
    Luau0709,
    PendingOracle,
    PendingImpl,
    RustiaOnly,
}

impl OracleStatus {
    fn pending_label(self) -> Option<&'static str> {
        match self {
            Self::PendingOracle => Some("pending-oracle"),
            Self::PendingImpl => Some("pending-impl"),
            Self::Luau0709 | Self::RustiaOnly => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CatalogStatus {
    Luau0709,
    PendingOracle,
    PendingImpl,
    PendingFixture,
    RustiaOnly,
}

impl CatalogStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Luau0709 => "luau-0.709",
            Self::PendingOracle => "pending-oracle",
            Self::PendingImpl => "pending-impl",
            Self::PendingFixture => "pending-fixture",
            Self::RustiaOnly => "rustia-only",
        }
    }
}

enum Expect<'a> {
    Pass,
    Number(f64),
    NumberBits(u64),
    Bool(bool),
    Nil,
    String,
    StringExact(&'a str),
    StringHex(&'a str),
    ErrorContains(&'a str),
}

impl<'a> Case<'a> {
    fn parse(line: &'a str, line_number: usize) -> Result<Self> {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 7 {
            return Err(Error::new(format!(
                "manifest line {line_number} must have 7 tab-separated columns"
            )));
        }
        if fields[0].is_empty() {
            return Err(Error::new(format!(
                "manifest line {line_number}: id must not be empty"
            )));
        }
        assert_valid_manifest_id(fields[0], line_number)?;
        if fields[1].is_empty() {
            return Err(Error::new(format!(
                "manifest line {line_number}: category must not be empty"
            )));
        }
        assert_valid_manifest_path(fields[6], line_number)?;
        Ok(Self {
            line_number,
            id: fields[0],
            category: fields[1],
            refs: parse_refs(fields[2], line_number)?,
            profile: parse_profile(fields[3], line_number)?,
            oracle: parse_oracle_status(fields[4], line_number)?,
            expect: parse_expect(fields[5], line_number)?,
            path: fields[6],
        })
    }

    fn validate_manifest_identity(&self, seen: &mut ManifestSeen) -> Result<()> {
        if !seen.ids.insert(self.id.to_owned()) {
            return Err(Error::new(format!(
                "manifest line {} duplicates id {:?}",
                self.line_number, self.id
            )));
        }
        if !seen.paths.insert(self.path.to_owned()) {
            return Err(Error::new(format!(
                "manifest line {} duplicates path {:?}",
                self.line_number, self.path
            )));
        }
        Ok(())
    }

    fn run(&self, repo: &Path, known_refs: &BTreeMap<String, CatalogStatus>) -> Result<()> {
        self.validate_metadata(known_refs)?;

        if self.oracle == OracleStatus::PendingImpl {
            let source_path = resolve_manifest_source_path(repo, self.path)?;
            fs::read(&source_path).map_err(|error| Error::io(&source_path, error))?;
            return Ok(());
        }

        self.run_luau(repo)
    }

    fn run_forced(&self, repo: &Path, known_refs: &BTreeMap<String, CatalogStatus>) -> Result<()> {
        self.validate_metadata(known_refs)?;
        self.run_luau(repo)
    }

    fn validate_metadata(&self, known_refs: &BTreeMap<String, CatalogStatus>) -> Result<()> {
        let mut has_pending_oracle_ref = false;
        let mut has_pending_impl_ref = false;
        for reference in &self.refs {
            let Some(status) = known_refs.get(*reference) else {
                return Err(Error::new(format!(
                    "{}: manifest ref {reference:?} is not declared in conformance/REFS.tsv",
                    self.id
                )));
            };
            has_pending_oracle_ref |= *status == CatalogStatus::PendingOracle;
            has_pending_impl_ref |= *status == CatalogStatus::PendingImpl;
            match self.oracle {
                OracleStatus::Luau0709 if *status != CatalogStatus::Luau0709 => {
                    return Err(Error::new(format!(
                        "{}: luau-0.709 manifest row references {reference:?} with {} status in conformance/REFS.tsv",
                        self.id,
                        status.as_str()
                    )));
                }
                OracleStatus::PendingOracle
                    if !matches!(
                        *status,
                        CatalogStatus::Luau0709 | CatalogStatus::PendingOracle
                    ) =>
                {
                    return Err(Error::new(format!(
                        "{}: pending-oracle manifest rows must reference only luau-0.709 or pending-oracle refs; {reference:?} has {} status in conformance/REFS.tsv",
                        self.id,
                        status.as_str()
                    )));
                }
                OracleStatus::PendingImpl
                    if !matches!(
                        *status,
                        CatalogStatus::Luau0709 | CatalogStatus::PendingImpl
                    ) =>
                {
                    return Err(Error::new(format!(
                        "{}: pending-impl manifest rows must reference only luau-0.709 or pending-impl refs; {reference:?} has {} status in conformance/REFS.tsv",
                        self.id,
                        status.as_str()
                    )));
                }
                OracleStatus::RustiaOnly if *status != CatalogStatus::RustiaOnly => {
                    return Err(Error::new(format!(
                        "{}: rustia-only manifest rows must reference only rustia-only refs; {reference:?} has {} status in conformance/REFS.tsv",
                        self.id,
                        status.as_str()
                    )));
                }
                _ => {}
            }
        }
        if self.oracle == OracleStatus::PendingOracle && !has_pending_oracle_ref {
            return Err(Error::new(format!(
                "{}: pending-oracle manifest rows must reference at least one pending-oracle ref in conformance/REFS.tsv",
                self.id
            )));
        }
        if self.oracle == OracleStatus::PendingImpl && !has_pending_impl_ref {
            return Err(Error::new(format!(
                "{}: pending-impl manifest rows must reference at least one pending-impl ref in conformance/REFS.tsv",
                self.id
            )));
        }
        Ok(())
    }

    fn run_luau(&self, repo: &Path) -> Result<()> {
        let source_path = resolve_manifest_source_path(repo, self.path)?;
        let source = fs::read(&source_path).map_err(|error| Error::io(&source_path, error))?;
        let source_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(self.path);
        let result =
            compile_and_run_luau_conformance(&source, source_name, self.path, self.profile);
        match self.expect {
            Expect::Pass => {
                result.map_err(|error| {
                    Error::new(format!(
                        "{} ({}): expected script to pass, got error: {error}",
                        self.id, self.category
                    ))
                })?;
            }
            Expect::Number(expected) => {
                let value = result.map_err(|error| {
                    Error::new(format!(
                        "{} ({}): expected number, got error: {error}",
                        self.id, self.category
                    ))
                })?;
                if value.value.as_number() != Some(expected) {
                    return Err(Error::new(format!(
                        "{} ({}): expected number {expected:?}, got {:?}",
                        self.id, self.category, value.value
                    )));
                }
            }
            Expect::NumberBits(expected) => {
                let value = result.map_err(|error| {
                    Error::new(format!(
                        "{} ({}): expected number bits, got error: {error}",
                        self.id, self.category
                    ))
                })?;
                let Some(number) = value.value.as_number() else {
                    return Err(Error::new(format!(
                        "{} ({}): expected number value, got {:?}",
                        self.id, self.category, value.value
                    )));
                };
                let actual = number.to_bits();
                if actual != expected {
                    return Err(Error::new(format!(
                        "{} ({}): expected bits {expected:016x}, got {actual:016x}",
                        self.id, self.category
                    )));
                }
            }
            Expect::Bool(expected) => {
                let value = result.map_err(|error| {
                    Error::new(format!(
                        "{} ({}): expected bool, got error: {error}",
                        self.id, self.category
                    ))
                })?;
                if value.value.as_bool() != Some(expected) {
                    return Err(Error::new(format!(
                        "{} ({}): expected bool {expected}, got {:?}",
                        self.id, self.category, value.value
                    )));
                }
            }
            Expect::Nil => {
                let value = result.map_err(|error| {
                    Error::new(format!(
                        "{} ({}): expected nil, got error: {error}",
                        self.id, self.category
                    ))
                })?;
                if value.value.kind() != ValueKind::Nil {
                    return Err(Error::new(format!(
                        "{} ({}): expected nil, got {:?}",
                        self.id, self.category, value.value
                    )));
                }
            }
            Expect::String => {
                let value = result.map_err(|error| {
                    Error::new(format!(
                        "{} ({}): expected string, got error: {error}",
                        self.id, self.category
                    ))
                })?;
                if value.value.kind() != ValueKind::String {
                    return Err(Error::new(format!(
                        "{} ({}): expected string, got {:?}",
                        self.id, self.category, value.value
                    )));
                }
            }
            Expect::StringExact(expected) => {
                let value = result.map_err(|error| {
                    Error::new(format!(
                        "{} ({}): expected string {expected:?}, got error: {error}",
                        self.id, self.category
                    ))
                })?;
                if value.string.as_deref() != Some(expected.as_bytes()) {
                    return Err(Error::new(format!(
                        "{} ({}): expected string {expected:?}, got {:?}",
                        self.id, self.category, value.value
                    )));
                }
            }
            Expect::StringHex(expected) => {
                let expected = decode_hex_bytes(expected).map_err(|message| {
                    Error::new(format!(
                        "{}: bad string_hex expectation: {message}",
                        self.id
                    ))
                })?;
                let value = result.map_err(|error| {
                    Error::new(format!(
                        "{} ({}): expected string bytes {:02x?}, got error: {error}",
                        self.id, self.category, expected
                    ))
                })?;
                let Some(actual) = value.string.as_deref() else {
                    return Err(Error::new(format!(
                        "{} ({}): expected string bytes {:02x?}, got {:?}",
                        self.id, self.category, expected, value.value
                    )));
                };
                if actual != expected {
                    return Err(Error::new(format!(
                        "{} ({}): expected string bytes {:02x?}, got {:02x?}",
                        self.id, self.category, expected, actual
                    )));
                }
            }
            Expect::ErrorContains(expected) => {
                let Some(error) = result.err() else {
                    return Err(Error::new(format!(
                        "{}: expected error containing {expected:?}",
                        self.id
                    )));
                };
                let rendered = error.to_string();
                if !rendered.contains(expected) {
                    return Err(Error::new(format!(
                        "{} ({}): error {rendered:?} did not contain {expected:?}",
                        self.id, self.category
                    )));
                }
            }
        }
        Ok(())
    }
}

struct RunOutput {
    value: Value,
    string: Option<Vec<u8>>,
}

fn compile_and_run_luau_conformance(
    source: &[u8],
    source_name: &str,
    harness_path: &str,
    profile: Profile,
) -> std::result::Result<RunOutput, String> {
    let chunk = parse_source_bytes(source).map_err(|error| error.to_string())?;
    let options = CompileOptions {
        builtins: BuiltinEnvironment::LuauConformance,
        ..CompileOptions::default()
    }
    .with_source_bytes(source_name, source);
    let program = compile_with_options(&chunk, options).map_err(|error| error.to_string())?;
    let mut vm = Vm::with_settings(
        profile,
        VmSettings {
            loadstring_compiler: Some(compile_luau_loadstring),
            enable_conformance_coverage: is_upstream_0709_fixture(harness_path, "coverage.luau"),
            enable_conformance_top_level_yield: is_upstream_0709_fixture(
                harness_path,
                "ndebug_upvalues.luau",
            ),
            enable_conformance_int64_userdata: is_upstream_0709_fixture(
                harness_path,
                "userdata.luau",
            ),
            enable_conformance_native_userdata: is_upstream_0709_fixture(
                harness_path,
                "native_userdata.luau",
            ),
            enable_conformance_native_type_checks: is_upstream_0709_fixture(
                harness_path,
                "native_types.luau",
            ) || is_upstream_0709_fixture(
                harness_path,
                "native.luau",
            ),
            enable_conformance_pcall_helpers: is_upstream_0709_fixture(harness_path, "pcall.luau")
                || is_derived_0709_fixture(harness_path, "pcall_cxxthrow_helper.luau")
                || is_derived_0709_fixture(harness_path, "pcall_resumeerror_helper.luau"),
            ..VmSettings::default()
        },
    );
    install_luau_conformance_prelude(&mut vm, harness_path)?;
    let value = vm.run(&program).map_err(|error| error.to_string())?;
    let string = vm
        .string_value_bytes(value)
        .map_err(|error| error.to_string())?;
    Ok(RunOutput { value, string })
}

fn install_luau_conformance_prelude(
    vm: &mut Vm,
    source_name: &str,
) -> std::result::Result<(), String> {
    let source = luau_conformance_prelude_source(source_name);
    if source.is_empty() {
        return Ok(());
    }

    let chunk = parse_source_bytes(&source).map_err(|error| error.to_string())?;
    let options = CompileOptions {
        builtins: BuiltinEnvironment::LuauConformance,
        ..CompileOptions::default()
    }
    .with_source_bytes("rustia-conformance-preamble", &source);
    let program = compile_with_options(&chunk, options).map_err(|error| error.to_string())?;
    vm.run(&program)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn luau_conformance_prelude_source(source_name: &str) -> Vec<u8> {
    let mut source = Vec::new();
    if is_upstream_0709_fixture(source_name, "vector.luau")
        || is_upstream_0709_fixture(source_name, "vector_library.luau")
    {
        source.extend_from_slice(b"function is_native_if_supported() return true end\n");
    }
    if luau_conformance_limited_stack() {
        source.extend_from_slice(b"limitedstack = true\n");
    }
    if is_upstream_0709_fixture(source_name, "tables.luau") {
        source.extend_from_slice(
            b"do local rustia_newproxy = newproxy function makelud(_) return rustia_newproxy() end end\n",
        );
    }
    if is_upstream_0709_fixture(source_name, "gc.luau") {
        source.extend_from_slice(b"function setblockallocations(_) end\n");
    }
    if is_upstream_0709_fixture(source_name, "debugger.luau") {
        source.extend_from_slice(b"function breakpoint(_, _) end\n");
    }
    if is_upstream_0709_fixture(source_name, "cyield.luau") {
        source.extend_from_slice(
            br#"
function singleYield()
    coroutine.yield(2)
    return 4
end

function multipleYields(x)
    coroutine.yield(x + 1)
    coroutine.yield(12)
    coroutine.yield(13)
    return 14
end

function multipleYieldsWithNestedCall(x, nested)
    if nested then
        coroutine.yield(x + 95)
        coroutine.yield(x + 105)
    else
        coroutine.yield(x + 100)
    end
    coroutine.yield(210)
    return 220
end

function passthroughCall(f, ...)
    return f(...)
end

passthroughCallMoreResults = passthroughCall
passthroughCallArgReuse = passthroughCall
passthroughCallVaradic = passthroughCall
passthroughCallWithState = passthroughCall
"#,
        );
    }
    source
}

fn is_upstream_0709_fixture(source_name: &str, fixture: &str) -> bool {
    source_name
        .strip_prefix("conformance/upstream/luau-0.709/tests/conformance/")
        .is_some_and(|name| name == fixture)
}

fn is_derived_0709_fixture(source_name: &str, fixture: &str) -> bool {
    source_name
        .strip_prefix("conformance/fixtures/upstream-luau-0.709/")
        .is_some_and(|name| name == fixture)
}

fn luau_conformance_limited_stack() -> bool {
    cfg!(debug_assertions)
}

fn compile_luau_loadstring(
    source: &[u8],
    source_name: Option<&[u8]>,
) -> std::result::Result<Program, String> {
    let source_name = loadstring_source_name(source, source_name);
    let chunk = parse_source_bytes(source)
        .map_err(|error| luau_loadstring_source_error(&source_name, error))?;
    let options = CompileOptions {
        builtins: BuiltinEnvironment::LuauConformance,
        ..CompileOptions::default()
    }
    .with_source_bytes(source_name.clone(), source);
    compile_with_options(&chunk, options).map_err(|error| format!("{source_name}:1: {error}"))
}

fn luau_loadstring_source_error(source_name: &str, error: SourceError) -> String {
    match error {
        SourceError::Parse(error) if error.message == "expected statement" => {
            format!("{source_name}:1: Incomplete statement: expected assignment or a function call")
        }
        error => format!("{source_name}:1: {error}"),
    }
}

fn loadstring_source_name(source: &[u8], explicit: Option<&[u8]>) -> String {
    if let Some(name) = explicit {
        return explicit_loadstring_source_name(name);
    }
    let mut preview = String::new();
    for byte in source.iter().copied().take(60) {
        match byte {
            b'\\' => preview.push_str("\\\\"),
            b'"' => preview.push_str("\\\""),
            b'\n' => break,
            0x20..=0x7e => preview.push(byte as char),
            _ => preview.push('?'),
        }
    }
    if source.len() > 60 {
        preview.push_str("...");
    }
    format!("[string \"{preview}\"]")
}

fn explicit_loadstring_source_name(name: &[u8]) -> String {
    let (name, mode) = match name.split_first() {
        Some((b'@', rest)) => (rest, LoadStringSourceNameMode::File),
        Some((b'=', rest)) => (rest, LoadStringSourceNameMode::Literal),
        _ => (name, LoadStringSourceNameMode::Literal),
    };
    match mode {
        LoadStringSourceNameMode::File if name.len() > 255 => {
            let start = name.len().saturating_sub(252);
            format!("...{}", String::from_utf8_lossy(&name[start..]))
        }
        LoadStringSourceNameMode::Literal if name.len() > 255 => {
            String::from_utf8_lossy(&name[..255]).into_owned()
        }
        _ => String::from_utf8_lossy(name).into_owned(),
    }
}

#[derive(Clone, Copy)]
enum LoadStringSourceNameMode {
    File,
    Literal,
}

fn load_ref_catalog(repo: &Path) -> Result<BTreeMap<String, CatalogStatus>> {
    let path = repo.join("conformance/REFS.tsv");
    let text = fs::read_to_string(&path).map_err(|error| Error::io(&path, error))?;
    let mut refs = BTreeMap::new();
    for (line_number, raw_line) in text.lines().enumerate() {
        if raw_line.is_empty() || raw_line.starts_with('#') {
            continue;
        }
        let fields: Vec<_> = raw_line.split('\t').collect();
        if fields.len() != 5 {
            return Err(Error::new(format!(
                "refs line {} must have 5 tab-separated columns",
                line_number + 1
            )));
        }
        let reference = fields[0];
        assert_valid_ref(reference, line_number + 1)?;
        if fields[1].is_empty() {
            return Err(Error::new(format!(
                "refs line {} category must not be empty",
                line_number + 1
            )));
        }
        if fields[2].is_empty() {
            return Err(Error::new(format!(
                "refs line {} source must not be empty",
                line_number + 1
            )));
        }
        let status = parse_catalog_status(fields[3], line_number + 1)?;
        if fields[4].is_empty() {
            return Err(Error::new(format!(
                "refs line {} summary must not be empty",
                line_number + 1
            )));
        }
        if refs.insert(reference.to_owned(), status).is_some() {
            return Err(Error::new(format!(
                "refs line {} duplicates ref {reference:?}",
                line_number + 1
            )));
        }
    }
    if refs.is_empty() {
        return Err(Error::new("conformance refs catalog must not be empty"));
    }
    Ok(refs)
}

fn parse_refs(text: &str, line_number: usize) -> Result<Vec<&str>> {
    let refs: Vec<_> = text.split(',').collect();
    if refs.is_empty() {
        return Err(Error::new(format!(
            "manifest line {line_number}: refs column must not be empty"
        )));
    }
    for reference in &refs {
        if reference.is_empty() {
            return Err(Error::new(format!(
                "manifest line {line_number}: refs column contains an empty ref component"
            )));
        }
        assert_valid_ref(reference, line_number)?;
    }
    Ok(refs)
}

fn assert_valid_ref(reference: &str, line_number: usize) -> Result<()> {
    if !reference.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
    }) {
        return Err(Error::new(format!(
            "line {line_number}: bad ref {reference:?}"
        )));
    }
    Ok(())
}

fn assert_valid_manifest_id(id: &str, line_number: usize) -> Result<()> {
    if !id.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
    }) {
        return Err(Error::new(format!(
            "manifest line {line_number}: bad id {id:?}"
        )));
    }
    Ok(())
}

fn assert_valid_manifest_path(path: &str, line_number: usize) -> Result<()> {
    if path.is_empty() {
        return Err(Error::new(format!(
            "manifest line {line_number}: path must not be empty"
        )));
    }
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return Err(Error::new(format!(
            "manifest line {line_number}: path {path:?} must be a repo-relative forward-slash path"
        )));
    }
    if !path.starts_with("conformance/") {
        return Err(Error::new(format!(
            "manifest line {line_number}: path {path:?} must stay under conformance/"
        )));
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(Error::new(format!(
                "manifest line {line_number}: path {path:?} contains an invalid segment"
            )));
        }
    }
    Ok(())
}

fn resolve_manifest_source_path(repo: &Path, path: &str) -> Result<PathBuf> {
    let conformance_root = repo.join("conformance");
    let canonical_conformance =
        fs::canonicalize(&conformance_root).map_err(|error| Error::io(&conformance_root, error))?;
    let source_path = repo.join(path);
    let canonical_source =
        fs::canonicalize(&source_path).map_err(|error| Error::io(&source_path, error))?;
    if !canonical_source.starts_with(&canonical_conformance) {
        return Err(Error::new(format!(
            "manifest path {path:?} resolves outside conformance/"
        )));
    }
    Ok(canonical_source)
}

fn parse_profile(text: &str, line_number: usize) -> Result<Profile> {
    match text {
        "LuauCompat" => Ok(Profile::LuauCompat),
        "ReproducibleStandard" => Ok(Profile::ReproducibleStandard),
        "DeterministicSim" => Ok(Profile::DeterministicSim),
        _ => Err(Error::new(format!(
            "manifest line {line_number}: unknown profile {text:?}"
        ))),
    }
}

fn parse_oracle_status(text: &str, line_number: usize) -> Result<OracleStatus> {
    match text {
        "luau-0.709" => Ok(OracleStatus::Luau0709),
        "pending-oracle" => Ok(OracleStatus::PendingOracle),
        "pending-impl" => Ok(OracleStatus::PendingImpl),
        "rustia-only" => Ok(OracleStatus::RustiaOnly),
        _ => Err(Error::new(format!(
            "manifest line {line_number}: unknown oracle status {text:?}"
        ))),
    }
}

fn parse_catalog_status(text: &str, line_number: usize) -> Result<CatalogStatus> {
    match text {
        "luau-0.709" => Ok(CatalogStatus::Luau0709),
        "pending-oracle" => Ok(CatalogStatus::PendingOracle),
        "pending-impl" => Ok(CatalogStatus::PendingImpl),
        "pending-fixture" => Ok(CatalogStatus::PendingFixture),
        "rustia-only" => Ok(CatalogStatus::RustiaOnly),
        _ => Err(Error::new(format!(
            "refs line {line_number}: unknown status {text:?}"
        ))),
    }
}

fn parse_expect<'a>(text: &'a str, line_number: usize) -> Result<Expect<'a>> {
    if text == "pass" {
        return Ok(Expect::Pass);
    }
    if let Some(value) = text.strip_prefix("number:") {
        return value
            .parse()
            .map(Expect::Number)
            .map_err(|_| Error::new(format!("manifest line {line_number}: bad number {value:?}")));
    }
    if let Some(value) = text.strip_prefix("number_bits:") {
        return u64::from_str_radix(value, 16)
            .map(Expect::NumberBits)
            .map_err(|_| Error::new(format!("manifest line {line_number}: bad bits {value:?}")));
    }
    if let Some(value) = text.strip_prefix("bool:") {
        return match value {
            "true" => Ok(Expect::Bool(true)),
            "false" => Ok(Expect::Bool(false)),
            _ => Err(Error::new(format!(
                "manifest line {line_number}: bad bool {value:?}"
            ))),
        };
    }
    if text == "nil" {
        return Ok(Expect::Nil);
    }
    if text == "string" {
        return Ok(Expect::String);
    }
    if let Some(value) = text.strip_prefix("string:") {
        return Ok(Expect::StringExact(value));
    }
    if let Some(value) = text.strip_prefix("string_hex:") {
        decode_hex_bytes(value).map_err(|message| {
            Error::new(format!(
                "manifest line {line_number}: bad string_hex {value:?}: {message}"
            ))
        })?;
        return Ok(Expect::StringHex(value));
    }
    if let Some(value) = text.strip_prefix("error_contains:") {
        return Ok(Expect::ErrorContains(value));
    }
    Err(Error::new(format!(
        "manifest line {line_number}: unknown expectation {text:?}"
    )))
}

fn decode_hex_bytes(text: &str) -> std::result::Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("hex string must have an even number of digits".to_owned());
    }
    let mut output = Vec::with_capacity(text.len() / 2);
    for pair in text.as_bytes().chunks_exact(2) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn hex_nibble(byte: u8) -> std::result::Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(format!("invalid hex digit 0x{byte:02x}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conformance_limitedstack_matches_debug_harness() {
        let output = compile_and_run_luau_conformance(
            b"return limitedstack == true",
            "limitedstack-test",
            "conformance/fixtures/rustia-only/limitedstack-test.luau",
            Profile::ReproducibleStandard,
        )
        .expect("limitedstack probe should run");

        assert_eq!(
            output.value.as_bool(),
            Some(luau_conformance_limited_stack())
        );
    }

    #[test]
    fn conformance_prelude_installs_native_supported_probe() {
        let output = compile_and_run_luau_conformance(
            b"return is_native_if_supported()",
            "vector.luau",
            "conformance/upstream/luau-0.709/tests/conformance/vector.luau",
            Profile::ReproducibleStandard,
        )
        .expect("native support probe should run");

        assert_eq!(output.value.as_bool(), Some(true));

        let output = compile_and_run_luau_conformance(
            b"return type(is_native_if_supported)",
            "vector.luau",
            "conformance/fixtures/p1-smoke/vector.luau",
            Profile::ReproducibleStandard,
        )
        .expect("non-vector native probe should run");

        assert_eq!(output.string.as_deref(), Some(&b"nil"[..]));
    }

    #[test]
    fn debugger_prelude_installs_breakpoint_helper() {
        let output = compile_and_run_luau_conformance(
            b"breakpoint(8) breakpoint(16, false) return type(breakpoint)",
            "debugger.luau",
            "conformance/upstream/luau-0.709/tests/conformance/debugger.luau",
            Profile::ReproducibleStandard,
        )
        .expect("debugger breakpoint probe should run");

        assert_eq!(output.string.as_deref(), Some(&b"function"[..]));

        let output = compile_and_run_luau_conformance(
            b"return type(breakpoint)",
            "debugger.luau",
            "conformance/fixtures/p1-smoke/debugger.luau",
            Profile::ReproducibleStandard,
        )
        .expect("non-debugger breakpoint probe should run");

        assert_eq!(output.string.as_deref(), Some(&b"nil"[..]));
    }

    #[test]
    fn cyield_prelude_installs_yieldable_helpers() {
        let output = compile_and_run_luau_conformance(
            b"local co = coroutine.wrap(singleYield) return co() * 10 + co()",
            "cyield.luau",
            "conformance/upstream/luau-0.709/tests/conformance/cyield.luau",
            Profile::ReproducibleStandard,
        )
        .expect("cyield helper probe should run");

        assert_eq!(output.value.as_number(), Some(24.0));

        let output = compile_and_run_luau_conformance(
            b"return type(singleYield)",
            "cyield.luau",
            "conformance/fixtures/p1-smoke/cyield.luau",
            Profile::ReproducibleStandard,
        )
        .expect("non-cyield helper probe should run");

        assert_eq!(output.string.as_deref(), Some(&b"nil"[..]));
    }

    #[test]
    fn table_iteration_order_is_profile_specific() {
        let source = b"
            local out = ''
            for _, value in pairs({a = 5, b = 6, c = 7}) do
                if out ~= '' then
                    out ..= ','
                end
                out ..= value
            end
            return out
        ";

        let compat = compile_and_run_luau_conformance(
            source,
            "basic.luau",
            "conformance/upstream/luau-0.709/tests/conformance/basic.luau",
            Profile::LuauCompat,
        )
        .expect("compat pairs probe should run");
        assert_eq!(compat.string.as_deref(), Some(&b"5,7,6"[..]));

        let standard = compile_and_run_luau_conformance(
            source,
            "standard_pairs_order.luau",
            "conformance/fixtures/rustia-only/standard_pairs_order.luau",
            Profile::ReproducibleStandard,
        )
        .expect("standard pairs probe should run");
        assert_eq!(standard.string.as_deref(), Some(&b"5,6,7"[..]));

        let deterministic = compile_and_run_luau_conformance(
            source,
            "deterministic_pairs_order.luau",
            "conformance/fixtures/rustia-only/deterministic_pairs_order.luau",
            Profile::DeterministicSim,
        )
        .expect("deterministic pairs probe should run");
        assert_eq!(deterministic.string.as_deref(), Some(&b"5,6,7"[..]));
    }

    #[test]
    fn tables_prelude_installs_makelud_helper() {
        let output = compile_and_run_luau_conformance(
            b"local t = {} t[makelud(1)] = 1 t[makelud(2)] = 2 local count = 0 for _, v in pairs(t) do count += v end return count",
            "tables.luau",
            "conformance/upstream/luau-0.709/tests/conformance/tables.luau",
            Profile::ReproducibleStandard,
        )
        .expect("tables makelud probe should run");

        assert_eq!(output.value.as_number(), Some(3.0));

        let output = compile_and_run_luau_conformance(
            b"return type(makelud)",
            "tables.luau",
            "conformance/fixtures/p1-smoke/tables.luau",
            Profile::ReproducibleStandard,
        )
        .expect("non-tables makelud probe should run");

        assert_eq!(output.string.as_deref(), Some(&b"nil"[..]));
    }
}
