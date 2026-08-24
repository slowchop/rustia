(function(n) {
    var values = [];
    for (var i = 0; i < 256; i++) {
        values[i] = i + 1;
    }

    var checksum = 0;
    for (var iter = 1; iter <= n; iter++) {
        for (var j = 0; j < 256; j++) {
            values[j] = (values[j] * 3 + iter) % 1000;
        }
        checksum += values[0] + values[255];
    }
    return checksum;
})
