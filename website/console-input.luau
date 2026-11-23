-- The host resumes this coroutine once per submitted line.
local request = {}
local resume, status, yield = coroutine.resume, coroutine.status, coroutine.yield

function readLine()
    return yield(request)
end

return function(program)
    local thread = coroutine.create(program)
    return function(...)
        local ok, result = resume(thread, ...)
        if not ok then error(result, 0) end
        if status(thread) == "dead" then return false end
        if result ~= request then error("Use readLine() to wait for input.", 0) end
        return true
    end
end
