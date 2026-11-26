//! Immutable Luau source bundles and deterministic static module graphs.
//!
//! This module performs no file I/O. Embedders choose project-path and file
//! policy, then provide canonical module keys and source bytes. Static graph
//! analysis recognizes unshadowed top-level calls of the form
//! `require("literal")` in local initializers or expression statements.
//! Dynamic and nested calls stay outside this graph and remain an
//! execution-time resolver concern.

use rustia_lexer::Span;
use rustia_parser::{parse_source_bytes, Expr, SourceError, Stmt};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

// Keep the accepted-source identity self-contained. Pulling a public crypto
// crate into rustia-compiler exposes that crate's blanket traits through the
// rustia facade's semantic API inventory even though this digest is private.
struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    length_bytes: u64,
}

impl Sha256 {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    const fn new() -> Self {
        Self {
            state: [
                0x6a09_e667,
                0xbb67_ae85,
                0x3c6e_f372,
                0xa54f_f53a,
                0x510e_527f,
                0x9b05_688c,
                0x1f83_d9ab,
                0x5be0_cd19,
            ],
            block: [0; 64],
            block_len: 0,
            length_bytes: 0,
        }
    }

    fn update(&mut self, bytes: impl AsRef<[u8]>) {
        let mut bytes = bytes.as_ref();
        self.length_bytes = self
            .length_bytes
            .wrapping_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        if self.block_len != 0 {
            let copied = (64 - self.block_len).min(bytes.len());
            self.block[self.block_len..self.block_len + copied].copy_from_slice(&bytes[..copied]);
            self.block_len += copied;
            bytes = &bytes[copied..];
            if self.block_len < 64 {
                return;
            }
            self.compress(self.block);
            self.block_len = 0;
        }
        while bytes.len() >= 64 {
            let (block, rest) = bytes.split_at(64);
            self.compress(block.try_into().expect("a SHA-256 block is 64 bytes"));
            bytes = rest;
        }
        self.block[..bytes.len()].copy_from_slice(bytes);
        self.block_len = bytes.len();
    }

    fn finalize(mut self) -> [u8; 32] {
        let bit_length = self.length_bytes.wrapping_mul(8).to_be_bytes();
        self.block[self.block_len] = 0x80;
        self.block_len += 1;
        if self.block_len > 56 {
            self.block[self.block_len..].fill(0);
            self.compress(self.block);
            self.block = [0; 64];
        } else {
            self.block[self.block_len..56].fill(0);
        }
        self.block[56..].copy_from_slice(&bit_length);
        self.compress(self.block);

        let mut output = [0; 32];
        for (bytes, word) in output.chunks_exact_mut(4).zip(self.state) {
            bytes.copy_from_slice(&word.to_be_bytes());
        }
        output
    }

    fn compress(&mut self, block: [u8; 64]) {
        let mut schedule = [0_u32; 64];
        for (word, bytes) in schedule[..16].iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_be_bytes(bytes.try_into().expect("a SHA-256 word is four bytes"));
        }
        let mut index = 16;
        while index < schedule.len() {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
            index += 1;
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        let mut round = 0;
        while round < Self::K.len() {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let temporary1 = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(Self::K[round])
                .wrapping_add(schedule[round]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temporary2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temporary1);
            d = c;
            c = b;
            b = a;
            a = temporary1.wrapping_add(temporary2);
            round += 1;
        }
        for (state, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *state = state.wrapping_add(value);
        }
    }
}

/// Identity of the canonical source-bundle encoding.
pub const SOURCE_BUNDLE_ABI: &str = "rustia-source-bundle-v1";
/// Identity of the canonical static-module-graph encoding.
pub const STATIC_MODULE_GRAPH_ABI: &str = "rustia-static-module-graph-v1";
/// Maximum number of modules in one active dependency chain.
pub const MAX_STATIC_MODULE_DEPTH: usize = 128;

/// A SHA-256 digest over a versioned, length-delimited compiler input.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentDigest([u8; 32]);

impl ContentDigest {
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// A source-located failure while constructing or traversing a module graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleGraphError {
    pub module: Option<String>,
    pub span: Option<Span>,
    pub message: String,
}

impl ModuleGraphError {
    fn bundle(module: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            module: Some(module.into()),
            span: None,
            message: message.into(),
        }
    }

    fn graph(module: &str, span: Span, message: impl Into<String>) -> Self {
        Self {
            module: Some(module.to_owned()),
            span: Some(span),
            message: message.into(),
        }
    }

    fn entry(message: impl Into<String>) -> Self {
        Self {
            module: None,
            span: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for ModuleGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.module, self.span) {
            (Some(module), Some(span)) => write!(
                formatter,
                "{module}:{}..{}: {}",
                span.start, span.end, self.message
            ),
            (Some(module), None) => write!(formatter, "{module}: {}", self.message),
            (None, _) => formatter.write_str(&self.message),
        }
    }
}

