# Accessibility

Canopy aims to meet WCAG 2.2 level AA where it applies to desktop software, and to work with
screen readers through AccessKit (AT-SPI on Linux, UI Automation on Windows, NSAccessibility on
macOS).

What Canopy does today:

- Commands are reachable from the keyboard through a command palette and a shortcut cheatsheet
  ([docs/keybindings.md](docs/keybindings.md)).
- Screen reader basics: the main widgets publish names, roles and states to assistive technology;
  full screen reader support is still planned.
- Motion follows the operating system's reduced-motion setting, and translucent surfaces (off by
  default) follow its reduced-transparency setting. Every theme has a high-contrast variant
  (Settings → Appearance), and the interface zooms from 80% to 160%.

Known gaps are tracked as issues: the full audit with a screen reader (#63), announcements and
disabled-button labels (#22), focus return across overlays (#24), complete keyboard reach (#176), and
following the system's contrast setting with zoom to 200% (#206). Search the [issues labelled `area:accessibility`](https://github.com/NecturaLabs/Canopy/issues?q=is%3Aissue+is%3Aopen+label%3Aarea%3Aaccessibility).

To report a barrier, file a bug with the "Accessibility" area, naming the assistive technology,
operating system and the step that fails.
