// Applies the saved theme before the first paint, so the page never flashes the other one.
(function () {
  var choice = "auto";
  try {
    choice = localStorage.getItem("canopy-theme") || "auto";
  } catch (e) {}
  if (choice === "dark" || choice === "light") {
    document.documentElement.setAttribute("data-theme", choice);
  }
})();