impl std::error::Error for ModuleGraphError {}

/// Immutable, canonically ordered Luau source files supplied by an embedder.
#[derive(Clone, Debug)]
pub struct SourceBundle {
    modules: BTreeMap<String, Arc<[u8]>>,
    digest: ContentDigest,
}

impl SourceBundle {
    pub fn new<I, N, S>(sources: I) -> Result<Self, ModuleGraphError>
    where
        I: IntoIterator<Item = (N, S)>,
        N: Into<String>,
        S: Into<Vec<u8>>,
    {
        let mut modules = BTreeMap::new();
        for (name, source) in sources {
            let name = name.into();
            validate_module_key(&name)
                .map_err(|message| ModuleGraphError::bundle(&name, message))?;
            if modules
                .insert(name.clone(), Arc::from(source.into()))
                .is_some()
            {
                return Err(ModuleGraphError::bundle(name, "duplicate Luau module"));
            }
        }
        let digest = source_digest(&modules);
        Ok(Self { modules, digest })
    }

    pub const fn digest(&self) -> ContentDigest {
        self.digest
    }

    pub fn len(&self) -> usize {
        self.modules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    pub fn source(&self, module: &str) -> Option<&[u8]> {
        self.modules.get(module).map(AsRef::as_ref)
    }

    pub fn modules(&self) -> impl ExactSizeIterator<Item = (&str, &[u8])> {
        self.modules
            .iter()
            .map(|(name, source)| (name.as_str(), source.as_ref()))
    }

    /// Resolve relative `./` and `../` requests with Rustia's default module
    /// policy and build a dependency-first graph of the reachable modules.
    pub fn static_module_graph(
        &self,
        entries: &[&str],
    ) -> Result<StaticModuleGraph, ModuleGraphError> {
        self.static_module_graph_with(entries, resolve_relative_module)
    }

    /// Build a static graph and require every reachable module to be a pure
    /// function module that may be instantiated without persistent state.
    pub fn stateless_function_graph(
        &self,
        entries: &[&str],
    ) -> Result<StaticModuleGraph, ModuleGraphError> {
        let graph = self.static_module_graph(entries)?;
        self.validate_stateless_graph(graph)
    }

    /// Apply embedder-owned request policy, then require every reachable
    /// module to satisfy Rustia's conservative stateless-function contract.
    pub fn stateless_function_graph_with<F, E>(
        &self,
        entries: &[&str],
        resolve: F,
    ) -> Result<StaticModuleGraph, ModuleGraphError>
    where
        F: FnMut(&str, &str) -> Result<String, E>,
        E: fmt::Display,
    {
        let graph = self.static_module_graph_with(entries, resolve)?;
        self.validate_stateless_graph(graph)
    }

    /// Build a graph with embedder-owned request policy. The resolver receives
    /// the importing module and UTF-8 literal request. Rustia still owns parse,
    /// traversal, ordering, cycle/missing checks, source locations and digest.
    pub fn static_module_graph_with<F, E>(
        &self,
        entries: &[&str],
        mut resolve: F,
    ) -> Result<StaticModuleGraph, ModuleGraphError>
    where
        F: FnMut(&str, &str) -> Result<String, E>,
        E: fmt::Display,
    {
        let entries: BTreeSet<_> = entries.iter().copied().collect();
        for entry in &entries {
            validate_module_key(entry)
                .map_err(|message| ModuleGraphError::bundle(*entry, message))?;
            if !self.modules.contains_key(*entry) {
                return Err(ModuleGraphError::entry(format!(
                    "missing entry module {entry:?}"
                )));
            }
        }

        let mut traversal = Traversal {
            bundle: self,
            resolve: &mut resolve,
            loading: Vec::new(),
            depth_below: BTreeMap::new(),
            modules: Vec::new(),
        };
        for entry in &entries {
            traversal.visit(entry, None)?;
        }
        let entries: Vec<String> = entries.into_iter().map(str::to_owned).collect();
        let digest = graph_digest(&entries, &traversal.modules);
        Ok(StaticModuleGraph {
            source_digest: self.digest,
            digest,
            entries,
            modules: traversal.modules,
        })
    }

    fn validate_stateless_graph(
        &self,
        graph: StaticModuleGraph,
    ) -> Result<StaticModuleGraph, ModuleGraphError> {
        for module in &graph.modules {
            let source = self
                .source(&module.name)
                .expect("a static graph module belongs to its source bundle");
            crate::stateless_helper::validate(source).map_err(|error| ModuleGraphError {
                module: Some(module.name.clone()),
                span: error.span,
                message: error.message,
            })?;
        }
        Ok(graph)
    }
}

/// One source-located literal dependency.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticRequire {
    pub alias: Option<String>,
    pub request: String,
    pub target: String,
    pub span: Span,
}

