(function(n) {
    var checksum = 0;
    for (var iter = 0; iter < n; iter++) {
        var array = [];
        for (var i = 0; i < 32; i++) {
            array[i] = i + iter;
        }

        for (var a = 0; a < array.length; a++) {
            checksum = (checksum + array[a]) % 1000000007;
        }

        var object = { a: iter, b: iter + 1, c: iter + 2, d: iter + 3, e: iter + 4 };
        for (var key in object) {
            if (Object.prototype.hasOwnProperty.call(object, key)) {
                checksum = (checksum + object[key]) % 1000000007;
            }
        }
    }
    return checksum;
})
