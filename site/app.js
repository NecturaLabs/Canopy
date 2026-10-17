"use strict";

const MANIFEST = "https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/latest.json";
const ASSET = /^https:\/\/github\.com\/NecturaLabs\/app-updates\/releases\/download\/canopy-build-\d+\/canopy-build-\d+-[a-z0-9_-]+\.(tar\.gz|zip)$/;
const root = document.documentElement;
const $ = (sel, el = document) => el.querySelector(sel);
const $$ = (sel, el = document) => [...el.querySelectorAll(sel)];
const calm = window.matchMedia("(prefers-reduced-motion: reduce)");

// A box that scrolls fades out at the edge where more waits (CSS: .has-more)
function edgeFade(el, sideways) {
  const more = sideways
    ? el.scrollWidth - el.scrollLeft - el.clientWidth > 2
    : el.scrollHeight - el.scrollTop - el.clientHeight > 2;
  el.classList.toggle("has-more", more);
}

function store(key, value) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch (e) { /* storage blocked: the choice lasts for this page only */ }
}

/* Theme: Midnight, Daylight or the system's choice */
const themeButtons = $$("[data-theme-choice]");
function setTheme(choice) {
  if (choice === "auto") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", choice);
  store("canopy-theme", choice === "auto" ? null : choice);
  $$('meta[name="theme-color"]').forEach((m) => {
    m.dataset.scheme ??= m.content;
    m.content = choice === "auto" ? m.dataset.scheme : choice === "light" ? "#FBFBFC" : "#09090B";
  });
  themeButtons.forEach((b) => {
    const on = b.dataset.themeChoice === choice;
    b.setAttribute("aria-checked", String(on));
    b.tabIndex = on ? 0 : -1;
  });
}
setTheme(root.getAttribute("data-theme") || "auto");
themeButtons.forEach((b, i) => {
  b.addEventListener("click", () => setTheme(b.dataset.themeChoice));
  b.addEventListener("keydown", (e) => {
    const d = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!d) return;
    const next = themeButtons[(i + d + themeButtons.length) % themeButtons.length];
    setTheme(next.dataset.themeChoice);
    next.focus();
    e.preventDefault();
  });
});

/* The visitor's system decides which install command comes first */
function detect() {
  const ua = navigator.userAgent;
  const platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  const touchMac = /Mac/i.test(platform) && navigator.maxTouchPoints > 1;
  if (/Android/i.test(ua) || /Android/i.test(platform)) return "phone";
  if (/iPhone|iPad|iPod/i.test(ua) || touchMac) return "phone";
  if (/Win/i.test(platform) || /Windows/.test(ua)) return "windows-x86_64";
  if (/Mac/i.test(platform) || /Mac OS X/.test(ua)) return "macos";
  if (/Linux/i.test(platform) || /Linux/.test(ua)) return /aarch64|arm64/i.test(navigator.platform + " " + platform + " " + ua) ? "linux-aarch64" : "linux-x86_64";
  return "unknown";
}
const system = detect();
const suggested = $('[data-platform="' + system + '"]');
if (suggested) suggested.classList.add("is-suggested");

const NOTES = {
  phone: ["\uE560", "It runs on Linux (x86_64 and arm64) and Windows (x86_64). There is no mobile app yet; open this page on your computer to install it.", "Canopy is a desktop app"],
  macos: ["", "There is no macOS build yet. Canopy runs on Linux and Windows."],
  unknown: ["", "Canopy runs on Linux and Windows. Choose your system below."],
};
if (NOTES[system]) {
  $$("[data-note]").forEach((n) => {
    $("[data-note-icon]", n).textContent = NOTES[system][0];
    const text = $("[data-note-text]", n);
    text.textContent = NOTES[system][1];
    if (NOTES[system][2]) {
      const title = document.createElement("strong");
      title.className = "note-title";
      title.textContent = NOTES[system][2];
      text.prepend(title, " ");
    }
    n.hidden = false;
  });
}

