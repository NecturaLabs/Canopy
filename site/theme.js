// Applies the saved theme before the first paint, so the page never flashes the other one.
// Midnight (dark) unless the visitor chose Daylight or the system's choice.
(function () {
  var choice = "dark";
  try {
    choice = localStorage.getItem("canopy-theme") || "dark";
  } catch (e) {}
  if (choice === "dark" || choice === "light") {
    document.documentElement.setAttribute("data-theme", choice);
  }
})();
