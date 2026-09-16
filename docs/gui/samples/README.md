# Dashboard UI samples

Static HTML mockups of the dashboard redesign described in
[`docs/DESIGN_PHILOSOPHY.md`](../../DESIGN_PHILOSOPHY.md). No backend, no
interaction: each file is one screen, rendered with real strings from the
Japanese locale so that line lengths and wrapping are the ones users will see.

| File | Screen | Register |
|---|---|---|
| `01-chat-empty.html` | Chat, no conversation yet — the agent is present, not a blank page | living room |
| `02-chat-conversation.html` | Chat in progress — inner voice, code, a question from the agent, streaming | living room |
| `03-agents.html` | Agents as a table with a split detail pane | workshop |
| `04-mcp-servers.html` | MCP servers as a table | workshop |
| `05-settings.html` | Settings as a page, not a modal | workshop |
| `06-memory.html` | Memory as a timeline with the episode stream beside it | workshop |

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
