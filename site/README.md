# Canopy website

The landing page for Canopy: one static page, led by the keyboard, plus the Privacy Policy and
Terms of Service. Its first screen is the headline and the install command beside a working
command palette, with the app's window below them showing the view of the palette's active row,
and each section names the keys that do the job. There is no build step and no framework:
`index.html`, `styles.css`, `app.js` and `theme.js` are the source and the output.
`privacy.html` and `terms.html` are rendered from `../docs/legal/` by
`python3 tools/check-site.py --write-legal`; never edit them by hand. `check-site.py` fails while
either differs from that render, so a change to a legal document that skips the step fails CI.

## How it looks

The site has its own palette, set in the tokens at the top of `styles.css`: a near-black ground
(`#09090B`, light `#FBFBFC`), a white pill for the one call to action (near-black in light), and
the app's own colours from `crates/nectura-ui/src/theme.rs`: Midnight's indigo (`#7C86FF`,
text `#9AA2FF`; Daylight's `#5E6AD2`, text `#4651C0`) for links, focus and selection, and its
success green only for Copied. The screenshots keep the app's own Midnight and Daylight. Headings
are Inter Display, text is Inter, and keys, commands and paths are JetBrains Mono. Surfaces follow
the app's Deeper style: panes sit on the ground with layered soft shadows, controls and keycaps are
lit from the top, and every screenshot sits in a window frame.

The page's motif is the app's commit graph. Each section has a lane colour, taken from the graph's
lanes in `nectura-ui/src/theme.rs` (Midnight's in dark, Daylight's in light): History indigo, Changes green,
Conflicts pink, Requests amber, Git settings blue, Keyboard orange, Themes violet, Privacy teal,
Download indigo again. A rail in the left gutter carries that colour from section to section, with
a commit dot level with each heading; it keeps at least 24 px from the window's edge (below
1200 px the section text moves in to 48 px to make room) and is left out on phones, where there is
no room beside the text, and the section's ground and the light under its shots are
faintly tinted with it (an offset, soft shadow; no zero-offset halos). Sections alternate their
layout: text beside a shot, on either side, or a full view with an enlarged detail of it (a hunk,
the conflict card, a request row) laid over one corner; phones show the full view's crop only.

Motion: the palette arrives once; the hero window crossfades to the view of the palette's active
row (The hero window, below); each section's lane draws down as the section scrolls into view and
its dot lights when the line reaches the heading; text and shots rise into place once, details a
beat later. Reduced motion stops all of it and shows the finished state. No gradient text, glass,
badges or decorative icons, and no copy that is not a fact about the app.

## Run it locally

```sh
python3 -m http.server 8417 --bind 127.0.0.1 -d site
```

Then open <http://127.0.0.1:8417/>. Any static server works. Opening `index.html` from disk works
too, except the live build list, which browsers refuse from a `file:` page.

## Deploy to Netlify

The site is live at <https://canopy.necturalabs.com>. CI deploys it: the Site workflow
(`.github/workflows/site.yml`) runs on every push to `main` that touches `site/`, first
`python3 tools/check-site.py` (every file the page, styles and scripts name exists, every
address outside the site is allowed by the content security policy, the scripts parse), then
a separate deploy job uploads this folder as one zip to Netlify's REST API with curl (no npm
package runs while the token is present) and waits until Netlify reports it live. It needs
`NETLIFY_AUTH_TOKEN`, a secret of the `site` environment that only `main` can use, and the
repository variable `NETLIFY_SITE_ID`; nothing is set by hand in Netlify. To redeploy without a change, choose Run workflow on the Site workflow.

The whole folder is published as it is, this README included (Netlify reads `netlify.toml` and does not serve it). `netlify.toml`
sets the headers: a content security policy that allows only the site's own files plus
`raw.githubusercontent.com` for the build list, no referrer, no framing, and long caching for the
fonts. Every response carries `Cache-Control: no-transform`, so Cloudflare, which proxies the
domain, injects nothing (its JavaScript detections and email obfuscation would add inline scripts
the policy refuses). The page has no inline scripts, styles or event handlers, so the policy needs no
exceptions. It also holds the two installer addresses: `/install` and `/install.ps1` are 302
redirects to `canopy/install.sh` and `canopy/install.ps1` in NecturaLabs/app-updates, so the
script that runs is always the published one and this site never serves a copy. The canonical
address, `og:url` and `og:image` in `index.html` use `https://canopy.necturalabs.com`.
`og:image` is `./assets/og-image.png`, a 1200 by 630 PNG crop of a real History render, since some
link previews do not read WebP.

