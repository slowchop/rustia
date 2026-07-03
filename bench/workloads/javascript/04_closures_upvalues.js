(function(n) {
    function makeCounter(seed) {
        var value = seed;
        return function(delta) {
            value = (value + delta * 1664525 + 1013904223) % 1000000007;
            return value;
        };
    }

    var checksum = 0;
    for (var iter = 0; iter < n; iter++) {
        var counter = makeCounter(iter);
        checksum = (checksum + counter(iter & 15) + counter((iter + 7) & 15)) % 1000000007;
    }
    return checksum;
})
