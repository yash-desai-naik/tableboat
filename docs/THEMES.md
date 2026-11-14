# Tableboat – themes plan

Status: **plan, not built yet.** This file is what to build, in what order, and the rules every theme must
pass, so the whole UI stays consistent and readable whichever theme is picked.

---

## 1. Goals

1. **Popular themes, not just light and dark.** Ship the colour schemes developers already use in 2026, in
   light and dark versions where they exist.
2. **Every pixel follows the theme.** Sidebar, tabs, SQL editor, grid, document view, dialogs, menus,
   Claude pane, update notice, scrollbars. No colour may be written directly in a component.
3. **Readable by rule, not by eye.** Every theme passes the contrast rules in section 6, checked by a test
   in CI. When an original palette is too faint, the colour is nudged lighter or darker just enough; its
   hue and saturation stay the same.
4. **Meaning never changes with the theme.** Red is always "danger/deleted", amber "changed/warning",
   green "added/success". Connection colours (red PROD) stay fixed in every theme.
5. **Cheap.** Switching themes changes CSS variables only: no reload, no extra memory, no new libraries.

## 2. What is trending (research, Nov 2026)

| Signal | Finding |
|---|---|
| VS Code installs | GitHub Theme (most installed), One Dark Pro, Dracula; Tokyo Night has the best rating of the popular ones ([devresourc.es](https://devresourc.es/blog/best-vscode-themes), [DevDreaming](https://devdreaming.com/blogs/top-10-popular-vs-code-themes)) |
| "Trending" lists | Catppuccin is named the top trending theme of 2026 in most roundups ([Windows Mode](https://www.windowsmode.com/best-vs-code-themes/), [jit.io](https://www.jit.io/blog/best-vs-code-themes-2023)) |
| Terminals (Ghostty, iTerm2) | Catppuccin and Tokyo Night lead; Gruvbox is "the safe all-day choice" ([Ghostty ranking](https://devresourc.es/blog/best-ghostty-themes), [MOLTamp](https://moltamp.com/blog/best-terminal-color-schemes-2026/)) |
| Neovim (Dotfyle trending) | onedark, catppuccin, kanagawa, rose-pine, gruber-darker, cyberdream ([Dotfyle](https://dotfyle.com/neovim/colorscheme/trending)) |
| UI design trends | Dark mode as the primary surface; soft blacks instead of `#000`; layered surfaces (background → raised → overlay) rather than inverted colours; semantic tokens so one system serves light and dark; WCAG 2.2 contrast now enforced in the EU ([updivision](https://updivision.com/blog/post/ui-color-trends-to-watch-in-2026), [muz.li](https://muz.li/blog/whats-changing-in-mobile-app-design-ui-patterns-that-matter-in-2026/)) |
| Colour of the year | Pantone 2026 is **Cloud Dancer**, a soft warm white ([Galerie](https://galeriemagazine.com/pantone-announces-cloud-dancer-color-of-the-year-2026/)). Tableboat Light's cream (`#faf9f5`) is already in that family |

Treat the rankings loosely (most sources are blogs); the same names recur everywhere, which is what matters.

## 3. Where the UI stands today (audit)

- ~40 tokens in `src/styles.css` (`--bg`, `--text-2`, `--accent`, …) for one light and one dark theme.
- **~45 colours are written directly in code** and would not follow a theme:

| Where | What | Fix |
|---|---|---|
| `src/editor/theme.ts` | VS Code Dark+/Light+ syntax colours, gutter, bracket, search, current-statement colours | `--syn-*`, `--editor-*`, `--search-*` tokens |
| `src/results/ResultGrid.tsx` | white header text on selection, search match, cell borders, changed/added/deleted row tints | `--on-accent`, `--search-*`, `--grid-line`, `--edit-*` |
| `src/results.css` | Mongo document colours (`--doc-*`), amber `#d29922`, red `#e5534b`, white badge text | `--syn-*`, `--warning`, `--danger`, `--on-danger` |
| `src/workspace.css` | object-kind colours (view/function/procedure/trigger), error red | `--kind-*`, `--danger` |
| `src/ai.css` | green "connected" dots `#3fa36b`, send-button hover mixes with black | `--success`, `--accent-hover` |
| `src/controls.css` | success notice green `#2e7d32`, danger menu item red, white avatar text, primary hover mixes with white | `--success*`, `--danger`, `--on-accent`, `--accent-hover` |
| `src/styles.css` | white text on the red close button | `--on-danger` |
| `--zebra` | defined but not used in CSS (the grid reads it from JS) | keep, document it |

- **Today's two themes already fail the rules** in places (section 7): hint text (`--text-3`) is 3.0–4.0:1,
  and white text on the clay accent button is 3.1:1 (dark) and 3.9:1 (light), below the 4.5:1 minimum.
  The plan fixes these for the Tableboat themes too.

Stays fixed in every theme: the app icon (`Icons.tsx` gradients), connection/tag colours
(`COLOR_PRESETS`, `TAGS` in `src/lib/connections.ts`, already given black or white text by contrast in
`TitleBar.tsx`), and the font (JetBrains Mono). A theme changes colours only.

## 4. How it works

### 4.1 Settings

| Key | Values | Meaning |
|---|---|---|
| `theme` (exists) | `system` · `light` · `dark` | Which **mode**. `system` follows Windows |
| `lightTheme` (new) | a light theme id, default `tableboat-light` | Used when the mode is light |
| `darkTheme` (new) | a dark theme id, default `tableboat-dark` | Used when the mode is dark |

So "Match Windows" switches between *your* light theme and *your* dark theme at sunset, like VS Code's
preferred light/dark themes. Existing settings files keep working: the new keys default to today's look.
`prefs.rs` validates ids (unknown id → default); `prefs::HELP`, `tb settings` and Claude's
`update_settings` learn the new keys.

### 4.2 One source of truth

```
src/themes/themes.json      every theme's seed colours (section 5.1), ~25 values each
src/themes/index.ts         loads the JSON, derives the rest (section 5.2), writes CSS variables on <html>
src/styles.css              only layout tokens (sizes, radii, fonts) and the derivation formulas
src-tauri/src/themes.rs     test only: reads the same JSON and enforces section 6 in `cargo test` (CI)
```

- `<html data-theme="dark" data-color-theme="catppuccin-mocha">`; `index.ts` sets the variables in one
  `<style>` element, so a switch is one style recalculation.
- **Editor:** the CodeMirror highlight style and chrome use `var(--syn-keyword)` etc., so one editor theme
  serves every colour theme (it only rebuilds for light/dark, as now).
- **Grid:** `readTheme()` already re-reads CSS variables when `data-theme` changes; it will also watch
  `data-color-theme`. Hard-coded colours there become tokens.
- **First paint:** `main.tsx` applies the theme before the first render, as now, so there is no flash.
  The window background colour is set from the theme too.
- **JSON shared with Rust** so CI can fail a pull request that adds an unreadable theme.

### 4.3 Where you choose

- **Settings → Appearance:** Mode (Match Windows / Light / Dark), then a **Light theme** and a **Dark theme**
  picker. Each choice is a small card that draws a miniature Tableboat (sidebar, tab, three syntax colours,
  a selected row) in that theme, so you see it before choosing. Hovering previews it live on the window;
  leaving restores the current one.
- **Command palette:** "Theme: Catppuccin Mocha", …, one command per theme (Claude sees these through
  `list_app_commands`), plus "Theme: Next Light/Dark Theme".
- **CLI / Claude:** `tb settings set darkTheme tokyo-night`; Claude's `update_settings` (asks first, as now).

## 5. Tokens

### 5.1 Seeds — what a theme defines (~25 values)

| Seed | Used for |
|---|---|
| `bg` | editor, grid cells, document view: where data is read |
| `chrome` | title bar, tab strip, toolbars, results bar, status areas |
| `sidebar` | the schema tree |
| `elevated` | dialogs, menus, palette, tooltips, autocomplete, popovers |
| `text`, `text2`, `text3` | primary text; secondary (labels, metadata); tertiary (hints, placeholders, counts) |
| `accent` | links, focus, selected header, caret, matched text, table icons: *as text/lines* |
| `accentFill` (optional) | primary button and badge background, when `accent` is too light for text on it |
| `onAccent` | text on `accentFill` |
| `selection` (optional) | text selection and selected rows; derived when absent |
| `red orange yellow green cyan blue purple pink` | the theme's 8 hues |
| `status.danger/warning/success/info` (optional) | default to red, yellow, green, blue |
| `syntax.*` (9) | keyword, type, function, string, number, literal (true/false/null), property (also JSON/Mongo keys), operator, comment; a hue name or a colour |
| `kind.*` (5) | table, view, function, procedure, trigger icons in the sidebar, palette and Claude mentions |

### 5.2 Derived — computed the same way for every theme (theme may override)

Mixes are `color-mix(in oklab, …)` so they look even across hues.

| Token | Formula | Used by |
|---|---|---|
| `--bg-hover` | text 6% over chrome | tree rows, icon buttons, caption buttons, db switch |
| `--bg-active` | text 11% over chrome | selected tree row, menu/palette selection |
| `--control-fill` / `-hover` | text 7% / 11% over chrome | buttons, segmented controls, result tabs |
| `--surface-raised` | dark: text 16% over chrome · light: elevated | active tab, active segment, active result tab |
| `--border` / `--border-strong` | text 10% / 20% over bg | dividers / input and dialog borders |
| `--focus-ring` | accent | keyboard focus outline (must reach 3:1, section 6) |
| `--zebra` | text 3% over bg | alternate grid rows |
| `--grid-line` | text 7% over bg | grid cell borders (today hard-coded) |
| `--selection` | accent 30% over bg | selected text, selected grid cells |
| `--accent-hover` | fill 8% toward text | primary button hover (replaces the white/black mixes) |
| `--accent-tint` / `--accent-soft` | accent 20% over bg / accent at 22% (dark) 14% (light) alpha | autocomplete selection, soft highlights |
| `--danger --warning --success --info` | status seeds | error text, "truncated", "cancelled", connected dots |
| `--danger-bg`, `--success-bg`, … | status 14% over bg | notices (error, saved) |
| `--on-danger` | white or black, whichever contrasts more | red close button, error badge |
| `--edit-changed / -added / -deleted` | warning / success / danger 18% over bg | pending-edit cell and row tints |
| `--search-match` / `--search-current` | warning 30% / 55% over bg | editor and grid search |
| `--editor-gutter` / `-active` | text3 / text2 | line numbers |
| `--editor-current-stmt` | text 4% over bg | the statement Ctrl+Enter runs |
| `--editor-bracket` | text 12% over bg, outline text3 | matching bracket |
| `--scrollbar` / `-hover` | text 22% / 32% over bg | scrollbars |
| `--scrim` | black 45% (dark) / 22% (light) | behind dialogs |
| `--shadow-raised` / `--shadow-popup` | black at 35%/50% (dark), warm/cool 14%/16% (light) | raised controls / popups |
| `--syn-*` | syntax seeds | SQL editor, Mongo document view, JSON view, SQL previews |
| `--kind-*` | kind seeds | object icons |

Layout tokens (`--fs-*`, `--row-h`, radii, heights) are not part of a theme and don't change.

## 6. Contrast rules (WCAG 2.2)

Checked in CI on every theme, against every surface the colour appears on:

| Pair | Minimum | On |
|---|---|---|
| `text` | **7:1** (AAA) | bg, chrome, sidebar, elevated |
| `text2`, `text3` | **4.5:1** (AA) | bg, chrome, sidebar, elevated |
| `accent` used as text | 4.5:1 | bg, elevated |
| `onAccent` on `accentFill`, `on-danger` on danger | 4.5:1 | |
| `text` on `selection` | 4.5:1 | |
| status colours as text | 4.5:1 | bg, elevated |
| syntax colours | 4.5:1; comments **3.5:1** (deliberately quiet, still legible) | bg |
| object-kind icons, focus ring | 3:1 (non-text, WCAG 1.4.11) | sidebar, bg |
| High Contrast themes | text 15:1, everything else 7:1 | |

If a palette colour misses, the build-time tool moves its OKLCH lightness the smallest step that passes,
keeping hue and chroma. The adjusted value is written into `themes.json` with the original beside it, so
the change is visible and reviewable.

## 7. The themes

19 variants, all from MIT-licensed palettes (credited in the README and About box). Contrast results
come from the checker run on the official colours; "adjusted" lists what section 6 forces.

| Theme | Mode | Pairs with | Adjusted to pass |
|---|---|---|---|
| **Tableboat Dark** (default) | dark | Tableboat Light | hints `#8f8d85→#9a9890`; button fill `#d97757→#b95a3b`; danger, success a step lighter |
| **Tableboat Light** (default) | light | Tableboat Dark | hints `#8d8b83→#6d6b63`; button fill `#c96442→#bd5937`; accent text `→#b85533`; 2 syntax colours a hair darker |
| Catppuccin Mocha | dark | Latte | hints a hair lighter |
| Catppuccin Latte | light | Mocha | text, hints, and 8 syntax/status colours darker (Latte's yellow/green/peach are 2.3–3.0:1) |
| Tokyo Night | dark | GitHub Light | hints and comments lighter |
| GitHub Dark | dark | GitHub Light | none |
| GitHub Light | light | GitHub Dark | hints a hair darker |
| One Dark | dark | One Light | text `#abb2bf→#b8bfcc`, hints, red a step lighter |
| One Light | light | One Dark | hints, yellow, green, cyan, red, comments darker |
| Rosé Pine | dark | Rosé Pine Dawn | hints, pine keywords lighter |
| Rosé Pine Dawn | light | Rosé Pine | most colours darker (official Dawn is 2.0–4.0:1); button fill `→#a85855` |
| Gruvbox Dark | dark | Gruvbox Light | hints, red, blue a step lighter |
| Gruvbox Light | light | Gruvbox Dark | hints, yellow, green, comments darker |
| Kanagawa Wave | dark | Kanagawa Lotus | hints and comments lighter |
| Kanagawa Lotus | light | Kanagawa Wave | text, hints and 7 syntax colours darker |
| Nord | dark | GitHub Light | hints, red, comments lighter |
| Dracula | dark | GitHub Light | hints, comments, red lighter |
| High Contrast Dark | dark | High Contrast Light | none (built for it) |
| High Contrast Light | light | High Contrast Dark | none |

**Left out on purpose:**
- *Tokyo Night Day* – its signature blue body text is 4.0:1; fixing it to 7:1 turns it navy and it no longer
  looks like Tokyo Night. Tokyo Night pairs with GitHub Light instead.
- *Nord Light* – not an official Nord theme, and nearly every Nord colour needs darkening on white.
- *Solarized, Monokai, Ayu, Everforest, Cyberdream* – fine candidates for a second batch once the system
  exists; each is one entry in `themes.json`.

Syntax colours follow each theme's own guide: e.g. Catppuccin keyword mauve / function blue / string green;
GitHub keyword red / function purple / string light blue; Gruvbox keyword red / type yellow / string green.
The Tableboat themes keep VS Code Dark+/Light+ syntax, so the default looks the same as today apart from
the readability fixes.

## 8. Element inventory → tokens

Every element and the tokens it may use. Anything not listed uses `--text` on its parent surface.

| Area | Element | Tokens |
|---|---|---|
| **Title bar** | background · title · caption buttons · close hover · connection badge | chrome · text2 · bg-hover · danger/on-danger · connection colour (fixed) |
| **Sidebar** | background · database switch · tree row (hover/selected) · object icons · filter box · empty/error note | sidebar · bg-hover · bg-hover/bg-active · kind-* · elevated+border-strong · text3/danger |
| **Tabs** | strip · tab (hover/active) · dirty dot · close button · preview (italic) | chrome · control-fill/surface-raised · warning · control-fill-hover · text2 |
| **SQL editor** | background · text · gutter · active gutter line · caret · selection · current statement · brackets · selection matches · search matches · fold marker · lint underline · autocomplete · search panel | bg · text · editor-gutter · editor-gutter-active · accent · selection · editor-current-stmt · editor-bracket · accent-soft · search-match/-current · text3 · danger · elevated/accent-tint/accent · chrome+border |
| **Syntax** | keywords, types, functions, strings, numbers, true/false/null, keys/properties, operators, comments, invalid | syn-* · danger |
| **Grid** | cells · zebra · lines · header (hover/selected) · header type label · sort arrow · selection · search · NULL/muted · edited/new/deleted · drill-down bubble · links | bg · zebra · grid-line · chrome/bg-hover/accentFill+onAccent · text3 · accent · selection · search-* · text3 · edit-* · elevated · accent |
| **Results bar** | result tabs · error badge · summary · truncated note · pager · Documents/Table switch | control-fill/surface-raised · danger/on-danger · text3 · warning · control-fill · seg tokens |
| **Messages** | statement · rows/affected · error · cancelled | text · text2 · danger · warning |
| **Document view** | keys · strings · numbers · literals · types (`ObjectId(…)`) · card · actions | syn-property · syn-string · syn-number · syn-literal · syn-type · bg/border · btn tokens |
| **Dialogs** | scrim · panel · title · body · footer · inputs · focus | scrim · elevated+shadow-popup · text · text2 · border · bg+border-strong · focus-ring |
| **Buttons** | default · hover · primary · primary hover · danger · disabled | control-fill · control-fill-hover · accentFill/onAccent · accent-hover · danger/on-danger · text3 |
| **Inputs** | field · placeholder · focus · invalid | bg+border-strong · text3 · focus-ring · danger |
| **Segmented / toggles** | track · active segment | control-fill · surface-raised+shadow-raised |
| **Menus & palette** | panel · item · selected · shortcut caps · category label · danger item | elevated · text · bg-active · control-fill+text2 · text3 · danger |
| **Notices** | info · success · error · update notice | elevated+border-strong · success-bg+success · danger-bg+danger · elevated+accent |
| **Claude pane** | messages · your message · tool cards · approval card · connected dots · mention chips · send button | bg · control-fill · elevated+border · warning-bg/accent · success · kind-*/success · accentFill/accent-hover |
| **Misc** | scrollbars · tooltips · avatars · keyboard focus everywhere | scrollbar · elevated · accentFill+onAccent · focus-ring |

## 9. Build order

1. **Tokens first, no new themes.** Add the missing tokens, move every hard-coded colour in section 3 onto
   them, with values that reproduce today's two themes (plus the readability fixes). Nothing else
   changes visibly. This is the step that makes consistency possible.
2. **Theme data and loader.** `themes.json`, `index.ts`, derivations, `data-color-theme`, first-paint path,
   grid/editor reading tokens.
3. **Contrast test in CI** (`cargo test`), and the adjusted values for each theme.
4. **Settings and commands.** `lightTheme`/`darkTheme` in `prefs.rs` (+ HELP, `tb`, Claude), Appearance
   cards with live preview, palette commands.
5. **Add the 17 other themes**, then check each by eye on a real connection: grid with edits, editor with
   a search, Mongo documents, a dialog, the Claude pane.
6. Docs (README screenshots, CHANGELOG), release as **0.3.0** (new feature → minor).

Later, if wanted: your own themes as JSON files in `%APPDATA%\Tableboat\themes` (same seeds, same test
applied when loading), a custom accent colour, and a per-connection theme tint.

## 10. Credits

Palettes: Catppuccin, Tokyo Night, GitHub Primer, One Dark/One Light (Atom), Rosé Pine, Gruvbox, Kanagawa,
Nord, Dracula, all under the MIT licence. Only colour values are used (no code), adjusted for contrast
as listed above, and each is credited by name in the README and About box.
