# Canopy UI spec: controls, type, motion, layout

**Recommendation.** Keep the current palette and the Inter + JetBrains Mono pairing, and tighten the system in four ways.
- Controls: three heights (24/28/32), a 2px outside focus ring shown only for keyboard focus, and a 3:1 `control_border` token for form controls.
- Type: integer sizes, with the current 13.5/12.5/11.5 values collapsed to 13/13/12.
- Motion: three tokens (100/150/200 ms, ease-out), and instant for anything keyboard-driven.
- Layout: breakpoints in egui points, with the window minimum guaranteeing three panes.

Omarchy ships Hyprland with rounding 0 and blur off (snippet), which supports flat, low-chrome controls, no gradients on controls, and visible keyboard hints.

**Mismatches found in the repo:**
- `design.md` says body is 13, the code uses 13.5 (`theme.rs:541`).
- `design.md` says minimum window 960x600, `main.rs:170` says 900x560.
- Settings booleans are "On/Off" `selectable_value` buttons (`settings_screen.rs:441`).
- Hover fades in `toolbar.rs`, `history.rs`, `sidebar.rs`, `commit_detail.rs` and `widgets.rs:191` use literal durations, so they ignore reduce-motion. Only `primary_button` goes through `theme::fade`.

## 1. Controls (points, 13 px body)

### How other clients do it

