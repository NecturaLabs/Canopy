# Canopy website

The landing page for Canopy: one static page in the app's own look (Midnight and Daylight, Inter
and JetBrains Mono, the indigo primary), led by the keyboard. Its first screen is the History view
with a working command palette beside it, and each section names the keys that do the job. There
is no build step and no framework: `index.html`, `styles.css`, `app.js` and `theme.js` are the
source and the output.

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
`netlify-cli deploy --dir=. --prod --no-build` from this folder. It needs the organisation secret
`NETLIFY_AUTH_TOKEN` and the repository variable `NETLIFY_SITE_ID`; nothing is set by hand in
Netlify. To redeploy without a change, choose Run workflow on the Site workflow.

The whole folder is published as it is, this README and `netlify.toml` included. `netlify.toml`
sets the headers: a content security policy that allows only the site's own files plus
`raw.githubusercontent.com` for the build list, no referrer, no framing, and long caching for the
fonts. The page has no inline scripts, styles or event handlers, so the policy needs no
exceptions. It also holds the two installer addresses: `/install` and `/install.ps1` are 302
redirects to `canopy/install.sh` and `canopy/install.ps1` in NecturaLabs/app-updates, so the
script that runs is always the published one and this site never serves a copy. The canonical
address, `og:url` and `og:image` in `index.html` use `https://canopy.necturalabs.com`.

## How the page behaves

- **Theme.** `theme.js` applies a saved choice before the first paint; otherwise the system's
  light or dark setting decides. The switch in the header saves Midnight or Daylight in
  `localStorage` under `canopy-theme`, and only when the visitor picks one. Every screenshot has a
  Midnight (`m-`) and a Daylight (`d-`) render, and the page shows the one that matches.
- **Palette.** With a mouse or trackpad and a window wider than 720 px, the palette is a
  combobox: type to filter, arrows to move, Enter to run, Esc to clear. `Ctrl+K` focuses it,
  `Ctrl+Enter` goes to the downloads, and `J` and `K` move between sections when no text field
  has focus. On touch screens, on phones and tablets (whatever their pointer reports) and in
  narrow windows it stays the plain list of section links written into the page, placed under the
  screenshot: one column on phones, two on tablets, every row at least 48 px tall.
- **Install command.** The main download is one command, chosen from the visitor's system:
  `curl -fsSL https://canopy.necturalabs.com/install | sh` on Linux and
  `irm https://canopy.necturalabs.com/install.ps1 | iex` on Windows. A switch shows the other
  line, the copy button says Copied and a polite status message announces it. Android, iPhone and
  iPad visitors get no command up front: the page says Canopy is a desktop app for Linux (x86_64
  and arm64) and Windows (x86_64) with no mobile app yet, offers to copy the page's link to open
  on a computer, and keeps the commands behind Show the install commands. The header's Download
  button is hidden for them. Unrecognised systems see that Canopy runs on Linux and Windows, and
  macOS visitors that there is no macOS build yet (`install.sh` supports macOS, but no macOS
  archive is published); both lines stay available to them. The archives remain listed below the
  command, with the visitor's own build marked.
- **Phones.** Below 720 px each full-window screenshot is replaced by a crop of the same render
  (`*-crop.webp`, `*-changes-hunk.webp`). Below 600 px the crops sit in a taller frame
  (`object-fit: cover`, anchored top left) so they show at about three quarters of their real size
  and their text stays readable; the hero shot is drawn at 150 % width for the same reason, and
  the shortcuts sheet is left out, since the rules beside it say the same. Phones and tablets
  (detected, or any coarse pointer) get controls at least 44 px tall, and the header, page edges
  and footer keep clear of notches through `viewport-fit=cover` and `env(safe-area-inset-*)`.
  Tablets from 721 to 960 px keep the five steps to a commit on one row.
- **Words and pictures.** Copy names the real thing: the key, the feature, the exact behaviour.
  No superlatives, taglines or claims the sources under Keeping it true do not make. Every picture
  is a render of the app or one of its own controls, with no decorative gradients, glows or icon
  grids.

## What it loads

Everything is served from this folder: no third-party fonts, scripts, analytics or cookies. The one
request elsewhere is the public update manifest,
`https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/latest.json`, which the page
reads to show the newest build number, date and download links. The links written into the page
(build 690) stay in place if that request fails, and every link the manifest returns must match
the release-asset pattern in `app.js` before the page uses it.

## Keeping it true

- Every claim comes from the Done and Unreleased rows of `docs/features.md`, the product rules in
  `AGENTS.md`, `docs/keybindings.md` for the keys, and `docs/legal/privacy-policy.md` for the
  privacy section. When one of them changes, check the matching section here.
- The installer notes on the page come from the published `install.sh` and `install.ps1`
  themselves: per-user folders, no `sudo` or administrator rights, the SHA-256
  check against the manifest, upgrading by running it again, the uninstaller it leaves behind and
  the git 2.30 requirement. Read them again before changing those lines.
- The themes section names Midnight, Daylight, Observatory, the Custom theme and the High
  contrast switch, from the Unreleased rows in `docs/features.md`. Midnight, Daylight and Custom
  (`m-custom`, `d-custom`: accent hue 150, tint 40 at hue 150, built from Midnight and from
  Daylight) have screenshots; do not show Observatory or High contrast until there are real
  renders.
- The screenshots in `./assets/shots/` are real renders from `tools/screenshot.sh` on demo
  repositories made by `tools/mkfixture.sh`, at 1440 by 900, converted to WebP at quality 80
  (`magick in.png -strip -quality 80 -define webp:method=6 out.webp`); the crops are cut
  from those renders. The pull requests picture uses the app's invented demo data and says so on
  the page. Each picture records its origin in a `.json` file beside it. To refresh one, render it
  again and convert it to WebP, then cut the crop again at the offsets its `.json` file gives.
- The icons in `./assets/` are copies of the app icon in `../assets/`.

## Fonts and licences

| File | Font | Licence |
|---|---|---|
| `fonts/Inter-*.woff2` | Inter (Regular, Medium, SemiBold), Latin subset | SIL OFL 1.1, `fonts/OFL-Inter.txt` |
| `fonts/JetBrainsMonoNL-Regular.woff2` | JetBrains Mono NL, Latin subset | SIL OFL 1.1, `fonts/OFL-JetBrainsMono.txt` |
| `fonts/Phosphor.woff2` | Phosphor Regular from the `egui-phosphor` crate, only the icons the page uses | MIT, `fonts/LICENSE-Phosphor-MIT.txt` |

A new icon on the page needs the Phosphor subset made again with `pyftsubset` (fontTools) from the
crate's `res/Phosphor.ttf`, listing every code point that `index.html` and `app.js` use (today
E182, E18C, E1CA, E1E0, E20C, E27A, E27C, E280, E282, E2D8, E30C, E330, E40C, E472, E516,
E560, E5B8, E692 and EB02), with `--flavor=woff2`.
