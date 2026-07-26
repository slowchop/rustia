// JavaScript equivalent of memory_burst.
(function () {
  const rows = 512;
  const width = 64;
  return function f(n) {
    let checksum = 0;
    for (let epoch = 1; epoch <= n; epoch += 1) {
      const batch = [];
      for (let i = 1; i <= rows; i += 1) {
        const row = [];
        for (let j = 1; j <= width; j += 1) {
          row.push((epoch * 131 + i * 17 + j * 7) % 65521);
        }
        batch.push(row);
        checksum += row[0] + row[width - 1];
      }
      checksum += batch.length;
    }
    return checksum;
  };
})()
