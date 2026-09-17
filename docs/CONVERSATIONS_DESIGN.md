# Conversations — Design

**Status:** Proposed
**Author:** kernel team · 2026-09-17
**Related:** `DESIGN_PHILOSOPHY.md` §4.6 and §7 ("conversations need to
exist"), `RECALL_SESSION_SCOPE_V2_DESIGN.md` (long-term recall scope, which
this does not change), `crates/core/src/managers/session_manager.rs` (the
in-memory transcript this demotes to a cache)

The dashboard's chat is one flat log per agent with no beginning, no end and
no way back. This document gives the dashboard **conversations** in the shape
every current chat product has settled on: a conversation is a persistent
thread you can leave and return to at any time; the model reads *that
thread* as its context; long-term memory is a separate layer that crosses
threads. Nothing ends, nothing expires, and nothing has to be closed.

---

## 1. Problem & current state

All findings below were confirmed by code inspection on 2026-09-17.

### There is no conversation

`chat_messages(id, agent_id, user_id, source, content, metadata, created_at,
parent_id, branch_index)` — one log per `(agent_id, user_id)`
(`migrations/20260217000000_add_chat_persistence.sql`,
`20260305000000_add_chat_branching.sql`). The API is one list,
`GET /api/chat/{agent_id}/messages` (newest 50, paged by `before`). The only
history operation is **Reset**, which deletes everything
(`AgentConsole.tsx:805`, `DELETE /api/chat/{agent_id}/messages`).

### The model does not read the log

The kernel never loads `chat_messages` as model input. It writes the table
for display (`save_chat_message_reliable`, `handlers/system.rs:557`, `:1088`)
and reads one row back only to resolve attachments (`:3196`). The context
handed to the engine (`think_with_tools(agent, message, context, …)`,
`system.rs:2901`) is the memory server's recall (`memory_context_limit` rows)
merged with an **in-memory transcript** keyed by
`metadata["external_session_id"]` (`SessionManager`, `system.rs:742`).

The dashboard never sends that key (`AgentConsole.tsx:595-648`), so every
dashboard message falls to the kernel's cron default — its own session,
`kernel:{msg.id}` (`system.rs:476-489`) — and the transcript is always empty.
On the dashboard the previous turn reaches the model only when long-term
recall happens to return it. The Discord bridge, by contrast, sends a chunk
id (`servers/discord/src/bridge.rs:105-140`, forwarded at `events.rs:523`),
so Discord gets a transcript for up to 24 hours and the dashboard gets none.

### What "the same as every other chat product" means, concretely

Read from the vendors' own help pages on 2026-09-17 (sources in §7):

| | ChatGPT | Claude.ai | Gemini | Dashboard today |
|---|---|---|---|---|
| Unit | a chat, "saved to your account until you delete [it] manually", listed in the sidebar | a conversation, listed | a chat, listed under Recent | one log per agent |
| Ending | none — you start a new one or come back | none | none | none, but for the wrong reason |
| Model context | "remembers context within a chat" | "context from the current thread" | (not stated) | recall + an empty transcript |
| Memory | a separate layer: saved memories + reference to past chats, global across chats | a separate layer, per project / global | activity log | CPersona recall (this part already matches) |
| Rename / pin | rename, pin | rename | rename, pin | — |
| Archive | yes: hides from the sidebar, kept under Settings › Data controls, unarchive, still searchable, same retention | no | no | — |
| Delete | removed at once, unrecoverable, purged within 30 days; memories derived from it survive | removed at once, purged within 30 days; memories survive | removed, also from the activity log | Reset deletes everything |
| Bulk | archive all / delete all, including chats in projects | delete selected | — | — |
| Temporary chat | not saved, no memory written; can be saved later | incognito | — | — |

Three things every product agrees on: nothing ends, memory outlives the
conversation it came from, and delete is immediate for the person. Only
ChatGPT has archive, and its archive is a *hidden, not gone* state: out of
the sidebar, still searchable, still under the normal retention rules, and
reversible.

## 2. Decisions

### (a) The conversation is the unit, and it lives in the database

```sql
CREATE TABLE conversations (
    id          TEXT PRIMARY KEY,          -- UUID v4, minted by the kernel
    agent_id    TEXT NOT NULL REFERENCES agents(id),
    user_id     TEXT NOT NULL DEFAULT 'default',
    title       TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,          -- ms
    updated_at  INTEGER NOT NULL,          -- ms; the newest message's time
    archived_at INTEGER                    -- NULL = live
);
ALTER TABLE chat_messages ADD COLUMN conversation_id TEXT;
CREATE INDEX idx_chat_messages_conversation ON chat_messages(conversation_id, created_at);
CREATE INDEX idx_conversations_agent ON conversations(agent_id, user_id, archived_at, updated_at DESC);
```

`parent_id` / `branch_index` stay as they are: branches are *inside* a
conversation, exactly as an edited-and-regenerated turn is today.

### (b) The kernel mints the id; the client names the conversation it is in

