type Result<T> = { ok: true, value: T } | { ok: false, error: string }

local function answer<T>(value: T): Result<T>
    return { ok = true, value = value }
end

return answer<number>(42)

