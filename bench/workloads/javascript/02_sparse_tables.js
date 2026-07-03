(function(n) {
    var checksum = 0;
    for (var iter = 0; iter < n; iter++) {
        var values = Object.create(null);
        for (var i = 0; i < 128; i++) {
            var a = "k" + ((i * 7919 + iter) % 100003);
            var b = "n" + ((i * 104729 + iter * 17) % 100003);
            values[a] = i + iter;
            values[b] = i * 3;
        }

        for (var j = 0; j < 128; j++) {
            var key = "k" + ((j * 7919 + iter) % 100003);
            checksum = (checksum + (values[key] || 0)) % 1000000007;
        }
    }
    return checksum;
})
