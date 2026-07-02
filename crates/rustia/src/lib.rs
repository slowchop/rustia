use rustia_compiler::CompileError;
use rustia_parser::SourceError;
use rustia_vm::Vm;
use std::fmt;

pub use rustia_vm::{
    Error as VmError, ErrorKind as VmErrorKind, ExtensionHandler, ExtensionOp, OpCapability,
    Profile, Value, ValueKind, VmSettings,
};

pub struct Rustia {
    vm: Vm,
}

impl Rustia {
    pub fn new(profile: Profile) -> Self {
        Self {
            vm: Vm::new(profile),
        }
    }

    pub fn with_settings(profile: Profile, settings: VmSettings) -> Self {
        Self {
            vm: Vm::with_settings(profile, settings),
        }
    }

    pub fn compile_and_run(&mut self, source: &str) -> Result<Value> {
        let chunk = rustia_parser::parse_source(source).map_err(Error::Source)?;
        let program = rustia_compiler::compile(&chunk).map_err(Error::Compile)?;
        self.vm.run(&program).map_err(Error::Vm)
    }

    #[inline]
    pub const fn vm(&self) -> &Vm {
        &self.vm
    }

    #[inline]
    pub fn vm_mut(&mut self) -> &mut Vm {
        &mut self.vm
    }
}