| Client | Notable detail |
|---|---|
| Zed | Button heights 32/28/22/18 and a Switch with a `focus_visible` border (source read). Tooltips right-align the keybinding (source read). Hover popover delay 300 ms (snippet). |
| GitHub Primer | Buttons 28/32/40 (snippet). GitHub Desktop has a fixed 11px small text; the request to raise it was closed "not-planned". |
| Fluent 2 | Buttons 24/32/40 (snippet). Toggle switch for immediate effect, check box for optional items (snippet). |
| libadwaita | Entry min-height 34, padding 9, radius 9. Button min 24 plus 5/10 padding. Switch thumb 20 with 3px padding. Check box 14 with radius 6. Card-button focus ring offset -1. All from SCSS read. Prefers switches over checkboxes (snippet). Sized for touch, so too big for Canopy. |
| macOS HIG | Toggle is for immediate effect, checkbox for deferred. Don't replace an existing checkbox with a switch. Don't use a switch for a minor single setting (snippet). |
| VS Code, Linear | No control specs confirmed. Linear's March 2026 refresh dims the sidebar and uses Inter Display for headings. |
| Fork, Tower, Sublime Merge | No public control specs found (unverified). Sublime Merge has `dpi_scale` and an open "fixed UI font size" request (#851). |

The recurring complaint is text that is too small and cannot be scaled. Keep zoom and never go below 11 pt.

### Tokens

| Token | Value |
|---|---|
| Control height | sm 24, md 28 (default; was 26), lg 32 (dialogs, primary CTA) |
| Min hit target | 24x24 everywhere (WCAG 2.5.8 AA, snippet). Toolbar and icon buttons 28x28. |
| Radii | xs 4 (checkbox, kbd, inner segment), sm 6 (buttons, inputs, chips), md 8 (menus, popovers), lg 12 (dialogs). Add `RADIUS_XS`. |
| Strokes | 1.0 hairline, 2.0 focus. Drop the 1.5 values (blurry at 1x). |
| State layers | Hover 8%, press 12% (M3 uses 8/10/10, snippet). Selection stays at 18%/12% accent plus the 2px left bar. |
| Disabled | Text goes to `text_faint`, fill transparent, border `border`. Set `visuals.disabled_alpha` to 0.4 (M3 uses about 38%). Explain why with `on_disabled_hover_text`. |

### Contrast tokens

I computed these from the Midnight hex values.
- `accent` on `bg` is 4.5:1, so it works as a focus ring.
- `border` (#262D3B) on `bg` is only about 1.4:1. That is fine for panel hairlines but fails WCAG 1.4.11 (3:1) for form-control boundaries.
- Add a `control_border` token at 3:1 or more against `panel` and `bg`. `text_faint` on `panel` is about 3.4:1, so it is a valid starting point.
- Use `control_border` for input, combo, checkbox, radio, switch-off track and slider rail.
- Add theme tests alongside the existing contrast test: `control_border` against `panel`/`bg` at 3:1, and `accent` against `panel`/`elevated` at 3:1.

### Controls

- **Switch vs checkbox.**
  - Switch: settings that take effect immediately (the settings screen, replacing On/Off).
  - Checkbox: dialog forms confirmed by a button, multi-select lists (staging), tri-state, and sub-options ("Also delete remote branch").
  - egui has no switch, so write a custom one: track 32x18 (radius 9), thumb 12, 3px inset, hit area is the whole row, thumb animation `fast`.
- **Checkbox.** 16x16, radius 4, 1px `control_border`. Checked is a solid accent fill with a 2px check stroke. egui's own checkbox does not fill when checked, so this needs a custom paint. Set `icon_width` 16 and `icon_spacing` 8.
- **Radio.** 16 circle with a 6 dot. Use it for 2-4 options that need descriptions. Otherwise use a segmented control (2-5 short options) or a combo (more than 5).
- **Segmented.** Height 28, `bg` well with a 1px border, 2px inset, selected segment on `elevated` with a 1px border. One tab stop with Left/Right to move. Never use it for on/off.
- **Slider.** Set `slider_rail_height` 4 (egui default 8), handle `Circle{radius 7}`, value shown in a 56 px `DragValue`. egui's trailing fill and `ProgressBar` both use `selection.bg_fill`, which Canopy sets translucent (alpha 90). Paint the fill custom or it will look weak.
- **Text input.**
  - Height 28 (32 in dialogs and settings), padding x 8, 1px `control_border`, radius 6, fill `bg`.
  - Focus: egui already swaps the stroke to `selection.stroke` on focus, so set that to 2.0 accent.
  - Placeholder: `text_faint`.
  - Error: `danger` border, plus an icon and a caption-12 message below. Never colour alone.
- **Combo.** Same height as an input, 12 px caret icon. Popup items 28 high with a check on the selected one. Set `combo_height` to about 320. Add a filter field above 8 items.
- **Buttons.**
  - Primary: flat accent fill, `on_button` text, one per surface. Drop the hover glow ring in `primary_button` and use hover +8% / press -8% brightness. Keep the gradient for the welcome mark and progress only.
  - Secondary: `elevated` fill, 1px border.
  - Ghost: transparent, 8% tint on hover. Used for toolbar and icon buttons (glyph 16 in 28x28).
  - Danger: keep the tinted fill with `danger` text and border. White on #F87171 is about 2.8:1, so do not use a solid danger fill in Midnight.
  - Dialog button labels name the action (already in `design.md`).
- **Chips.** 22 high, radius 6, with a 14 px remove icon. Count badges are fully round, 16-18 high. The ref pill stays at 18.
- **Kbd chip.** 18 high, JetBrains Mono 11, `text_dim` on `hover`, radius 4, padding 5.
- **Tooltips.**
  - Set `tooltip_delay` 0.4 (egui default 0.5), `tooltip_grace_time` 0.25, keep `show_tooltips_only_when_still`, `tooltip_width` 320.
  - Content is label plus kbd chips right-aligned.
  - Show the tooltip immediately on keyboard focus with `Response::show_tooltip_text`. Esc dismisses it (WCAG 1.4.13, snippet).
- **Focus ring.**
  - 2px accent, outside the widget with a 1px gap, radius = control radius + 2. Focused list rows get an inset ring instead.
  - egui has no focus ring. `Widgets::style` just returns `active` for pressed or focused, so keyboard focus looks identical to a press.
  - Track an app-level `keyboard_nav` flag (set on Tab, arrows or j/k, cleared on pointer press) and paint the ring only when it is set.
  - Paint it with an expanded clip rect, or edge buttons get cut off.
  - The result is the same as the CSS `:focus-visible` behaviour.

## 2. Typography

**Verdict: keep the proposal, with these changes.** Drop `hero` (use `display` 24). Align the code to the doc (13/13/12). Never go below 11 pt. Tabular numerals need mono, because egui exposes no OpenType features: `FontTweak` has only scale, offsets, hinting, variation coords, thin-space and tab width.

| Style | Font | Size / line | Notes |
|---|---|---|---|
| micro | Inter SemiBold, caps | 11 / 14 | `extra_letter_spacing` +0.7 px (about 0.06em). Section headers and kbd only. |
| caption | Inter Regular | 12 / 16 | Dates, authors, status bar. Replaces 11.5. |
| body | Inter Regular | 13 / 18 | Labels and buttons use Medium. Row heights 24-28. |
| code_sm | JetBrains Mono | 12 / 16 | SHAs in tables, kbd. |
| code | JetBrains Mono | 13 / 20 | Diffs, commit editor (replaces 12.5). |
| subtitle | Inter SemiBold | 14 / 20 | |
| title | Inter SemiBold | 16 / 22 | Tracking -0.17 px. |
| display | Inter SemiBold | 24 / 30 | Tracking -0.47 px. |

- **Letter-spacing.** Inter's dynamic-metrics formula is tracking = -0.0223 + 0.185·e^(-0.1745·px) em (snippet). That gives about 0 at 13, -0.011em at 16 and -0.02em at 24. Apply it via `RichText::extra_letter_spacing`.
- **Weights.** Use 400, 500 and 600 only.
- **Icons.** 12 (chevrons in inputs, new), 14, 16 (default), 20, 32. Use regular weight and the fill weight for state, as in `design.md`.
- **Mono usage.**
  - Use mono for SHAs, diff gutters, stderr, kbd, the commit editor and right-aligned numeric columns.
  - Use Inter Medium for branch and ref names. File paths in lists are Inter with a dim directory part.
- **Egui mapping.** Add custom `TextStyle::Name` entries for micro, subtitle, display and code_sm.
- **Phase 2.** Freeze `tnum` into an "Inter Tabular" derivative with fonttools so counts and dates in Inter columns align. The OFL permits this, but check the Reserved Font Name clause in `OFL-Inter.txt` (unverified).

## 3. Motion

NN/g gives about 100 ms for simple feedback and 200-300 ms for large changes (snippet). Fluent v9 uses 150/200 ms (snippet). M3 durations are 100 ms (short2) and 250 ms (medium1), and exits are shorter than enters (snippet). Apple HIG: "avoid adding motion to interactions that occur frequently" (snippet).

| Token | Value | Use |
|---|---|---|
| instant | 0 | Anything driven by keyboard or high-frequency use |
| fast | 100 ms | Button hover/press, switch thumb, checkbox, segmented thumb |
| base | 150 ms | Popover/menu/tooltip fade-in, expand/collapse, toast exit |
| slow | 200 ms | Dialog enter (fade + 0.98 to 1.0 scale), toast enter (8px slide + fade) |

Easing is `emath::easing::cubic_out` for enters and hover. Exits are about 75% of the enter duration and fade only. Spinners are linear at 1 revolution per second.

**Do NOT animate:**
- row hover (remove the 100-120 ms fades in the history list and sidebar)
- selection movement and focus ring movement (remove the 180 ms "selection highlight move" from `design.md`)
- tab switching (indicator and content instant)
- palette result filtering
- keyboard scroll-to-selection (egui's default `scroll_animation` runs 0.1-0.3 s, so set `ScrollAnimation::none()`)
- list updates from background fetch (instant; only a user-initiated insert may fade in at 150 ms)
- menu, tooltip and palette close

The palette fades in over 100 ms with no scale. Dialogs close in 100 ms with fade only.

**Reduce motion.**
- Make it a tri-state `System | Reduced | Full`, default System (VS Code `workbench.reduceMotion` is auto/on/off, snippet).
- Reduced means no slide or scale, crossfade only (libadwaita 1.9, snippet), static spinners (Zed, snippet), and caret blink off.
- Detection runs as a worker Job at startup and on window focus, and returns an Event. The UI thread never calls it.
  - **Linux:** the xdg portal `org.freedesktop.appearance` key `reduced-motion` (0/1), added in portal Settings v2. Read it with `ashpd` or the `gdbus` CLI (the CLI avoids a new dependency). Fall back to `gsettings get org.gnome.desktop.interface enable-animations` where false means reduced.
  - **macOS:** `NSWorkspace.accessibilityDisplayShouldReduceMotion` via `objc2-app-kit`, already in `Cargo.lock` (0.3.2). Whether the getter is safe Rust is unverified.
  - **Windows:** `SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION)`, where FALSE means reduced. The `windows` crate has it as `unsafe`, and `AGENTS.md` forbids unsafe workspace-wide. The WinRT `UISettings::AnimationsEnabled` call may be safe, but its minimum Windows version is unverified. Decide with the owner, or use the setting alone on Windows.
- Until detection exists, the setting is the fallback.

**egui implementation.**
- Use `animate_bool_with_time_and_easing(id, v, secs, cubic_out)`. It only calls `request_repaint` while 0 < t < 1 (read in `context.rs`), so idle cost is zero. A 150 ms transition is about 9 frames at 60 Hz.
- A time of 0 returns the end value immediately (checked in `animation_manager.rs`).
- Route every duration through one `motion` helper that returns 0 when reduced, and set `style.animation_time` to base (0.15).
- Text-cursor blink calls `request_repaint_after_secs` forever while a field is focused (read in `text_selection/visuals.rs`). Disable `blink` under reduced motion.
- `welcome.rs:39` repaints every 33 ms, so gate it on visibility and focus.

## 4. Responsive layout (egui points, so zoom is already applied)

**Window minimum.** 960x600. The pane minimums are sidebar 180 + centre 420 + detail 320 = 920, so all three panes always fit.
- Fix `main.rs` to match.
- egui points = OS logical size / zoom. Call `ViewportCommand::MinInnerSize(960*zoom, 600*zoom)` when zoom changes, clamped to the monitor.

Scale 1x-2x is handled by egui's pixels_per_point. Use integer point sizes (fractional font sizes aside) and test at 1.25 and 1.5. Text size never changes with breakpoints, because only the density of the chrome adapts. Scaling stays user-driven through zoom.

| Width (pt) | Behaviour |
|---|---|
| 960-1099 | Toolbar icon-only (32x32 with tooltips). Sidebar 200. Status bar drops secondary segments. |
| 1100-1599 | Toolbar with labels. Sidebar 240. Detail 40% of the remainder, clamped to 320-560. |
| 1600-2559 | Sidebar 260. Detail clamped to 360-640. Side-by-side diff on by default. |
| 2560 and up | Cap detail at 720. Dialogs and settings content max 720, centred. The table gets the extra width. |

**History columns** hide by centre-pane width: below 720 hide SHA, below 560 hide author, and the date stays as a relative time. The message never goes below 240.

**Toolbar** never wraps. When it overflows, items collapse into a "…" menu. Palette, Fetch, Pull and Push stay visible. Branch, Stash and the view toggle collapse first.

**Status bar** is one line. The left side holds branch, ahead/behind and in-progress state. The right side holds the job and the path. The path drops first, then secondary segments. Never wrap.

**Truncation.**
- Single-line labels use `Label::truncate()`.
- Show a tooltip with the full text only when the label is actually truncated (compare natural galley width to available).
- Paths truncate the directory part, never the filename.
- Ref pills collapse to `+N` (already in `design.md`).

## Risks and open questions

- Is a derived Inter font acceptable, or should tabular figures stay mono-only? (OFL check, see above.)
- Is the owner willing to allow a small `unsafe`-bearing crate for Windows and macOS reduce-motion detection? Otherwise Windows falls back to the setting alone.
- Flat primary buttons contradict `design.md`'s "primary = accent gradient". This needs owner sign-off, along with updating `docs/design.md`.
- A `control_border` at 3:1 is visually heavier than the current hairline. Review it in screenshots, dark and light, across all seven themes.
- Optional: a "Corner style" setting (Sharp/Soft) for the Omarchy look.
- egui does not tell keyboard focus from mouse focus, so the `keyboard_nav` flag is app code. Whether a click gives a button focus in egui 0.36 is unverified.

## Sources

- `/home/user/Canopy/docs/design.md`, `/home/user/Canopy/crates/canopy-app/src/ui/theme.rs`, `/home/user/Canopy/crates/canopy-app/src/ui/widgets.rs` — the current tokens and widgets.
- `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/egui-0.36.2/src/{style.rs,context.rs,animation_manager.rs,widgets/}` and `epaint-0.36.2/src/text/fonts.rs` — egui defaults, animation and repaint behaviour, no focus ring, no font features.
- https://raw.githubusercontent.com/zed-industries/zed/main/crates/ui/src/components/button/button_like.rs — Zed button heights (read for numbers only; the repo is GPL).
- https://raw.githubusercontent.com/zed-industries/zed/main/crates/ui/src/components/toggle.rs — Zed switch and checkbox dimensions and focus.
- https://raw.githubusercontent.com/zed-industries/zed/main/crates/ui/src/styles/typography.rs — Zed text sizes.
- https://github.com/GNOME/libadwaita/blob/main/src/stylesheet/widgets/_entries.scss — libadwaita entry 34px, padding 9, focus ring.
- https://github.com/GNOME/libadwaita/blob/main/src/stylesheet/widgets/_buttons.scss — libadwaita button sizes and focus-ring offsets.
- https://github.com/GNOME/libadwaita/blob/main/src/stylesheet/_common.scss — 200 ms ease-out transitions, radius 9.
- https://github.com/GNOME/libadwaita/blob/main/src/stylesheet/widgets/_switch.scss — libadwaita switch sizes.
- https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.portal.Settings.xml — the `reduced-motion` and `contrast` keys, portal v2.
- https://github.com/longbridge/gpui-kit/pull/3094 — per-platform reduced-motion reading (ashpd, objc2-app-kit, SPI_GETCLIENTAREAANIMATION).
- https://github.com/MicrosoftDocs/win32/blob/docs/desktop-src/WinAuto/client-area-animation.md — SPI_GETCLIENTAREAANIMATION.
- https://developer.apple.com/documentation/appkit/nsworkspace/accessibilitydisplayshouldreducemotion — the macOS API (snippet).
- https://github.com/desktop/desktop/issues/3543 — GitHub Desktop 11px complaint, closed not-planned.
- https://github.com/sublimehq/sublime_merge/issues/851 — Sublime Merge fixed UI font complaint.
- https://github.com/microsoft/fluentui/issues/31191 — Fluent easing curves.
- https://www.nngroup.com/articles/animation-duration/ — 100 ms feedback, 200-300 ms large changes (snippet).
- https://developers.apple.com/design/human-interface-guidelines/foundations/motion — avoid motion on frequent interactions, make motion optional (snippet).
- https://developers.apple.com/design/human-interface-guidelines/components/selection-and-input/toggles/ — toggle vs checkbox (snippet).
- https://developers.apple.com/design/human-interface-guidelines/macos/visual-design/typography/ — macOS text styles (snippet).
- https://developer.gnome.org/hig/patterns/controls/switches.html — GNOME prefers switches (snippet).
- https://learn.microsoft.com/en-us/windows/apps/design/controls/toggles — toggle vs checkbox guidance (snippet).
- https://d.rsms.me/inter-website/v3/dynmetrics/ — Inter tracking formula (snippet).
- https://primer.github.io/design/components/button/ — Primer button sizes (snippet).
- https://zed.dev/docs/reference/all-settings — Zed font sizes, hover delay, `reduce_motion` (snippet).
- https://learn.omacom.io/2/the-omarchy-manual — Omarchy defaults, rounding 0 and blur off (snippet).
- https://code2020.dev/reduce-motion/ — VS Code `workbench.reduceMotion` auto/on/off (snippet).
- https://nyaa.place/blog/libadwaita-1-9/ — libadwaita 1.9 reduced-motion crossfades (snippet).
- WCAG 2.2 (2.5.8 target size, 1.4.11 non-text contrast, 1.4.13 content on hover or focus), from search summaries only. Note: 2.4.11 is Focus Not Obscured and 2.4.13 is Focus Appearance (AAA); one summary I saw mislabelled them.
- https://m3.material.io/styles/motion/easing-and-duration/tokens-specs — M3 duration and easing tokens (snippet).