/* One install command per system; the switch shows the other one */
const COMMANDS = {
  sh: { name: "Linux", prompt: "$", text: "curl -fsSL https://canopy.necturalabs.com/install | sh" },
  ps: { name: "Windows", prompt: "PS>", text: "irm https://canopy.necturalabs.com/install.ps1 | iex" },
};
const copyStatus = $("#copy-status");
function setOs(os) {
  const c = COMMANDS[os];
  $$("[data-install]").forEach((box) => {
    $$("[data-os]", box).forEach((b) => {
      const on = b.dataset.os === os;
      b.setAttribute("aria-checked", String(on));
      b.tabIndex = on ? 0 : -1;
    });
    const code = $("[data-cmd]", box);
    code.textContent = c.text;
    $("[data-prompt]", box).textContent = c.prompt;
    edgeFade(code, true);
    $("[data-copy-cmd]", box).setAttribute("aria-label", "Copy the " + c.name + " install command");
  });
  setOs.current = os;
}
async function copyCommand(os, button) {
  const c = COMMANDS[os];
  let ok = true;
  try { await navigator.clipboard.writeText(c.text); } catch (e) { ok = false; }
  copyStatus.textContent = ok
    ? "Copied the " + c.name + " install command. Paste it into a terminal."
    : "This browser blocked copying. Select the command and copy it instead.";
  if (button && ok) {
    $("[data-copy-label]", button).textContent = "Copied";
    $("[data-copy-icon]", button).textContent = "";
    button.classList.add("is-copied");
    clearTimeout(button.copyTimer);
    button.copyTimer = setTimeout(() => {
      $("[data-copy-label]", button).textContent = "Copy";
      $("[data-copy-icon]", button).textContent = "";
      button.classList.remove("is-copied");
    }, 2400);
  }
  clearTimeout(copyCommand.timer);
  copyCommand.timer = setTimeout(() => { copyStatus.textContent = ""; }, 5000);
}
$$("[data-install]").forEach((box) => {
  const code = $("[data-cmd]", box);
  code.addEventListener("scroll", () => edgeFade(code, true), { passive: true });
  window.addEventListener("resize", () => edgeFade(code, true), { passive: true });
  const radios = $$("[data-os]", box);
  radios.forEach((b, i) => {
    b.addEventListener("click", () => setOs(b.dataset.os));
    b.addEventListener("keydown", (e) => {
      const d = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
      if (!d) return;
      const next = radios[(i + d + radios.length) % radios.length];
      setOs(next.dataset.os);
      next.focus();
      e.preventDefault();
    });
  });
  const button = $("[data-copy-cmd]", box);
  button.addEventListener("click", () => copyCommand(setOs.current, button));
});
setOs(system === "windows-x86_64" ? "ps" : "sh");

/* Phones and tablets: no install command up front, a link to take to a computer instead */
if (system === "phone") {
  root.classList.add("is-phone");
  const link = ($('link[rel="canonical"]') || {}).href || location.href;
  $$("[data-phone]").forEach((box) => {
    box.hidden = false;
    const toggle = $("[data-show-install]", box);
    const target = document.getElementById(toggle.getAttribute("aria-controls"));
    target.hidden = true;
    toggle.addEventListener("click", () => {
      const open = target.hidden;
      target.hidden = !open;
      toggle.setAttribute("aria-expanded", String(open));
      toggle.textContent = open ? "Hide the install commands" : "Show the install commands";
    });
    const button = $("[data-copy-link]", box);
    button.addEventListener("click", async () => {
      let ok = true;
      try { await navigator.clipboard.writeText(link); } catch (e) { ok = false; }
      copyStatus.textContent = ok ? "Copied the link to this page." : "This browser blocked copying. The address is " + link;
      if (!ok) return;
      $("[data-copy-label]", button).textContent = "Link copied";
      $("[data-copy-icon]", button).textContent = "\uE182";
      button.classList.add("is-copied");
      clearTimeout(button.copyTimer);
      button.copyTimer = setTimeout(() => {
        $("[data-copy-label]", button).textContent = "Copy the link to this page";
        $("[data-copy-icon]", button).textContent = "\uE1CA";
        button.classList.remove("is-copied");
      }, 2400);
    });
  });
}

/* Direct links to the newest build from the public update feed; the releases page links stay if it fails */
function applyManifest(m) {
  if (!m || !/^\d+$/.test(String(m.version)) || typeof m.platforms !== "object" || m.platforms === null) return;
  const urls = {};
  for (const [key, asset] of Object.entries(m.platforms)) {
    if (asset && typeof asset.url === "string" && ASSET.test(asset.url)) urls[key] = asset.url;
  }
  $$("[data-platform]").forEach((row) => {
    const url = urls[row.dataset.platform];
    const a = $("[data-href]", row);
    if (url && a) {
      a.href = url;
    }
  });
}
const ctl = new AbortController();
setTimeout(() => ctl.abort(), 6000);
fetch(MANIFEST, { signal: ctl.signal, credentials: "omit", referrerPolicy: "no-referrer", cache: "no-cache" })
  .then((r) => (r.ok ? r.json() : null))
  .then(applyManifest)
  .catch(() => {});

