# Accessibility

Canopy aims to meet WCAG 2.2 level AA where it applies to desktop software, and to work with
screen readers through AccessKit (AT-SPI on Linux, UI Automation on Windows, NSAccessibility on
macOS).

What Canopy does today:

- Commands are reachable from the keyboard through a command palette and a shortcut cheatsheet
  ([docs/keybindings.md](docs/keybindings.md)).
- Widgets publish names, roles and states to assistive technology.
- Reduced motion and reduced transparency follow the operating system; every theme has a
  high-contrast variant (Settings → Appearance), and the interface can be zoomed.

Known gaps are tracked as issues: the full audit with a screen reader (#63), announcements and
disabled-button labels (#22), focus return across overlays (#24) and complete keyboard reach
(#176). Search the [issues labelled `area:accessibility`](https://github.com/NecturaLabs/Canopy/issues?q=is%3Aissue+is%3Aopen+label%3Aarea%3Aaccessibility).

To report a barrier, file a bug with the "Accessibility" area, naming the assistive technology,
operating system and the step that fails.
