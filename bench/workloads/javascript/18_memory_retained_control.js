// JavaScript equivalent of the intentional-retention positive control.
(function () {
  const rows = 128;
  const width = 32;
  const retained = [];
  let step = 0;

  function makeBatch(seed) {
    const batch = [];
    let checksum = 0;
    for (let i = 1; i <= rows; i += 1) {
      const row = [];
      for (let j = 1; j <= width; j += 1) {
        row.push((seed * 131 + i * 17 + j * 7) % 65521);
      }
      batch.push(row);
      checksum += row[0] + row[width - 1];
    }
    return [batch, checksum];
  }

  return function f(n) {
    let checksum = 0;
    for (let epoch = 0; epoch < n; epoch += 1) {
      step += 1;
      const [batch, batchChecksum] = makeBatch(step);
      retained.push(batch);
      checksum += batchChecksum;
    }
    return checksum + step + retained.length;
  };
})()