/* Midnight and Daylight, side by side */
$$("[data-compare]").forEach((fig) => {
  const frame = $(".compare-frame", fig);
  const range = $("input[type=range]", fig);
  const set = (v) => {
    fig.style.setProperty("--split", v + "%");
    range.setAttribute("aria-valuetext", v + " percent Midnight");
  };
  range.addEventListener("input", () => set(range.value));
  let dragging = false;
  const at = (x) => {
    const r = frame.getBoundingClientRect();
    const v = Math.round(Math.min(100, Math.max(0, ((x - r.left) / r.width) * 100)));
    range.value = v;
    set(v);
  };
  frame.addEventListener("pointerdown", (e) => { dragging = true; frame.setPointerCapture(e.pointerId); at(e.clientX); });
  frame.addEventListener("pointermove", (e) => { if (dragging) at(e.clientX); });
  frame.addEventListener("pointerup", () => { dragging = false; });
  frame.addEventListener("pointercancel", () => { dragging = false; });

  // Standard or Deeper: the same two renders in the other style
  const styles = $$("[data-style]", fig);
  const images = $$("[data-standard]", frame);
  const setStyle = (style) => {
    styles.forEach((b) => {
      const on = b.dataset.style === style;
      b.setAttribute("aria-checked", String(on));
      b.tabIndex = on ? 0 : -1;
    });
    images.forEach((img) => {
      img.src = img.dataset[style];
      img.alt = img.alt.replace(/(Standard|Deeper) style/, style === "deeper" ? "Deeper style" : "Standard style");
    });
  };
  styles.forEach((b, i) => {
    b.addEventListener("click", () => setStyle(b.dataset.style));
    b.addEventListener("keydown", (e) => {
      const d = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
      if (!d) return;
      const next = styles[(i + d + styles.length) % styles.length];
      setStyle(next.dataset.style);
      next.focus();
      e.preventDefault();
    });
  });
});

/* The palette. With a keyboard and a mouse it is a command palette; on touch screens it stays the
   plain list of sections written into the page. */
const palette = $("[data-palette]");
const input = $("#pal-input");
const list = $("#pal-list");
const search = $("[data-pal-search]");
const foot = $("[data-pal-foot]");
const body = $("[data-pal-body]");
const sectionLinks = $$("a", list).map((a) => ({ href: a.getAttribute("href"), label: a.textContent.trim(), icon: $(".ph", a).textContent, view: a.dataset.view }));
const staticList = list.innerHTML;
const go = (href) => () => {
  const target = $(href);
  target.scrollIntoView();
  target.setAttribute("tabindex", "-1");
  target.focus({ preventScroll: true });
};
const commands = [
  ...sectionLinks.map((s) => ({ group: "Go to", label: s.label, icon: s.icon, view: s.view, keys: s.href === "#download" ? ["Ctrl", "Enter"] : [], run: go(s.href) })),
  { group: "Actions", label: "Copy the Linux install command", icon: "", keys: [], run: () => { setOs("sh"); copyCommand("sh", $("[data-install] [data-copy-cmd]")); } },
  { group: "Actions", label: "Copy the Windows install command", icon: "", keys: [], run: () => { setOs("ps"); copyCommand("ps", $("[data-install] [data-copy-cmd]")); } },
  { group: "Actions", label: "Theme: Midnight", icon: "", keys: [], run: () => setTheme("dark") },
  { group: "Actions", label: "Theme: Daylight", icon: "", keys: [], run: () => setTheme("light") },
  { group: "Actions", label: "Theme: match the system", icon: "", keys: [], run: () => setTheme("auto") },
];
let shown = [];
let active = 0;

/* The window under the headline shows the view of the palette's active row, one real render per
   section. Until the visitor reaches for the palette, it steps through them by itself. */
