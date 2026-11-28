const controls = document.getElementById("benchmark-measures");
const charts = document.getElementById("benchmark-charts");
const measures = Array.from(controls.querySelectorAll("input"));
const views = charts.querySelectorAll("[data-benchmark-view]");
const minimumHint = document.getElementById("benchmark-measure-minimum");
const sortBy = document.getElementById("benchmark-sort");
const engineLists = charts.querySelectorAll(":scope > div > ul");

function sortCharts() {
  const metric = sortBy.value;
  for (const list of engineLists) {
    const rows = Array.from(list.querySelectorAll(".benchmark-engine:not(.benchmark-skipped)"));
    rows.sort((left, right) => {
      const byName = () => left.firstElementChild.textContent.localeCompare(
        right.firstElementChild.textContent, "en", { sensitivity: "base" },
      );
      if (metric === "name") return byName();
      const timeDifference = Number(left.getAttribute(`data-sort-${metric}`)) -
        Number(right.getAttribute(`data-sort-${metric}`));
      return timeDifference || byName();
    });
    list.prepend(...rows);
  }
}

function showBenchmarkMeasures() {
  let selected = measures.filter(input => input.checked);
  if (selected.length === 0) {
    measures[0].checked = true;
    selected = [measures[0]];
  }
  const visible = new Set(selected.map(input => input.value));
  if (sortBy.value !== "name" && !visible.has(sortBy.value)) {
    sortBy.value = selected[0].value;
  }
  for (const view of views) {
    view.hidden = !visible.has(view.dataset.benchmarkView);
  }
  charts.toggleAttribute("data-multiple-measures", selected.length > 1);
  for (const input of measures) {
    const locked = selected.length === 1 && input.checked;
    const button = input.nextElementSibling;
    if (locked) {
      input.setAttribute("aria-disabled", "true");
      input.setAttribute("aria-describedby", minimumHint.id);
      button.title = minimumHint.textContent;
    } else {
      input.removeAttribute("aria-disabled");
      input.removeAttribute("aria-describedby");
      button.removeAttribute("title");
    }
  }
  sortCharts();
}

controls.addEventListener("click", event => {
  if (event.target.matches('input[aria-disabled="true"]')) event.preventDefault();
});
controls.addEventListener("change", showBenchmarkMeasures);
sortBy.addEventListener("change", () => {
  const measure = measures.find(input => input.value === sortBy.value);
  if (measure) measure.checked = true;
  showBenchmarkMeasures();
});
showBenchmarkMeasures();

const exactTimes = document.getElementById("benchmark-exact-times");

function showTimes() {
  charts.toggleAttribute("data-exact-times", exactTimes.checked);
}

exactTimes.addEventListener("change", showTimes);
showTimes();

const tooltip = document.getElementById("benchmark-tooltip");
let tooltipAnchor;
let tooltipCloseTimer;

function hideTooltip() {
  clearTimeout(tooltipCloseTimer);
  if (tooltipAnchor) tooltip.hidePopover();
  tooltipAnchor = undefined;
}

function scheduleTooltipClose() {
  clearTimeout(tooltipCloseTimer);
  tooltipCloseTimer = setTimeout(hideTooltip, 140);
}

charts.addEventListener("pointerover", event => {
  if (event.pointerType !== "mouse") return;
  const line = event.target.closest("[data-benchmark-tooltip]");
  if (!line || line.contains(event.relatedTarget)) return;
  clearTimeout(tooltipCloseTimer);
  const row = line.closest(".benchmark-engine");
  tooltip.textContent = `${row.firstElementChild.textContent} · ${line.dataset.benchmarkTooltip}`;
  if (!tooltipAnchor) tooltip.showPopover();
  tooltipAnchor = line;

  const inset = 8;
  const gap = 10;
  const viewportWidth = document.documentElement.clientWidth;
  const viewportHeight = document.documentElement.clientHeight;
  const anchor = row.getBoundingClientRect();
  tooltip.style.maxHeight = "";
  const { width, height } = tooltip.getBoundingClientRect();
  let left;
  let top;

  if (anchor.right + gap + width <= viewportWidth - inset) {
    left = anchor.right + gap;
  } else if (anchor.left - gap - width >= inset) {
    left = anchor.left - gap - width;
  }

  if (left !== undefined) {
    top = Math.max(inset, Math.min(event.clientY - height / 2, viewportHeight - height - inset));
  } else {
    const above = anchor.top - gap - inset;
    const below = viewportHeight - inset - anchor.bottom - gap;
    const placeAbove = above >= height || above > below;
    const available = placeAbove ? above : below;
    if (available <= 0) {
      hideTooltip();
      return;
    }
    tooltip.style.maxHeight = `${available}px`;
    left = Math.max(inset, Math.min(event.clientX - width / 2, viewportWidth - width - inset));
    top = placeAbove ? anchor.top - gap - tooltip.getBoundingClientRect().height : anchor.bottom + gap;
  }

  tooltip.style.left = `${left}px`;
  tooltip.style.top = `${top}px`;
});

charts.addEventListener("pointerout", event => {
  if (tooltipAnchor?.contains(event.target) && !tooltipAnchor.contains(event.relatedTarget) &&
      !tooltip.contains(event.relatedTarget)) {
    scheduleTooltipClose();
  }
});
tooltip.addEventListener("pointerenter", () => clearTimeout(tooltipCloseTimer));
tooltip.addEventListener("pointerleave", event => {
  if (!tooltipAnchor?.contains(event.relatedTarget)) scheduleTooltipClose();
});
document.addEventListener("keydown", event => {
  if (event.key === "Escape") hideTooltip();
});
window.addEventListener("scroll", hideTooltip, { capture: true, passive: true });
window.addEventListener("resize", hideTooltip);
