let active;
let skipFocus;
let closeTimer;

function positionPopup() {
  if (!active) return;
  const { button, popup } = active;
  const anchor = button.getBoundingClientRect();
  const inset = 8;
  const viewportWidth = document.documentElement.clientWidth;
  const viewportHeight = document.documentElement.clientHeight;
  const popupWidth = popup.offsetWidth;
  const popupHeight = popup.offsetHeight;
  popup.style.left = `${Math.max(inset, Math.min(anchor.left, viewportWidth - popupWidth - inset))}px`;
  const top = anchor.bottom + popupHeight <= viewportHeight - inset
    ? anchor.bottom
    : Math.max(inset, anchor.top - popupHeight);
  popup.style.top = `${top}px`;
}

function closePopup({ restoreFocus = false } = {}) {
  if (!active) return;
  clearTimeout(closeTimer);
  const { button, popup } = active;
  active = undefined;
  if (restoreFocus && popup.contains(document.activeElement)) {
    skipFocus = button;
    button.focus({ preventScroll: true });
    skipFocus = undefined;
  }
  popup.hidePopover();
  button.setAttribute("aria-expanded", "false");
}

function openPopup(button, popup, pinned = false) {
  clearTimeout(closeTimer);
  if (active?.button === button) {
    active.pinned ||= pinned;
    return;
  }
  closePopup();
  active = { button, popup, pinned };
  popup.showPopover();
  positionPopup();
  button.setAttribute("aria-expanded", "true");
}

function closeAfterLeaving() {
  clearTimeout(closeTimer);
  closeTimer = setTimeout(() => {
    if (!active || active.pinned) return;
    const { button, popup } = active;
    if (button.matches(":hover, :focus-visible") || popup.matches(":hover, :focus-within")) return;
    closePopup();
  }, 130);
}

for (const button of document.querySelectorAll(".preview-platform")) {
  const popup = document.getElementById(button.getAttribute("popovertarget"));
  button.addEventListener("pointerenter", event => {
    if (event.pointerType === "mouse" && (active?.button === button ||
        (!active?.pinned && !active?.popup.contains(document.activeElement)))) {
      openPopup(button, popup);
    }
  });
  button.addEventListener("pointerleave", closeAfterLeaving);
  button.addEventListener("focus", () => {
    if (button !== skipFocus && button.matches(":focus-visible")) openPopup(button, popup);
  });
  button.addEventListener("focusout", closeAfterLeaving);
  button.addEventListener("click", event => {
    event.preventDefault();
    if (active?.button === button && active.pinned) closePopup();
    else openPopup(button, popup, true);
  });
  popup.addEventListener("pointerenter", () => clearTimeout(closeTimer));
  popup.addEventListener("pointerleave", closeAfterLeaving);
  popup.addEventListener("focusout", event => {
    if (active?.popup === popup && event.relatedTarget !== button &&
        !popup.contains(event.relatedTarget)) {
      closePopup();
    }
  });
}

document.addEventListener("pointerdown", event => {
  if (active && !active.button.contains(event.target) && !active.popup.contains(event.target)) {
    closePopup();
  }
});
document.addEventListener("keydown", event => {
  if (event.key === "Escape" && active) {
    event.preventDefault();
    closePopup({ restoreFocus: true });
  }
});
window.addEventListener("resize", positionPopup);
window.addEventListener("scroll", positionPopup, { capture: true, passive: true });