const stage = $("[data-stage]");
const views = $$(".view", stage);
let shownView = "history";
function showView(name) {
  const next = views.find((v) => v.dataset.view === name);
  if (!next || name === shownView) return;
  views.forEach((v) => {
    v.classList.toggle("is-on", v === next);
    v.setAttribute("aria-hidden", String(v !== next));
  });
  shownView = name;
}
const TOUR_MS = 4200;
let tourTimer = 0;
let tourOff = false;
let heroInView = true;
palette.style.setProperty("--tour", TOUR_MS + "ms");
function tourStep() {
  const stops = shown.map((c, i) => (c.view ? i : -1)).filter((i) => i >= 0);
  if (!stops.length) return;
  active = stops.find((i) => i > active) ?? stops[0];
  render();
}
function tourSync() {
  const run = !tourOff && !calm.matches && !search.hidden && heroInView && !document.hidden && !input.value;
  palette.classList.toggle("is-touring", run);
  if (run && !tourTimer) tourTimer = setInterval(tourStep, TOUR_MS);
  if (!run && tourTimer) { clearInterval(tourTimer); tourTimer = 0; }
}
function stopTour() {
  if (tourOff) return;
  tourOff = true;
  tourSync();
}

function fuzzy(label, q) {
  let i = 0;
  for (const ch of label.toLowerCase()) if (i < q.length && ch === q[i]) i += 1;
  return i === q.length;
}
function filter(q) {
  if (!q) return commands;
  const exact = commands.filter((c) => c.label.toLowerCase().includes(q));
  return exact.length ? exact : commands.filter((c) => fuzzy(c.label, q));
}
function highlight(label, q) {
  const out = document.createDocumentFragment();
  const at = q ? label.toLowerCase().indexOf(q) : -1;
  if (at < 0) { out.append(label); return out; }
  const m = document.createElement("mark");
  m.textContent = label.slice(at, at + q.length);
  out.append(label.slice(0, at), m, label.slice(at + q.length));
  return out;
}
function render() {
  const q = input.value.trim().toLowerCase();
  shown = filter(q);
  active = Math.min(active, Math.max(0, shown.length - 1));
  list.replaceChildren();
  if (!shown.length) {
    const li = document.createElement("li");
    li.className = "pal-empty";
    li.setAttribute("role", "presentation");
    li.textContent = "Nothing matches “" + input.value.trim() + "”. Try “download” or “theme”.";
    list.append(li);
    input.removeAttribute("aria-activedescendant");
    return;
  }
  let group = null;
  shown.forEach((c, i) => {
    if (c.group !== group) {
      group = c.group;
      const g = document.createElement("li");
      g.className = "pal-group";
      g.setAttribute("role", "presentation");
      g.textContent = group;
      list.append(g);
    }
    const li = document.createElement("li");
    li.id = "pal-opt-" + i;
    li.setAttribute("role", "option");
    li.setAttribute("aria-selected", String(i === active));
    const icon = document.createElement("i");
    icon.className = "ph";
    icon.setAttribute("aria-hidden", "true");
    icon.textContent = c.icon;
    const label = document.createElement("span");
    label.className = "pal-label";
    label.append(highlight(c.label, q));
    li.append(icon, label);
    if (c.keys.length) {
      const keys = document.createElement("span");
      keys.className = "pal-keys";
      keys.setAttribute("aria-hidden", "true");
      c.keys.forEach((k) => { const kb = document.createElement("kbd"); kb.textContent = k; keys.append(kb); });
      li.append(keys);
    }
    li.addEventListener("click", () => { active = i; run(); });
    li.addEventListener("pointermove", () => { if (active !== i) { active = i; render(); } });
    list.append(li);
  });
  input.setAttribute("aria-activedescendant", "pal-opt-" + active);
  showView(shown[active].view);
  edgeFade(list);
  const el = document.getElementById("pal-opt-" + active);
  if (el && list.scrollHeight > list.clientHeight) {
    const top = el.offsetTop - list.offsetTop;
    if (top < list.scrollTop) list.scrollTop = top - 6;
    else if (top + el.offsetHeight > list.scrollTop + list.clientHeight) list.scrollTop = top + el.offsetHeight - list.clientHeight + 6;
  }
}
function run() {
  const c = shown[active];
  if (!c) return;
  input.value = "";
  active = 0;
  render();
  c.run();
}
input.addEventListener("input", () => { active = 0; render(); });
input.addEventListener("keydown", (e) => {
  const n = Math.max(1, shown.length);
  if (e.key === "ArrowDown") { active = (active + 1) % n; render(); e.preventDefault(); }
  else if (e.key === "ArrowUp") { active = (active - 1 + n) % n; render(); e.preventDefault(); }
  else if (e.key === "Home" && !input.value) { active = 0; render(); e.preventDefault(); }
  else if (e.key === "End" && !input.value) { active = n - 1; render(); e.preventDefault(); }
  else if (e.key === "Enter" && !(e.ctrlKey || e.metaKey)) { run(); e.preventDefault(); }
  else if (e.key === "Escape") { input.value = ""; active = 0; render(); input.blur(); e.preventDefault(); }
});