## How the page behaves

- **Theme.** Midnight (dark) is the default. `theme.js` applies a saved choice before the first
  paint. The switch in the header saves Daylight or Match the system in `localStorage` under
  `canopy-theme`; choosing Midnight again removes the saved choice. Without scripts the system's
  light or dark setting decides. Every screenshot has a
  Midnight (`m-`) and a Daylight (`d-`) render, and the page shows the one that matches.
- **Palette.** With a mouse or trackpad and a window wider than 720 px, the palette is a
  combobox: type to filter, arrows to move, Enter to run, Esc to clear. `Ctrl+K` focuses it,
  `Ctrl+Enter` goes to the downloads, and `J` and `K` move between sections when no text field
  has focus. On touch screens, on phones and tablets (whatever their pointer reports) and in
  narrow windows it stays the plain list of section links written into the page, placed under the
  screenshot: one column on phones, two on tablets, every row at least 48 px tall. On wide windows
  it sits beside the headline; below 960 px it moves under the screenshot.
- **The hero window.** It holds one real render per palette row with a view (History, Changes,
  the conflict resolver, pull requests, Settings → Git, the shortcuts sheet and Observatory for
  Themes), and crossfades to the active row's view whenever that row changes: by arrow key, by
  pointer or as the filter narrows the list. The tour steps through the rows every 4.2 seconds.
  The active row's background fills from left to right until the next step. The button below
  the palette pauses or plays the tour. Hovering over or focusing the palette pauses it until
  the visitor selects Play. The tour also stops while the hero is out of view or the tab is
  hidden. The button is hidden when the search field has text, when reduced motion is on or
  when the palette is a list of links. In link list mode, only the History render is in the page
  (the others are `hidden`, so they are never downloaded), under the title On this page. When the
  list is longer than its box, it fades out at the bottom until scrolled to the end.
- **Install command.** The main download is one command, chosen from the visitor's system:
  `curl -fsSL https://canopy.necturalabs.com/install | sh` on Linux and macOS and
  `irm https://canopy.necturalabs.com/install.ps1 | iex` on Windows; below 720 px the line
  scrolls sideways (fading at the edge while more follows) rather than breaking. A switch shows the other
  line, the copy button says Copied and a polite status message announces it. Android, iPhone and
  iPad visitors get no command up front: the page says Canopy is a desktop app for Linux (x86_64
  and arm64), macOS (Apple silicon and Intel) and Windows (x86_64) with no mobile app yet, offers to copy the page's link to open
  on a computer, and keeps the commands behind Show the install commands. The header's Download
  button and the archive downloads are hidden for them: there is nothing to install on a phone.
  Unrecognised systems see that Canopy runs on Linux, macOS and Windows; both lines stay
  available to them. The archives remain listed below the command as one card per build, the
  visitor's own marked with the white button (both macOS cards on a Mac, since a browser cannot
  tell Apple silicon from Intel), with a note that the macOS builds are not notarised. The footer
  groups its links in three labelled columns: Tour, Get Canopy and Legal.
- **Phones.** Below 720 px each full-window screenshot is replaced by a crop of the same render
  (`*-crop.webp`, `*-changes-hunk.webp`). Below 600 px the crops sit in a taller frame
  (`object-fit: cover`, anchored top left) so they show at about three quarters of their real size
  and their text stays readable; the hero shot is drawn at 150 % width for the same reason, and
  the shortcuts sheet is left out, since the rules beside it say the same; the enlarged details
  are left out too, as the crops already show those parts at that size. A picture wider than the
  screen (the hero window, the cropped full views) fades out over its last 56 px instead of ending
  in a hard cut. Phones and tablets
  (detected, or any coarse pointer) get controls at least 44 px tall, and the header, page edges
  and footer keep clear of notches through `viewport-fit=cover` and `env(safe-area-inset-*)`.
- **Themes.** The Midnight and Daylight slider has a Standard and Deeper switch that swaps both
  renders for the same view in the other style (`m-deeper`, `d-deeper`).