impl Default for Rustia {
    fn default() -> Self {
        Self::new(Profile::ReproducibleStandard)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Source(SourceError),
    Compile(CompileError),
    Vm(VmError),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => error.fmt(formatter),
            Self::Compile(error) => error.fmt(formatter),
            Self::Vm(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::{Profile, Rustia, VmSettings};
    use rustia_compiler::{compile_with_options, CompileOptions};
    use rustia_parser::parse_source;
    use rustia_vm::Vm;

    #[test]
    fn compile_and_run_smoke() {
        let mut rustia = Rustia::new(Profile::DeterministicSim);
        let result = rustia
            .compile_and_run("local x: number = 6 * 7; return x")
            .unwrap();
        assert_eq!(result.as_number(), Some(42.0));
    }

    #[test]
    fn debug_traceback_for_errored_coroutine_reports_top_frame() {
        let source = "\
function halp(key, value)
    local t = {}
    t[key] = value
    return t
end

local co = coroutine.create(halp)
local ok, err = coroutine.resume(co, 0 / 0, 42)
return tostring(ok) .. \"|\" .. tostring(err) .. \"|\" .. debug.traceback(co)
";
        let chunk = parse_source(source).unwrap();
        let program = compile_with_options(
            &chunk,
            CompileOptions::default().with_source("debug.luau", source),
        )
        .unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        let traceback = vm.string_value(result).unwrap().unwrap();
        assert_eq!(
            traceback,
            "false|debug.luau:3: table index is NaN|debug.luau:3 function halp\n"
        );
    }

    #[test]
    fn debug_info_line_defined_precedes_active_line() {
        let source = "\
function testlinedefined()
    local line = debug.info(1, \"l\")
    local linedefined = debug.info(testlinedefined, \"l\")
    return tostring(line) .. \"|\" .. tostring(linedefined)
end

return testlinedefined()
";
        let chunk = parse_source(source).unwrap();
        let program = compile_with_options(
            &chunk,
            CompileOptions::default().with_source("debug.luau", source),
        )
        .unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        let lines = vm.string_value(result).unwrap().unwrap();
        assert_eq!(lines, "2|1");
    }

    #[test]
    fn conformance_int64_userdata_supports_metamethods_and_value_field() {
        let settings = VmSettings {
            enable_conformance_int64_userdata: true,
            ..VmSettings::default()
        };
        let mut rustia = Rustia::with_settings(Profile::ReproducibleStandard, settings);
        let result = rustia
            .compile_and_run(
                "\
local v = int64(42)
v.value = 4
return tostring(v + 2) .. '|' .. tostring(v == int64(4)) .. '|' .. tostring(int64(-5) // 2)
",
            )
            .unwrap();
        let text = rustia.vm_mut().string_value(result).unwrap().unwrap();
        assert_eq!(text, "6|true|-3");
    }

    #[test]
    fn conformance_native_userdata_supports_vec2_and_vertex_surface() {
        let settings = VmSettings {
            enable_conformance_native_userdata: true,
            ..VmSettings::default()
        };
        let mut rustia = Rustia::with_settings(Profile::ReproducibleStandard, settings);
        let result = rustia
            .compile_and_run(
                "\
local s = vec2(5, 4)
local t = vec2(1, 2)
local u = vec2(10, 20)
local x = -u + s * t
local m = u:Min(s)
local v = vertex(vector.create(1, 2, 3), vector.create(0.5, 0, 0.5), vec2(0.25, 0.5))
return x.X == -5 and x.Y == -12 and s:Dot(t) == 13 and m.X == 5 and m.Y == 4 and v.uv.X == 0.25 and v.normal.Z == 0.5
",
            )
            .unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn conformance_native_type_checks_validate_annotated_parameters() {
        let settings = VmSettings {
            enable_conformance_native_type_checks: true,
            ..VmSettings::default()
        };
        let mut rustia = Rustia::with_settings(Profile::ReproducibleStandard, settings);
        let result = rustia
            .compile_and_run(
                "\
local function add(a: number, b: number)
    assert(is_native())
    return a + b
end
local function checkbuffer(a: buffer?) return a == nil or type(a) == 'buffer' end
local ok = pcall(add, nil, 2)
return add(1, 3) == 4 and not ok and checkbuffer(nil) and checkbuffer(buffer.create(1))
",
            )
            .unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn coroutine_resume_arguments_return_from_yield() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .compile_and_run(
                "\
local co
local seen
local function body(a, ...)
    local arg = {...}
    for i = 1, #arg do
        seen = { coroutine.yield(table.unpack(arg[i])) }
    end
    return table.unpack(a)
end
co = coroutine.create(body)
local s, a, b, c, d = coroutine.resume(co, {1, 2, 3}, {}, {1}, {'a', 'b', 'c'})
local ok1 = s and a == nil and coroutine.status(co) == 'suspended'
s, a, b, c, d = coroutine.resume(co, 1, 2, 3)
local ok2 = s and #seen == 3 and seen[1] == 1 and seen[2] == 2 and seen[3] == 3
    and a == 1 and b == nil
s, a, b, c, d = coroutine.resume(co, 'done')
local ok3 = s and #seen == 1 and seen[1] == 'done'
    and a == 'a' and b == 'b' and c == 'c' and d == nil
s, a, b, c, d = coroutine.resume(co)
local ok4 = s and a == 1 and b == 2 and c == 3 and d == nil
    and coroutine.status(co) == 'dead'
return ok1 and ok2 and ok3 and ok4
",
            )
            .unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn coroutine_wrap_resumes_yield_call_results() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .compile_and_run(
                "\
local f = coroutine.wrap(function()
    local a, b = coroutine.yield('yielded')
    return a, b
end)
local first = f()
local second, third = f('resume', 42)
return first == 'yielded' and second == 'resume' and third == 42
",
            )
            .unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn pcall_can_yield_and_resume_successfully() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .compile_and_run(
                "\
local co = coroutine.create(function()
    return pcall(function()
        coroutine.yield('pause')
        return 1, 2, 3
    end)
end)
local ok, value = coroutine.resume(co)
local first = ok and value == 'pause' and coroutine.status(co) == 'suspended'
local done, protected, a, b, c = coroutine.resume(co)
return first and done and protected and a == 1 and b == 2 and c == 3
",
            )
            .unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn nested_pcall_preserves_protected_results_after_yield() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .compile_and_run(
                "\
local co = coroutine.create(function()
    return pcall(function()
        coroutine.yield(1)
        return pcall(function()
            coroutine.yield(2)
            return 3
        end)
    end)
end)
local ok, a = coroutine.resume(co)
local first = ok and a == 1
ok, a = coroutine.resume(co)
local second = ok and a == 2
local done, outer, inner, value = coroutine.resume(co)
return first and second and done and outer and inner and value == 3
",
            )
            .unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn pcall_catches_error_after_yield() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .compile_and_run(
                "\
local co = coroutine.create(function()
    return pcall(function()
        coroutine.yield()
        error('foo')
    end)
end)
local ok = coroutine.resume(co)
local done, protected, err = coroutine.resume(co)
return ok and done and not protected and string.find(err, 'foo', 1, true) ~= nil
",
            )
            .unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }
}
