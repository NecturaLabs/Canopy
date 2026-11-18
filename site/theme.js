// Applies the saved theme before the first paint, so the page never flashes the other one.
// Midnight (dark) unless the visitor chose Daylight or the system's choice.
(function () {
  var choice = "dark";
  try {
    choice = localStorage.getItem("canopy-theme") || "dark";
  } catch (e) {}
  if (choice === "dark" || choice === "light") {
    document.documentElement.setAttribute("data-theme", choice);
    // The browser's own bar follows the page, not the system; app.js restores it for "auto"
    var metas = document.querySelectorAll('meta[name="theme-color"]');
    for (var i = 0; i < metas.length; i++) {
      metas[i].setAttribute("data-scheme", metas[i].content);
      metas[i].content = choice === "light" ? "#FBFBFC" : "#09090B";
    }
  }
})();
