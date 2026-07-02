assert(not utf8.offset("alo", 5))
assert(not utf8.offset("alo", -4))
assert(utf8.offset("alo", 0) == 1)
assert(utf8.offset("alo", 1) == 1)
assert(utf8.offset("alo", 2) == 2)
assert(utf8.offset("alo", -1) == 3)
return "OK"
