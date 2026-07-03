(function(n) {
    var checksum = 0;
    var values = [];
    for (var iter = 0; iter < n; iter++) {
        var x = (iter % 360) / 57.29577951308232;
        var y = Math.sin(x) * Math.cos(x) + Math.sqrt((iter % 97) + 1);
        values.push(Math.floor(Math.abs(y) * 1000000) % 1000003);

        if (values.length === 64) {
            values.sort(function(a, b) {
                return a - b;
            });
            checksum = (checksum + values[0] + values[values.length - 1]) % 1000000007;
            values.length = 0;
        }
    }

    if (values.length > 0) {
        values.sort(function(a, b) {
            return a - b;
        });
        checksum = (checksum + values[0] + values[values.length - 1]) % 1000000007;
    }
    return checksum;
})
