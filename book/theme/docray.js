// Injects a "Try docray" entry at the very top of the book sidebar.
// path_to_root is defined globally by mdBook on every page, so the link
// resolves correctly from any nesting depth and any hosting prefix.
(function () {
  function inject() {
    // mdBook 0.5's scrollbox is a self-populating custom element that would
    // wipe children injected into it — insert ABOVE it as a sibling instead.
    var sidebar = document.getElementById("mdbook-sidebar") || document.getElementById("sidebar");
    if (!sidebar || document.getElementById("try-docray-link")) return;
    var a = document.createElement("a");
    a.id = "try-docray-link";
    a.href = (typeof path_to_root !== "undefined" ? path_to_root : "") + "try/index.html";
    a.innerHTML = "Try docray <span>in your browser — no install, documents never leave your machine</span>";
    sidebar.prepend(a);
  }
  document.readyState === "loading"
    ? document.addEventListener("DOMContentLoaded", inject)
    : inject();
})();
