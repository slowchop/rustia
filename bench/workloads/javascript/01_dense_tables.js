(function(n) {
    var checksum = 0;
    for (var iter = 0; iter < n; iter++) {
        var values = [];
        for (var i = 0; i < 256; i++) {
            values[i] = (i * 1103515245 + iter * 12345) % 1000003;
        }

        var sum = 0;
        for (var j = 0; j < values.length; j++) {
            sum += values[j];
        }

        values.sort(function(a, b) {
            return a - b;
        });
        checksum = (checksum + sum + values[0] + values[255]) % 1000000007;
    }
    return checksum;
})
