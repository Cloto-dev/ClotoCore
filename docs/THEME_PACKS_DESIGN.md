# Theme Packs — Design

**Status:** Approved 2026-09-18; implemented in the same change.
**Author:** dashboard team · 2026-09-17
**Related:** `DESIGN_PHILOSOPHY.md` §4 (the tokens a theme is allowed to
change), `dashboard/src/index.css` `@layer base` (where they live today),
`dashboard/src/i18n.ts` + `scripts/check-language-packs.py` (the precedent:
a data file someone outside the project can write, loaded at runtime and
gated in CI)

A theme today is a CSS class compiled into the bundle, and its name is
written in four places. Nobody outside the project can add one without
building the app. This document makes a theme a **data file** — a table of
token values — that can be written without touching a component, dropped into
a directory or imported from the settings page, and rejected with a reason
when it is incomplete or unreadable.

---

## 1. Problem & current state

Measured on the branch that carries the redesign (2026-09-17).

### A theme is code

`index.css` defines the tokens under `:root` (light), `.dark`,
`.theme-legacy` and `.theme-legacy.dark`. `tailwind.config.js` reads them as
`hsl(var(--token) / <alpha-value>)`. Adding a theme means adding a CSS block
and rebuilding.

### The list of themes is written four times

| Place | What it holds |
| --- | --- |
| `hooks/useTheme.ts` | `ThemePreference = 'light' \| 'dark' \| 'system' \| 'legacy'` |
| `components/settings/GeneralSection.tsx` | the picker's options |
| `components/SetupWizard.tsx` | the first-run picker's options |
| `index.html` | the pre-boot script that sets `dark` / `theme-legacy` before React |

### One setting holds two questions

`light / dark / system / legacy` mixes *which palette* with *which face of
it*. Legacy had to be special-cased as "a palette that also follows the OS"
(`followsSystem`). A fifth entry would need the same special case again.

### The fixed accent is forced through `!important`

`AgentProvider` writes `--agent` inline on `<html>`. Legacy wants one blue for
every agent, so its stylesheet value carries `!important`, and
`agentColor()` branches on the `theme-legacy` class because an inline style
cannot read a class. That is the one place a component knows a theme's name.

### The accent's readability is computed against a copied surface

`lib/agentIdentity.tsx` raises an agent colour's lightness until it holds
4.5:1 on the raised surface — but the raised surface it measures against is
three constants (`SCALE_HUE = 190`, `RAISED_SATURATION = 0.07`,
`RAISED_LIGHTNESS = 0.16`) copied from the dark block of `index.css`. Under
any other theme, or under light, the correction is computed against a
surface that is not on screen.

### Nothing checks a palette

No test says which tokens a theme must define, and nothing measures
contrast. Measured now, text on the three surface steps:

| Face | `text-secondary` | `text-tertiary` | `agent-ink` on `agent` |
| --- | --- | --- | --- |
| light | 5.73 – 6.90 | 4.72 – 5.68 | 8.17 |
| dark | 6.61 – 8.40 | 5.51 – 7.00 | 8.17 |
| Legacy light | **4.34** – 4.76 | **2.34 – 2.56** | 6.32 |
| Legacy dark | 9.85 – 12.02 | 5.71 – 6.96 | **3.70** |