const desktop = window.matchMedia("(min-width: 721px) and (not (pointer: coarse))");
function paletteMode() {
  const on = desktop.matches && !root.classList.contains("is-phone");
  search.hidden = !on;
  foot.hidden = !on;
  views.forEach((v) => { v.hidden = !on && v.dataset.view !== "history"; });
  $("[data-pal-title]").hidden = on;
  if (!on) showView("history");
  if (on) {
    body.removeAttribute("aria-label");
    body.setAttribute("role", "presentation");
    list.setAttribute("role", "listbox");
    list.setAttribute("aria-label", "Commands");
    list.tabIndex = -1;
    render();
  } else {
    body.removeAttribute("role");
    body.setAttribute("aria-label", "On this page");
    list.removeAttribute("role");
    list.removeAttribute("aria-label");
    list.removeAttribute("tabindex");
    list.innerHTML = staticList;
  }
  edgeFade(list);
}
paletteMode();
desktop.addEventListener("change", () => { paletteMode(); tourSync(); });

list.addEventListener("scroll", () => edgeFade(list), { passive: true });
palette.addEventListener("pointerenter", stopTour);
palette.addEventListener("focusin", stopTour);
calm.addEventListener("change", tourSync);
document.addEventListener("visibilitychange", tourSync);
if ("IntersectionObserver" in window) {
  new IntersectionObserver(([entry]) => { heroInView = entry.isIntersecting; tourSync(); }, { threshold: 0.3 }).observe($(".hero"));
}
tourSync();

/* Keys: Ctrl+K the palette, Ctrl+Enter downloads, J and K between sections */
const sections = $$("main > section");
function typing(el) {
  return el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)) && el.type !== "range";
}
function stepSection(dir) {
  const y = window.scrollY + 80;
  const tops = sections.map((s) => s.getBoundingClientRect().top + window.scrollY);
  let i = tops.findIndex((t) => t > y + 4);
  if (i === -1) i = tops.length;
  const target = dir > 0 ? sections[i] : sections[Math.max(0, i - 2)];
  if (target) target.scrollIntoView();
}
document.addEventListener("keydown", (e) => {
  const mod = e.ctrlKey || e.metaKey;
  if (mod && !e.altKey && e.key.toLowerCase() === "k" && !search.hidden) {
    e.preventDefault();
    palette.scrollIntoView({ block: "center" });
    input.focus({ preventScroll: true });
    input.select();
    return;
  }
  if (mod && !e.altKey && e.key === "Enter") {
    e.preventDefault();
    go("#download")();
    return;
  }
  if (mod || e.altKey || typing(document.activeElement)) return;
  if (e.key === "j" || e.key === "J") stepSection(1);
  else if (e.key === "k" || e.key === "K") stepSection(-1);
});

/* The rail: each section's lane draws in as it scrolls into view, and its commit lights when the
   line reaches the heading. Sections and pictures rise into place once. Neither with reduced motion. */
const railSections = $$("main > .sec");
let railFrame = 0;
function drawRail() {
  railFrame = 0;
  const reach = window.innerHeight * 0.82;
  railSections.forEach((sec) => {
    const box = sec.getBoundingClientRect();
    const heading = $("h2", sec).getBoundingClientRect();
    sec.style.setProperty("--draw", Math.min(1, Math.max(0, (reach - box.top) / box.height)).toFixed(3));
    sec.classList.toggle("is-lit", reach > heading.top);
  });
}
function motionSync() {
  const on = !calm.matches;
  root.classList.toggle("rail-on", on);
  root.classList.toggle("reveal-on", on && "IntersectionObserver" in window);
  if (on) drawRail();
  else railSections.forEach((sec) => sec.style.removeProperty("--draw"));
}
window.addEventListener("scroll", () => { if (!calm.matches && !railFrame) railFrame = requestAnimationFrame(drawRail); }, { passive: true });
window.addEventListener("resize", () => { if (!calm.matches) drawRail(); }, { passive: true });
calm.addEventListener("change", motionSync);
if ("IntersectionObserver" in window) {
  const reveal = new IntersectionObserver((entries) => {
    entries.forEach((e) => {
      if (!e.isIntersecting) return;
      e.target.classList.add("is-in");
      reveal.unobserve(e.target);
    });
  }, { rootMargin: "0px 0px -8% 0px", threshold: 0.12 });
  $$("[data-reveal]").forEach((el) => reveal.observe(el));
}
motionSync();
