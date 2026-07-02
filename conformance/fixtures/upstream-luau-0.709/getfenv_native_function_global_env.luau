-- Derived from Luau 0.709 VM/src/lbaselib.cpp native getfenv handling.
return getfenv(setfenv) == getfenv(0)