- **Words and pictures.** Copy names the real thing: the key, the feature, the exact behaviour.
  No superlatives, taglines or claims the sources under Keeping it true do not make. Every picture
  is a render of the app or one of its own controls, with no icon grids or stock imagery.

## What it loads

Everything is served from this folder: no third-party fonts, scripts, analytics or cookies. The one
request elsewhere is the public update manifest,
`https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/latest.json`, which the page
reads to point the download buttons straight at the newest archives. The page shows no build number;
the links written into it go to the latest releases page and stay in place if that request fails, and every link the manifest returns must match
the release-asset pattern in `app.js` before the page uses it.

## Keeping it true

- Every claim comes from the Done and Unreleased rows of `docs/features.md`, the product rules in
  `AGENTS.md`, `docs/keybindings.md` for the keys, and `docs/legal/privacy-policy.md` for the
  privacy section. When one of them changes, check the matching section here.
- The installer notes on the page come from the published `install.sh` and `install.ps1`
  themselves: per-user folders, no `sudo` or administrator rights, the SHA-256
  check against the manifest, upgrading by running it again, the uninstaller it leaves behind and
  the git 2.30 requirement. Read them again before changing those lines. The arm64 Linux archive
  is marked experimental, as the Release workflow builds it.
- The themes section names Midnight, Daylight, Observatory, the Custom theme and the High
  contrast switch, from `docs/features.md`. Midnight, Daylight, Observatory (`m-observatory`,
  `--theme observatory`, the hero's Themes view) and Custom (`m-custom`, `d-custom`: accent hue
  150, tint 40 at hue 150, built from Midnight and from Daylight) have screenshots, as has the
  Deeper style (`m-deeper`, `d-deeper`, `--style deeper`); do not show High contrast until there
  is a real render.
- The screenshots in `./assets/shots/` are real renders from `tools/screenshot.sh` on demo
  repositories made by `tools/mkfixture.sh`, at 1440 by 900, converted to WebP at quality 80
  (`magick in.png -strip -quality 80 -define webp:method=6 out.webp`); the crops are cut
  from those renders. The pull requests picture uses the app's invented demo data and says so on
  the page. Each picture records its origin in a `.json` file beside it. To refresh one, render it
  again and convert it to WebP, then cut the crop again at the offsets its `.json` file gives.
- The enlarged details are crops of the same renders: `*-changes-hunk.webp`, `*-conflict-card.webp`
  (784 by 188 at 646,326 of the conflict render) and `*-request-row.webp` (540 by 104 at 238,190
  of the requests render); the shortcuts sheet in the Keyboard section fades out over its last
  96 px, where its list goes on.
- The icons in `./assets/` are copies of the app icon in `../assets/`.

## Fonts and licences

| File | Font | Licence |
|---|---|---|
| `fonts/Inter-*.woff2` | Inter (Regular, Medium, SemiBold), Latin subset | SIL OFL 1.1, `fonts/OFL-Inter.txt` |
| `fonts/InterDisplay-SemiBold.woff2` | Inter Display SemiBold from Inter 4.1, cut to the same code points as the Inter files | SIL OFL 1.1, `fonts/OFL-Inter.txt` |
| `fonts/JetBrainsMonoNL-Regular.woff2` | JetBrains Mono NL, Latin subset | SIL OFL 1.1, `fonts/OFL-JetBrainsMono.txt` |
| `fonts/Phosphor-4.woff2` | Phosphor Regular from the `egui-phosphor` crate, only the icons the page uses | MIT, `fonts/LICENSE-Phosphor-MIT.txt` |

A new icon on the page needs the Phosphor subset made again with `pyftsubset` (fontTools) from the
crate's `res/Phosphor.ttf`, listing every code point that `index.html` and `app.js` use (today
E06C, E182, E18C, E1CA, E1E0, E20C, E27A, E27C, E280, E282, E2D8, E30C, E330, E39E, E3D0,
E40C, E434, E472, E516, E560, E5B8, E692 and EB02), with `--flavor=woff2`. Give the new subset a
new file name (`Phosphor-5.woff2`, and so on) and update `styles.css`: `fonts/` is served with a
one-year `immutable` cache, so a file changed in place stays stale for returning visitors.
