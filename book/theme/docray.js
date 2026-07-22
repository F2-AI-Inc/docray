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
  // Site-wide "Built by F2" footer — docray is owned by F2 AI.
  function injectFooter() {
    var main = document.querySelector("main");
    if (!main || document.getElementById("f2-footer")) return;
    var root = typeof path_to_root !== "undefined" ? path_to_root : "";
    var footer = document.createElement("footer");
    footer.id = "f2-footer";
    var link = document.createElement("a");
    link.href = "https://f2.ai";
    link.target = "_blank";
    link.rel = "noopener";
    link.setAttribute("aria-label", "F2 AI");
    var label = document.createElement("span");
    label.textContent = "Built by";
    var img = document.createElement("img");
    img.src = root + "f2-logo.png";
    img.alt = "F2 AI";
    img.width = 26;
    img.height = 26;
    link.append(label, img);
    footer.appendChild(link);
    main.appendChild(footer);
  }
  function run() {
    inject();
    injectFooter();
  }
  document.readyState === "loading"
    ? document.addEventListener("DOMContentLoaded", run)
    : run();
})();
