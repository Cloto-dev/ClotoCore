# Writing a theme

A ClotoCore theme is one JSON file of colours. You do not need to build the
app or touch a component: write the file, import it from **Settings → General →
Theme packs**, and it is applied. On the desktop you can also drop the file into
`Documents/ClotoCore/themes/` and restart.

The quickest start is **Export** in that settings group, which hands you the
default theme as a template.

## The file

```json
{
  "schema": 1,
  "id": "my-theme",
  "label": "My theme",
  "author": "You",
  "version": "1.0.0",
  "license": "MIT",
  "accent": "agent",
  "dark": {
    "surface-base": "260 20% 6%",
    "surface-secondary": "260 20% 10%",
    "surface-primary": "260 20% 14%",
    "border-default": "260 20% 22%",
    "border-subtle": "260 20% 14%",
    "surface-overlay": "0 0% 0% / 0.6",
    "text-primary": "260 10% 94%",
    "text-secondary": "260 8% 72%",
    "text-tertiary": "260 6% 64%",
    "text-muted": "260 6% 40%"
  }
}
```

| Field | Meaning |
| --- | --- |
| `schema` | Always `1`. |
| `id` | 1–40 characters of `a–z`, `0–9` and `-`. It is the file's name on disk, and it cannot be the id of a built-in theme. |
| `label` | The name shown in the picker (up to 60 characters). |
| `author`, `version`, `license` | Optional, shown as text. |
| `accent` | `"agent"` (the default) or a fixed accent — see below. |
| `light`, `dark` | The two faces. Give both, or only one. |

Every colour is an HSL triplet written as `"H S% L%"` — hue 0–360, saturation
and lightness 0–100. `surface-overlay` (the scrim behind a modal) also takes an
alpha: `"H S% L% / A"`.

### The colours of a face

All ten are required:

| Token | Where it is drawn |
| --- | --- |
| `surface-base` | The work area. |
| `surface-secondary` | What recedes: the sidebar, panels. |
| `surface-primary` | What is raised: inputs, hover, cards. |
| `border-default`, `border-subtle` | Boundaries. |
| `surface-overlay` | The scrim behind a modal. |
| `text-primary` | Body text. |
| `text-secondary` | Supporting text. |
| `text-tertiary` | The smallest text that is still meant to be read. |
| `text-muted` | Not for reading: disabled marks, decoration. |

Four more are optional: `surface-panel`, `surface-field`, `surface-control`
and `surface-card`. Leave them out and they follow `surface-secondary` /
`surface-primary`; set them when panels, inputs, prominent controls or cards
should have a colour of their own.

### One face or two

With both faces, the theme follows the user's light / dark / system setting.
With one, it is always drawn in that face, and the settings page says so.

### The accent

By default the accent is the colour of the agent you are talking to, corrected
so it stays readable on your `surface-primary`. A theme can hold one accent of
its own instead — one entry for each face the theme has:

```json
"accent": {
  "light": { "agent": "229 78% 54%", "agent-ink": "0 0% 100%" },
  "dark":  { "agent": "228 100% 68%", "agent-ink": "0 0% 100%" }
}
```

`agent-ink` is the colour of text set on the accent.

## What is checked

A theme is refused, with the reason shown, when:

- the file is not JSON, is larger than 16 KB, or `schema` is not `1`;
- a required colour is missing, or a value is not a colour in the form above;
- it contains a key or a token that is not listed here — a theme changes
  colours only, not type, radius or layout;
- **`text-primary` has less than 4.5:1 contrast on any of the three surfaces.**
  Under such a theme the settings page that switches away from it could not be
  read.

A theme is accepted but marked **low contrast** in the picker when
`text-secondary` or `text-tertiary` falls under 4.5:1 on a surface, or a fixed
accent's ink falls under 4.5:1 on the accent.

Nothing in the file is inserted into the page as written: each colour is parsed
into numbers, and the stylesheet is generated from the numbers.

## Where a theme is kept

| You are using | Import keeps it in | Also read from |
| --- | --- | --- |
| The desktop app | `Documents/ClotoCore/themes/<id>.json` | every `.json` in that directory, at startup |
| A browser | that browser's local storage | — |

**Remove** in the settings group forgets an imported theme. Removing the theme
on screen goes back to the default.
