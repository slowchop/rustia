use rustia_compiler::compile;
use rustia_parser::parse_source;
use rustia_vm::{Profile, Vm};

const BOUNDARY: &[u8] = b"attempt to yield across metamethod/C-call boundary";

#[test]
fn retained_program_native_callbacks_reject_yield_inside_protected_calls() {
    let source = r#"
        local function nestedCoroutineCanYield()
            local seen
            local child = coroutine.create(function()
                seen = coroutine.isyieldable()
                coroutine.yield("ok")
                return "done"
            end)
            local firstOk, firstValue = coroutine.resume(child)
            local secondOk, secondValue = coroutine.resume(child)
            local failing = coroutine.create(function() error("child failure", 0) end)
            local failureOk, failure = coroutine.resume(failing)
            return seen == true and firstOk == true and firstValue == "ok"
                and secondOk == true and secondValue == "done"
                and failureOk == false and failure == "child failure"
        end

        local tostringSeen, tostringOk, tostringError
        local value = setmetatable({}, {
            __tostring = function()
                assert(nestedCoroutineCanYield())
                tostringSeen = coroutine.isyieldable()
                tostringOk, tostringError = pcall(coroutine.yield, "pause")
                return "value"
            end,
        })
        tostring(value)

        local gsubSeen, gsubOk, gsubError
        string.gsub("a", ".", function()
            assert(nestedCoroutineCanYield())
            gsubSeen = coroutine.isyieldable()
            gsubOk, gsubError = pcall(coroutine.yield, "pause")
            return "b"
        end)

        local foreachSeen, foreachOk, foreachError
        table.foreach({key="value"}, function()
            foreachSeen = coroutine.isyieldable()
            foreachOk, foreachError = xpcall(coroutine.yield, function(message)
                return message
            end, "pause")
        end)

        local foreachiSeen, foreachiOk, foreachiError
        table.foreachi({"value"}, function()
            foreachiSeen = coroutine.isyieldable()
            foreachiOk, foreachiError = pcall(coroutine.yield, "pause")
        end)

        return tostringSeen, tostringOk, tostringError,
            gsubSeen, gsubOk, gsubError,
            foreachSeen, foreachOk, foreachError,
            foreachiSeen, foreachiOk, foreachiError
    "#;
    let program = compile(&parse_source(source).expect("parse")).expect("compile");
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let globals = vm.retain_globals().expect("globals");
    let entry = vm
        .retain_program_entry(program, &globals)
        .expect("retained entry");
    let results = vm
        .call_retained_function(entry.value(&vm).expect("entry value"), &[])
        .expect("call");

    assert_eq!(results.len(), 12);
    for result_group in results.chunks_exact(3) {
        assert_eq!(result_group[0].as_bool(), Some(false));
        assert_eq!(result_group[1].as_bool(), Some(false));
        assert_eq!(
            vm.string_value_bytes(result_group[2])
                .expect("error string"),
            Some(BOUNDARY.to_vec())
        );
    }
}