/// One reachable module and its authored-order static dependencies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticModule {
    pub name: String,
    pub requires: Vec<StaticRequire>,
}

/// Deterministic dependency-first closure of one or more entry modules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticModuleGraph {
    pub source_digest: ContentDigest,
    pub digest: ContentDigest,
    pub entries: Vec<String>,
    pub modules: Vec<StaticModule>,
}

impl StaticModuleGraph {
    pub fn module(&self, name: &str) -> Option<&StaticModule> {
        self.modules.iter().find(|module| module.name == name)
    }

    pub fn module_names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.modules.iter().map(|module| module.name.as_str())
    }
}

struct UnresolvedRequire {
    alias: Option<String>,
    request: String,
    span: Span,
}

struct Traversal<'a, F> {
    bundle: &'a SourceBundle,
    resolve: &'a mut F,
    loading: Vec<String>,
    depth_below: BTreeMap<String, usize>,
    modules: Vec<StaticModule>,
}

impl<F, E> Traversal<'_, F>
where
    F: FnMut(&str, &str) -> Result<String, E>,
    E: fmt::Display,
{
    fn visit(
        &mut self,
        name: &str,
        imported_at: Option<(&str, Span)>,
    ) -> Result<(), ModuleGraphError> {
        if let Some(index) = self.loading.iter().position(|loading| loading == name) {
            let mut cycle = self.loading[index..].to_vec();
            cycle.push(name.to_owned());
            let (module, span) = imported_at.expect("a cycle has an importing edge");
            return Err(ModuleGraphError::graph(
                module,
                span,
                format!("cyclic static require: {}", cycle.join(" -> ")),
            ));
        }
        if let Some(depth_below) = self.depth_below.get(name) {
            if self.loading.len().saturating_add(*depth_below) > MAX_STATIC_MODULE_DEPTH {
                return Err(module_depth_error(imported_at));
            }
            return Ok(());
        }
        if self.loading.len() >= MAX_STATIC_MODULE_DEPTH {
            return Err(module_depth_error(imported_at));
        }
        let source = self.bundle.source(name).ok_or_else(|| match imported_at {
            Some((module, span)) => {
                ModuleGraphError::graph(module, span, format!("missing static module {name:?}"))
            }
            None => ModuleGraphError::entry(format!("missing entry module {name:?}")),
        })?;
        self.loading.push(name.to_owned());
        let unresolved = static_requires(name, source)?;
        let mut requires = Vec::with_capacity(unresolved.len());
        for require in unresolved {
            let target = (self.resolve)(name, &require.request)
                .map_err(|error| ModuleGraphError::graph(name, require.span, format!("{error}")))?;
            validate_module_key(&target)
                .map_err(|message| ModuleGraphError::graph(name, require.span, message))?;
            self.visit(&target, Some((name, require.span)))?;
            requires.push(StaticRequire {
                alias: require.alias,
                request: require.request,
                target,
                span: require.span,
            });
        }
        let popped = self.loading.pop();
        debug_assert_eq!(popped.as_deref(), Some(name));
        let depth_below = requires
            .iter()
            .map(|require| {
                self.depth_below
                    .get(&require.target)
                    .copied()
                    .expect("a dependency is completed before its importer")
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.depth_below.insert(name.to_owned(), depth_below);
        self.modules.push(StaticModule {
            name: name.to_owned(),
            requires,
        });
        Ok(())
    }
}

fn module_depth_error(imported_at: Option<(&str, Span)>) -> ModuleGraphError {
    let message = format!("static module nesting exceeds {MAX_STATIC_MODULE_DEPTH} modules");
    match imported_at {
        Some((module, span)) => ModuleGraphError::graph(module, span, message),
        None => ModuleGraphError::entry(message),
    }
}

fn static_requires(
    module: &str,
    source: &[u8],
) -> Result<Vec<UnresolvedRequire>, ModuleGraphError> {
    let chunk = parse_source_bytes(source).map_err(|error| match error {
        SourceError::Lex(error) => ModuleGraphError::graph(module, error.span, error.message),
        SourceError::Parse(error) => ModuleGraphError::graph(module, error.span, error.message),
    })?;
    let mut requires = Vec::new();
    let mut require_shadowed = false;
    for statement in chunk.statements {
        match statement {
            Stmt::Local { bindings, init, .. } => {
                // Luau local bindings enter scope after every initializer has
                // run, so `local require = require("./real")` still calls the
                // global resolver once and shadows it for later statements.
                if !require_shadowed {
                    for (index, expression) in init.iter().enumerate() {
                        if let Some((request, span)) = literal_require(module, expression)? {
                            requires.push(UnresolvedRequire {
                                alias: bindings.get(index).map(|binding| binding.name.clone()),
                                request,
                                span,
                            });
                        }
                    }
                }
                require_shadowed |= bindings.iter().any(|binding| binding.name == "require");
            }
            Stmt::LocalFunction { name, .. } => {
                require_shadowed |= name == "require";
            }
            Stmt::Expr { expr, .. } if !require_shadowed => {
                if let Some((request, span)) = literal_require(module, &expr)? {
                    requires.push(UnresolvedRequire {
                        alias: None,
                        request,
                        span,
                    });
                }
            }
            _ => {}
        }
    }
    Ok(requires)
}

fn literal_require(
    module: &str,
    expression: &Expr,
) -> Result<Option<(String, Span)>, ModuleGraphError> {
    let Expr::Call { callee, args, .. } = expression else {
        return Ok(None);
    };
    if !matches!(callee.as_ref(), Expr::Name(name, _) if name == "require") {
        return Ok(None);
    }
    let [Expr::String(request, span)] = args.as_slice() else {
        return Ok(None);
    };
    let request = std::str::from_utf8(request)
        .map_err(|_| ModuleGraphError::graph(module, *span, "static require path must be UTF-8"))?;
    Ok(Some((request.to_owned(), *span)))
}

/// Default relative resolver for canonical project module keys.
pub fn resolve_relative_module(importer: &str, request: &str) -> Result<String, &'static str> {
    if request.contains(['\\', ':', '\0'])
        || !(request.starts_with("./") || request.starts_with("../"))
    {
        return Err("require expects a project-relative path");
    }
    let mut path: Vec<_> = importer.split('/').collect();
    path.pop();
    for part in request.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                path.pop().ok_or("require escapes the project root")?;
            }
            name => path.push(name),
        }
    }
    let mut key = path.join("/");
    if !key.ends_with(".luau") {
        key.push_str(".luau");
    }
    validate_module_key(&key)?;
    Ok(key)
}

