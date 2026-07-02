-- Derived from the upstream pcall(table.find, {}, 42, 0) error-path check.
return table.find({}, 42, 0)