- `POST /api/chat/{agent_id}/conversations` → `{ id, title: "", created_at }`.
  **New chat** calls this and switches to the returned id.
- Every message the dashboard sends carries `metadata.conversation_id`. The
  kernel persists the user message and the reply under it and bumps
  `updated_at`.
- A message without a `conversation_id` from a client that has the header is
  refused (400) — silently starting a fresh conversation per message is the
  defect this design removes.
- The dashboard remembers the open conversation per agent in `localStorage`,
  so a reload lands where the person was; if that conversation is gone, the
  list is shown.

Why the kernel and not the client: the id is a database key that other
clients (a browser session, a future bridge) must be able to look up; a
client-minted id would be a name the kernel has to trust.

### (c) The model reads the conversation

For a message with a `conversation_id`, the prior turns handed to the engine
are **that conversation's messages, newest-first up to a budget**, read from
`chat_messages` at dispatch time. The budget is `max_conversation_context`
(default: 40 turns; an agent-level override later if a model's window needs
it). Long-term recall is merged as it is today, de-duplicated by message id
(`system.rs:742-757`), so a memory of this same conversation is not shown
twice.

`SessionManager` keeps its role for **tool history within a run** and for
bridges; for the dashboard it becomes a cache: `external_session_id` is set
to the `conversation_id`, so the tier model and `recall_policy` keep working
unchanged, but the transcript is no longer the source of truth — the database
is, and it survives a restart. Reopening a conversation from a week ago gives
the model the same turns the person is looking at.

Why the database and not a bigger transcript: the transcript is in-memory
by design (Principle 1.4) and evicts after 24 hours; a conversation that can
be reopened months later cannot depend on it.

### (d) Nothing ends

There is no "end session" action, no timer and no gap that splits a
conversation. A conversation is left by opening another or starting a new
one, and returned to by opening it. `updated_at` orders the list; the
sidebar groups it as *Today / Yesterday / Previous 7 days / Older* on the
client.

Episodes in the memory server are unaffected: the volume-based archival
(`maybe_archive_episode`, ten unarchived memories, per channel) keeps running
in the background as it does now. Memories stored during a conversation carry
its id as their `session_id` (`system.rs:1404-1420` already forwards the key
when present), so the memory server can tell one conversation's memories from
another's. An explicit per-conversation episode is not part of this design.

### (e) Titles come from the first message, then from the model

On the first user message, `title` = the first line of that message, cut at
60 characters. After the first reply, the kernel asks the agent's engine for
a title of at most eight words (`call_engine_think_simple`, the same helper
that writes episode summaries) and stores it; on failure the first-line title
stands. `PATCH /api/chat/{agent_id}/conversations/{id}` renames by hand.

### (f) Archive: hidden, not gone

Archive is the state ChatGPT gives it, and nothing more:

- **Hidden from the list.** `PATCH … { "archived_at": <ms> }` sets it; the
  sidebar and `GET …/conversations` omit archived conversations unless
  `include_archived=true`.
- **Kept whole.** Messages, attachments and the conversation row are
  untouched. Nothing about retention changes; there is no timer.
- **Reversible.** `PATCH … { "archived_at": null }` returns it to the list in
  its old position (`updated_at` is not bumped by archiving).
- **Still found.** Search, when it exists, covers archived conversations; the
  memory server is unaffected, since memories were never tied to the list.
- **Reachable.** Settings gains an *Archived conversations* section listing
  them per agent with *Unarchive* and *Delete*; "Archive all" for an agent
  lives there too.

Opening an archived conversation from search or settings and sending a
message does **not** unarchive it; the person does that on purpose.

### (g) Delete: immediate and permanent

`DELETE /api/chat/{agent_id}/conversations/{id}` removes the conversation
row and its messages; attachments cascade as they do today. It is immediate
and not recoverable — this is a local application with no server-side
retention window to hide behind, so there is no "within 30 days". The
dashboard confirms before deleting, as it does for an agent.

Memories the memory server stored during the conversation are **not**
deleted with it — the same rule ChatGPT and Claude state for their own
memory. Forgetting a memory is the memory server's own operation.

"Delete all" for an agent lives next to "Archive all" in settings and
includes archived conversations.

**Reset** is removed. Its two meanings — "start over" and "get rid of this" —
are New chat and Delete.

### (h) Existing history becomes one conversation per agent and user

The migration creates one conversation per distinct `(agent_id, user_id)`
that has messages, titled by its date range ("2026-03-09 – 2026-09-17"), and
assigns every existing row to it. No row is moved, split or dropped; the
count of messages before and after the migration must be equal and is
asserted in the migration test. Splitting old history by time gaps would be a
guess about conversations that were never drawn as such; one thread is what
the person actually saw.

## 3. API