fn validate_module_key(name: &str) -> Result<(), &'static str> {
    if name.contains(['\\', ':', '\0'])
        || !name.ends_with(".luau")
        || name
            .split('/')
            .any(|part| matches!(part, "" | "." | ".." | ".luau"))
    {
        return Err("expected a canonical project-relative .luau module key");
    }
    Ok(())
}

fn source_digest(modules: &BTreeMap<String, Arc<[u8]>>) -> ContentDigest {
    let mut digest = Sha256::new();
    for (name, source) in modules {
        // Preserve the accepted-source identity already shared by the live
        // compiler and host: sorted path, NUL, decimal byte length, NUL, exact
        // bytes. Canonical module keys cannot contain NUL, and the explicit
        // source length makes adjacent source payloads unambiguous.
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update(source.len().to_string().as_bytes());
        digest.update([0]);
        digest.update(source.as_ref());
    }
    ContentDigest(digest.finalize())
}

fn graph_digest(entries: &[String], modules: &[StaticModule]) -> ContentDigest {
    let mut digest = Sha256::new();
    digest_field(&mut digest, STATIC_MODULE_GRAPH_ABI.as_bytes());
    digest_count(&mut digest, entries.len());
    for entry in entries {
        digest_field(&mut digest, entry.as_bytes());
    }
    digest_count(&mut digest, modules.len());
    for module in modules {
        digest_field(&mut digest, module.name.as_bytes());
        digest_count(&mut digest, module.requires.len());
        for require in &module.requires {
            digest_field(
                &mut digest,
                require.alias.as_deref().unwrap_or_default().as_bytes(),
            );
            digest_field(&mut digest, require.request.as_bytes());
            digest_field(&mut digest, require.target.as_bytes());
        }
    }
    ContentDigest(digest.finalize())
}

fn digest_count(digest: &mut Sha256, count: usize) {
    digest.update(u64::try_from(count).unwrap_or(u64::MAX).to_le_bytes());
}

fn digest_field(digest: &mut Sha256, field: &[u8]) {
    digest_count(digest, field.len());
    digest.update(field);
}

#[cfg(test)]
mod sha256_tests {
    use super::{ContentDigest, Sha256};

    fn digest(parts: &[&[u8]]) -> String {
        let mut digest = Sha256::new();
        for part in parts {
            digest.update(part);
        }
        ContentDigest(digest.finalize()).to_string()
    }

    #[test]
    fn private_sha256_matches_fixed_vectors_and_incremental_updates() {
        assert_eq!(
            digest(&[b""]),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest(&[b"a", b"b", b"c"]),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            digest(&[b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"]),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }
}
