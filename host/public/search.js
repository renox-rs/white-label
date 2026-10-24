// The global search's shortcut (wl-kernel, BL-AGT-02): mod+k (Ctrl+K, or
// Cmd+K on a Mac) focuses the search page's own box when that page is open,
// else the top bar's, opening it first on a phone, where it sits behind the
// search button. A box turned off (no searchable permission) stays off.
document.addEventListener("keydown", function (event) {
  if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;
  if (event.key !== "k" && event.key !== "K") return;
  var box = document.getElementById("wl-search-page-q") || document.getElementById("wl-search-q");
  if (!box || box.disabled) return;
  event.preventDefault();
  if (box.getClientRects().length === 0) {
    var toggle = document.querySelector("[data-rx-search-toggle][aria-expanded='false']");
    if (toggle) toggle.click();
  }
  box.focus();
  box.select();
});
