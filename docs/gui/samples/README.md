# Dashboard UI samples

Static HTML mockups of the dashboard redesign described in
[`docs/DESIGN_PHILOSOPHY.md`](../../DESIGN_PHILOSOPHY.md). No backend, no
interaction: each file is one screen, rendered with real strings from the
Japanese locale so that line lengths and wrapping are the ones users will see.

| File | Screen | Register |
|---|---|---|
| `01-chat-empty.html` | Chat, no conversation yet — the agent is present, not a blank page | living room |
| `02-chat-conversation.html` | Chat in progress — inner voice, code, a question from the agent, streaming | living room |
| `03-agents.html` | Agents as a roster (face, name, one line of state) with a detail pane | workshop |
| `04-mcp-servers.html` | MCP servers in one column, grouped by capability, each with a sentence saying what it is for; status written only when it deviates | workshop |
| `05-settings.html` | Settings as a page, not a modal | workshop |
| `06-memory.html` | Memory on a vertical time axis — days as thin bands, empty stretches compressed, a 30-day density strip beside it | workshop |

View them with any static server, one folder, one port:

```sh
python3 -m http.server 4100 --directory docs/gui/samples
# then open http://localhost:4100/01-chat-empty.html
```

The typeface (IBM Plex Sans JP / IBM Plex Mono) is loaded from Google Fonts in
these samples only; the product bundles it. Offline, the system Japanese font
is used instead.

These files are excluded from the documentation site (`gui/` in
`mkdocs.yml`); they are working material for the redesign, not user docs.
