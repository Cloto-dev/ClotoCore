# Design Philosophy — Living Room and Workshop

**Status**: Draft (direction approved 2026-09-16; not yet applied to `dashboard/`)
**Related**: `PROJECT_VISION.md` (what the product is), `DEVELOPMENT.md` §1.5
(agents and tools must not be confused), `gui/samples/` (one static mockup per
screen, built to this document)

---

## 1. Why this document exists

The dashboard was measured on 2026-09-16 (83 components, 15,860 lines of
TSX). What it showed is a console, not a companion:

| Measure | Value |
|---|---|
| Type declarations at 12 px or smaller / at 14 px or larger | 580 / 35 |
| `font-mono` uses | 305 (more than the number of components) |
| `uppercase` uses | 163 |
| `focus:outline-none` / any `focus-visible` replacement | 30 / 0 |
| Responsive prefixes (`sm:` `md:` `lg:`) across the app | 13 |
| Message reading column | none — `max-w-[80%]` of the window |
| Composer | a single-line `<input type="text">`: no newline, no stop button |
| Conversations | none — one flat message log per agent; "Reset" deletes it |

`PROJECT_VISION.md` says the product "is not a chatbot. It is not an AI
assistant." — it is a partner you build and live with. The measured interface
says the opposite: monospace, capitals, grid lines, a command prompt
("ENTER COMMAND..."). A first redesign that copied a popular chat product fixed
the console but produced an assistant. Neither expresses the product.

The other finding is about *how* interfaces come to look this way. Generated
UI clusters around defaults — a framework's colour scale, a component kit's
radius, a font nobody chose — and reads as generated precisely because no
decision is visible. The fix is not "be more original"; it is to **decide
first, and write the decisions down** so every later change inherits them.
This document is those decisions.

## 2. Three questions the design answers

1. **What state is the user in when they arrive?** They came to see someone
   they built. Sometimes they need to look inside the machine.
2. **What authority does the product claim?** "Your machine, your data, your
   hands." A private workroom, not a vendor's console.
3. **What should they feel when they leave?** That someone was there, and
   that they held the reins.

## 3. Two registers, one material

The dashboard has two kinds of surface. They share every token and differ
only in density and colour.

| | Living room | Workshop |
|---|---|---|
| Screens | Chat, the agent's face, questions the agent asks | MCP servers, cron, settings, memory, logs |
| Who faces whom | The person and their agent | The person and the machine |
| Density | Open. Body text 15–16 px, few controls | Dense. Tables, lists, numbers |
| Colour | The colour of the agent who is present | Neutral, plus status colours only |
| Corner radius | Composer 18 px, surfaces 10 px | Controls 6 px |

This removes a whole class of sameness at once: a workshop screen has no reason
to be a grid of cards, so it isn't one.

## 4. Six decisions

1. **Neutral scale.** Not a framework palette. A near-black tinted 4 % toward
   the present agent's hue, in four fixed steps: base L8 %, receding surface
   (sidebar) L12 %, raised surface L16 %, boundary L22 %. Separation is done
   with lightness, not lines; a 1 px rule is used only inside tables. Dark is
   the default by decision — this product lives at night. A light theme is the
   same four steps reversed, and is not yet drawn.
2. **Accent.** Only the colour of the agent currently present (Sapphy:
   `hsl(190 70% 58%)`), and only on things that belong to the agent: the face,
   the mark beside their words, the edge of a question they ask, the send
   button, their name on the conversation you are in. Never decoration. The
   workshop has no accent at all. A list of agents is monochrome except the
   one selected.
3. **Type.** IBM Plex Sans JP for everything read (body 15–16, UI 13.5, notes
   12.5; headings 600, letter-spacing 0). IBM Plex Mono only for identifiers,
   code and tabular numbers. The product bundles the fonts.
4. **Radius.** Three steps and no more: controls 6, surfaces 10, the composer
   18 — the one soft element on the screen.
5. **Fewer parts.** No pills. No icon on a button (icons belong to navigation
   and status, at 14 px). No avatar on every message — the face is in the
   header. Metadata is written as a sentence, not `A · B · C`. No shadow under
   a surface that already has a border. No motion that carries no meaning.
6. **The signature is the agent's presence.** An empty chat is not a blank
   page with a greeting; it is the agent, centred, with a line of their own
   and the threads you left with them. When you speak, the face moves to the
   header and the state ("thinking", "waiting for you") is a sentence there.
   The reasoning trace is the agent's *inner voice*, folded. A permission
   request is *a question from the agent*, in their words, with a 2 px edge in
   their colour. While they answer, send becomes stop and a cursor blinks.

## 5. What is deliberately not copied

The redesign borrows from products that feel calm — the constrained reading
column, the composer as a card, the conversation list — but borrows the
*decisions*, not the *styling*:

- The tool trace stays visible. An operator's console that hides what the
  agent did is the wrong trade.
- The agent axis stays. There is no single assistant here.
- Approvals, MCP state and kernel state stay inside the conversation, not in
  a drawer.
- Status colours stay. Full monochrome would throw away real information.
- No element without a reason: no mode switch the product does not have, no
  suggestion chips, no greeting copy written by the interface instead of the
  agent.

## 6. Scoring

Before a screen ships, score it against the checklist in the `ui-craft`
review procedure (25 signals: 22 that read as generated, 3 that read as
crafted) and write the count down. On 2026-09-16 the shipped dashboard scored
15/20, the first redesign 13/20, the samples in `gui/samples/` 1/20 (the font
is loaded from a CDN there).

## 7. What has to change underneath

Two of the decisions cannot be met by the front end alone:

- **Conversations need to exist.** `chat_messages` has no conversation id;
  the sidebar can list threads only once the kernel has them. `parent_id`
  already gives messages a tree, so this is an addition, not a rewrite.
- **Search needs an index.** There is no search anywhere except the
  marketplace.

Everything else — the composer, the reading column, the type, the tokens, the
two registers — is front-end work and can land screen by screen.

## 8. Sources

The measurements above are reproducible with `grep` over `dashboard/src`. The
reasoning about generated-looking interfaces draws on:

- Anthropic, *frontend-design* skill — patterns that recur regardless of
  subject, and the instruction to derive choices from the subject
- Paul Bakaus, *Impeccable* slop catalogue — decorative grids, glass without
  layering, hairline-plus-shadow, monotonous spacing
- Linear, *A calmer interface for a product in motion* (2026-03) — receding
  navigation, fewer and smaller icons, softer boundaries, unequal weight
- Andrei Nita, *Why AI-generated UI looks generic* — the three questions,
  accent discipline, writing standards where every session inherits them
