(function(n) {
    var fragments = ["alpha", "beta", "gamma", "delta"];
    var checksum = 0;
    for (var iter = 0; iter < n; iter++) {
        var parts = [];
        for (var i = 0; i < 64; i++) {
            var fragment = fragments[(i + iter) % fragments.length];
            parts.push(fragment + ":" + (i % 17) + ":" + (iter % 31) + "|");
        }
        var s = parts.join("");
        checksum += s.length;
    }
    return checksum;
})
