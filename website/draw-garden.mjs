const motifs = [['bloom','#e78063'],['bud','#8eae78'],['ring','#d99c38'],
                ['bud','#d5a6c7'],['bloom','#adc2de']];

export function drawGarden(layer, shapes) {
  while (layer.children.length > shapes.length) layer.lastElementChild.remove();
  shapes.forEach(([kind,x,y,angle], index) => {
    let node = layer.children[index];
    if (!node) {
      node = document.createElementNS('http://www.w3.org/2000/svg','use');
      layer.append(node);
    }
    const [shape,color] = motifs[((kind % motifs.length) + motifs.length) % motifs.length];
    if (node.dataset.kind !== String(kind)) {
      node.setAttribute('href','#' + shape);
      node.style.color = color;
      node.dataset.kind = String(kind);
    }
    node.setAttribute('transform', `translate(${x} ${y}) rotate(${angle})`);
  });
}