| Route | Purpose |
|---|---|
| `GET /api/chat/{agent_id}/conversations?user_id=&include_archived=` | list, newest `updated_at` first: `{ id, title, created_at, updated_at, archived_at, message_count }` |
| `POST /api/chat/{agent_id}/conversations` | create; returns the id |
| `PATCH /api/chat/{agent_id}/conversations/{id}` | `title`, `archived_at` (a timestamp archives, `null` unarchives) |
| `DELETE /api/chat/{agent_id}/conversations/{id}` | delete with messages |
| `POST /api/chat/{agent_id}/conversations/archive-all` · `…/delete-all` | the bulk actions behind settings |
| `GET /api/chat/{agent_id}/messages?conversation_id=` | the existing list, now filtered; `conversation_id` becomes required once the dashboard sends it |
| `POST /api/chat` | unchanged shape; `metadata.conversation_id` required from the dashboard |

## 4. What changes

| Where | Change |
|---|---|
| `crates/core/migrations/` | the two statements in (a) plus the backfill in (h), one migration |
| `crates/core/src/db.rs` | conversation CRUD; `get_conversation_context(id, budget)`; `save_chat_message*` take the id |
| `crates/core/src/handlers/chat.rs` | the routes in §3; `get_messages` filters by conversation |
| `crates/core/src/handlers/system.rs` | dispatch: load the conversation's turns as `context` when the id is present; set `external_session_id` = id; title after the first reply |
| `dashboard/src/components/AppSidebar.tsx` | the conversation list (New chat, groups by day, the agent's name on each row, "…" → rename / archive / delete) |
| `dashboard/src/components/AgentConsole.tsx` | open a conversation; send with the id; remove Reset |
| `dashboard/src/components/settings/` | *Archived conversations* (list, unarchive, delete) and the two bulk actions |
| `dashboard/src/services/api.ts` | the five calls |
| `docs/ARCHITECTURE.md` | the data-model table gains `conversations` |

The Discord path does not change: its messages have no `conversation_id`,
keep their chunk session and their transcript. A later design may give
bridges conversations too; nothing here prevents it.

## 5. Verification

Each row is a behaviour this design introduces, with the mutation that must
turn its test red.

| Behaviour | Test | Mutation |
|---|---|---|
| Messages do not cross conversations | kernel: two conversations on one agent, a message in each → the context built for the second contains none of the first's turns | drop the `conversation_id` filter from `get_conversation_context` |
| The model sees the conversation's turns after a restart | kernel: write turns, rebuild `AppState` on the same DB, dispatch → context holds them | read the context from `SessionManager` instead of the DB |
| The budget holds | kernel: 50 turns, budget 40 → the 40 newest, in order | drop the limit; take the oldest |
| Old history is kept whole | migration test: N rows before → N rows after, all with one `conversation_id` per `(agent, user)` | skip the backfill; assign per row |
| A dashboard message without an id is refused | kernel: `POST /api/chat` with the dashboard header and no id → 400 | fall back to a fresh conversation |
| The dashboard sends the open conversation's id | component: open A, send → body carries A; New chat → the next body carries the new id | send the agent id; mint per send |
| Delete removes the messages too | kernel: delete → `get_messages` empty, attachments gone | delete the row only |
| Archive hides without losing | kernel: archive → absent from the list, present with `include_archived`, messages intact; unarchive → back in the list at the old `updated_at` | delete on archive; bump `updated_at`; list ignores the flag |
| Delete leaves memories alone | kernel with a recording memory server: delete → zero `delete_memory` calls | call the memory server on delete |
| The title is written once from the model and never overwritten by later replies | kernel with a recording engine: two replies → one title request | request on every reply |

The visual verification tier drives the whole path once the routes exist:
new chat, three turns, reload, reopen from the list, and the reply that
proves the model saw the earlier turns.

## 6. Out of scope

- Search across conversations (a separate design; this one gives it the
  table to index, and the rule that archived conversations are included).
- Pinning, projects / folders, and a temporary (unsaved) chat. Each is a
  column or a flag on the table this design creates, so none needs a second
  data model; they are left out to keep the first change small.
- Conversations for bridges.
- The empty state's "continue from" list and the sidebar's exact drawing,
  which follow `DESIGN_PHILOSOPHY.md` and the per-screen work.

## 7. Sources

Read on 2026-09-17. The comparison in §1 quotes them; where a page does not
say something (auto-titling, time grouping, context-window size), the table
says so rather than guessing.

- OpenAI Help Center — *How to delete and archive chats in ChatGPT*
  (help.openai.com/en/articles/8809935), *Chat and file retention policies*
  (…/8983778), *Memory FAQ* (…/8590148), *Temporary Chat FAQ* (…/8914046),
  *How do I search my chat history* (…/10056348), *Projects in ChatGPT*
  (…/10169521), *ChatGPT release notes* (…/6825453, pinning and renaming).
- Claude Help Center — *Delete or rename a conversation*
  (support.claude.com/en/articles/8230524), *Use Claude's chat search and
  memory* (…/11817273).
- Gemini Apps Help — *Find & manage your recent chats*
  (support.google.com/gemini/answer/13666746).
