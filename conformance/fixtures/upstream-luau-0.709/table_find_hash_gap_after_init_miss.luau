-- Derived from Luau 0.709 table.find integer-key probing: search stops before later sparse keys.
return table.find({[(2)] = true, [(4)] = true}, true, 3)