`text-primary` is at least 12:1 everywhere. The current palette passes
4.5:1 on every pair; Legacy does not, on five (light `text-tertiary` on all
three surfaces, light `text-secondary` on the receding surface, and the dark
accent's ink). That matters for decision (f):
a gate that simply blocks under 4.5:1 would reject a theme the product
already ships on purpose.

### The precedent has a hole this design should not copy

Language packs are scanned from `Documents/ClotoCore/languages/` through a
Tauri command. In a browser session that command returns nothing, which is
why the bundled languages had to be moved into the bundle. A theme loader
built on the same command alone would offer external themes on the desktop
and none over HTTP.

---

## 2. Decisions

### (a) A theme pack is a JSON table of values

```json
{
  "schema": 1,
  "id": "solarized",
  "label": "Solarized",
  "author": "…", "version": "1.0.0", "license": "MIT",
  "accent": "agent",
  "dark":  { "surface-base": "192 100% 11%", "…": "…" },
  "light": { "surface-base": "44 87% 94%",  "…": "…" }
}
```

- **Required per face (10):** `surface-base`, `surface-secondary`,
  `surface-primary`, `border-default`, `border-subtle`, `text-primary`,
  `text-secondary`, `text-tertiary`, `text-muted`, `surface-overlay`.
- **Optional per face (4):** the role names `surface-panel`, `surface-field`,
  `surface-control`, `surface-card`. Omitted, they resolve to the steps they
  resolve to today. They exist for palettes like Legacy that give a role a
  colour of its own.
- **At least one face.** A pack with both follows the mode setting (b). A
  pack with one face is drawn in that face whatever the mode says, and the
  picker says so.
- Colour values are HSL component triplets (`"H S% L%"`), the form the
  stylesheet already uses, because Tailwind adds the alpha. `surface-overlay`
  is a triplet plus an alpha.
- **Colour only.** Type, radius, spacing and the purple that marks an
  MGP-negotiated server are the design's, not the theme's
  (`DESIGN_PHILOSOPHY.md` §4.3–4.4). A pack that names them is rejected as
  carrying an unknown key, not silently trimmed.

JSON rather than a CSS file: a CSS file has to be injected to take effect,
and injected CSS can do anything CSS can do (e). A table of values can be
validated completely.

### (b) Theme and mode are two settings

`theme` = a pack id (`default`, `legacy`, or an external one). `mode` =
`light | dark | system`. The stored `cloto-theme` value migrates once:
`legacy` → theme `legacy` + mode `system`; anything else → theme `default` +
that mode. `followsSystem`'s special case disappears.

The two pickers (settings, first run) render the list the loader returns.
Neither names a theme.

### (c) Dark, light and Legacy become bundled packs

`dashboard/src/themes/packs/default.json` and `legacy.json`, imported into the
bundle (`import.meta.glob`), so they exist in a browser session too. The
colour declarations leave `index.css`; what stays is `:root` with the
non-colour tokens and the default pack's values as the fallback for a page
whose script has not run.

Applying a theme writes one generated `<style id="cloto-theme">` holding
`:root { … }` for the light face and `.dark { … }` for the dark one, built
from the validated numbers. The `dark` class keeps its meaning, so nothing
under `dark:` in Tailwind changes.

**Before React:** the pre-boot script in `index.html` sets the `dark` class
from `mode` as it does now, and re-inserts the generated stylesheet text
cached in `localStorage` by the last successful apply. It carries no theme
name and no token list. The cache is only ever written from validated values;
on boot the app validates the pack again and overwrites it, so a stale or
hand-edited cache lasts one paint.

### (d) The pack says whether the accent is the agent's

`"accent": "agent"` (default) — `--agent` is the present agent's colour, as
now. `"accent": { "light": { "agent": "…", "agent-ink": "…" }, "dark": { … } }`
— one fixed accent, with an entry for every face the pack has; `applyPresentAgent` does not write the inline value, and
`agentColor()` returns `hsl(var(--agent))`. Both read one flag the theme
applier sets (`<html data-accent="fixed">`). `!important` and the
`theme-legacy` class check are deleted; no component knows a theme's name.

The readability correction in `agentIdentity.tsx` measures against the active
face's `surface-primary` instead of the three copied constants. On a light
surface that means moving the accent *down* until it reads, and an accent that
dark no longer carries the stylesheet's dark ink: the ink becomes white when
white reads better on it. The accent is written again whenever a theme is
applied. Colours computed inline for other agents' faces are corrected at their
next render, not at the moment of the switch.

### (e) Values only — nothing a pack says is injected as written

Every value is parsed into numbers (hue 0–360, percentages 0–100, alpha 0–1)
and the stylesheet is written from the numbers. Unknown top-level keys,
unknown tokens, strings that do not parse, an `id` outside `[a-z0-9-]{1,40}`
and files over 16 KB are rejected. `label` and `author` are rendered as text
by React and never reach the stylesheet. There is no `css`, `url`, `font` or
`import` field, now or as an extension point.

### (f) One validator; two severities

One TypeScript validator is used by the loader, by import, and by the CI
test over the bundled packs — not a second implementation in a script that
would drift from the first.

- **Rejected (the pack is not listed; the settings page shows why):**
  malformed file, missing required token, unknown key, and
  **`text-primary` under 4.5:1 on any of the three surface steps**. That last
  one is the lock-out guard: a theme in which the settings page cannot be read
  cannot be switched away from.
- **Warned (listed, marked "low contrast" in the picker):** `text-secondary`
  or `text-tertiary` under 4.5:1 on a surface step; a fixed accent's ink
  under 4.5:1 on the accent. `text-muted` is exempt — it is defined as not for
  reading.

Why warn rather than reject: Legacy fails five of these pairs (§1) and is
shipped deliberately as "the colours it had". For the **bundled** packs the CI
test pins the exact set of warnings by name, so Legacy's five are written
down and a sixth fails the build.

### (g) Where external packs come from

1. **Bundled** — always present, desktop and browser.
2. **Directory (desktop):** `Documents/ClotoCore/themes/*.json`, scanned at
   startup through the same Tauri commands the language packs use,
   generalised to take the directory's name. Dropping a file there and
   restarting is the whole install.
3. **Import (desktop and browser):** the settings page takes a `.json`,
   validates it, and keeps it — on the desktop by saving it into the
   directory, in a browser in `localStorage`, since a pack is a few hundred
   bytes. This is what keeps external themes from being desktop-only.

An external pack whose `id` collides with a bundled one is rejected.
Removing the active theme falls back to `default`.

The settings page also exports the default pack as a template, as it does for
languages.

---

## 3. What changes

| Area | Change |
| --- | --- |
| `dashboard/src/themes/` | new: `packs/*.json`, `validate.ts`, `apply.ts`, `load.ts` |
| `index.css` | colour blocks for `.dark` / `.theme-legacy*` removed; `:root` keeps non-colour tokens + fallback colours |
| `hooks/useTheme.ts` | `theme` + `mode`, one-time migration of the stored value |
| `GeneralSection.tsx`, `SetupWizard.tsx` | list from the loader; import / remove / export template |
| `index.html` | pre-boot script: mode class + cached stylesheet, no names |
| `lib/agentIdentity.tsx` | fixed-accent flag instead of the class check; contrast against the active surface |
| `src-tauri/src/lib.rs` | the language-pack commands (scan / save / remove) take a directory kind (`languages` \| `themes`); same filename sanitiser |
| `CLAUDE.md` "Dashboard UI Rules" | "Themes change tokens" points at the pack format |
| docs | a short "Writing a theme" page for the published site |

No kernel change. No migration.

## 4. Verification

Each row needs a test that goes red under the mutation named.

| Behaviour | Mutation that must be caught |
| --- | --- |
| A pack missing a required token is rejected | drop one name from the required list |
| An unknown key / token is rejected | accept unknown keys |
| A value that is not a triplet never reaches the stylesheet | write the raw string instead of the parsed numbers |
| `text-primary` under 4.5:1 is rejected | lower the floor; compare against one surface instead of three |
| Bundled packs' warnings are exactly the pinned set | add a failing pair to Legacy; remove one from the pin |
| A pack placed in the directory appears in the picker and applies | skip the directory scan |
| A pack imported in a browser session survives a reload | skip the `localStorage` write |
| Fixed accent: selecting an agent does not change `--agent` | ignore the flag in `applyPresentAgent` |
| Agent accent is corrected against the active face's raised surface | restore the constants |
| No theme id appears in a component, a hook or `index.html` | a structural test that greps for the bundled ids outside `themes/packs/` (it must not read its own file) |
| The stored `legacy` preference migrates to theme + mode | map it to `default` |
| Rendered colours are unchanged for default dark, default light and both Legacy faces | computed-style comparison in a real browser of the 15 colour tokens (the 13 of a face plus the accent and its ink) against the values resolved from the stylesheet before the move — this is the proof the port is behaviour-preserving |

Visual check on the real app: the three existing looks are unchanged, and one
external pack applied from the directory.

## 5. Out of scope

- Themes that change type, radius, density or layout.
- Syntax-highlighting colours in code blocks (`highlight.js/styles/github-dark.css`
  is imported once and is dark under every theme today; a known limit, not
  made worse).
- Distributing themes through the marketplace, and serving a shared set of
  packs from the kernel to every browser of a headless deployment. Import
  covers one browser at a time; if a deployment-wide set is wanted later, the
  pack format does not change — only a fourth source is added to (g).
- Per-agent themes.

## 6. Alternatives that were considered and not taken

1. **Rejecting every low-contrast pack (f).** Simpler for users, but Legacy
   would need an exemption by name — a theme name back in code.
2. **Requiring both faces (a).** Halves the states the picker has to
   describe, and makes a dark-only theme impossible to publish.
3. **No cached stylesheet before React (c).** `index.html` would need nothing
   new, and every external theme would flash the default palette on load.
