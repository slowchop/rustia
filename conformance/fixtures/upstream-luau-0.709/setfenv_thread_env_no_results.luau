-- Derived from Luau 0.709 VM/src/lbaselib.cpp level-0 setfenv return behavior.
local env = getfenv(0)
return select("#", setfenv(0, env))
