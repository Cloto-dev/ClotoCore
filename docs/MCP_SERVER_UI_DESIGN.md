# MCP Server Management UI Design

> **Status:** Implemented (v0.5.3)
> **Updated:** 2026-03-04
> **Related:** `MCP_PLUGIN_ARCHITECTURE.md` Section 6, `ARCHITECTURE.md`, `SCHEMA.md`

---

## 1. Motivation

### 1.1 Background

The backend has already migrated to an MCP-only architecture (`MCP_PLUGIN_ARCHITECTURE.md`).
The legacy Plugin UI (ClotoPluginManager.tsx, AgentPluginWorkspace.tsx, PluginConfigModal.tsx)
was completely removed in v0.5.3 and replaced with the MCP Server Management UI.

### 1.2 Design Decision

**Rather than patching the legacy Plugin UI, a new MCP Server Management UI was built from scratch.**

- The legacy Plugin UI architecture itself was incompatible with MCP concepts
- Fundamentally resolved God Component / Double-save issues
- MCP server lifecycle management is qualitatively different from legacy plugin activate/deactivate

---

## 2. Design Decisions

| # | Topic | Options | Adopted | Rationale |
|---|-------|---------|---------|-----------|
| 1 | Access control granularity | Per-server / Per-tool | **Per-tool** | Risk level differs per tool (e.g. `execute_command` vs `recall`) |
| 2 | Default policy | opt-in / opt-out | **opt-in** (deny by default) | Safer default; can be changed to opt-out per server |
| 3 | Layout | Master-Detail / Card Grid + Modal | **Card Grid + Modal** | Coexistence with persistent sidebar layout; balance between overview and modal details |
| 4 | Access control UI | Matrix / Tree | **Directory hierarchy Tree** | Intuitively represents parent-child relationships of entries |
| 5 | Data model | Separate tables / Unified | **Unified** (`mcp_access_control`) | Centralized management of legacy `permission_requests` + new tool access |

---

## 3. Data Model

### 3.1 New Table: `mcp_access_control`

Unifies the legacy `permission_requests` table with the new MCP tool access control.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | INTEGER | PRIMARY KEY AUTOINCREMENT | Auto-incrementing ID |
| `entry_type` | TEXT | NOT NULL, CHECK IN ('capability', 'server_grant', 'tool_grant') | Entry type |
| `agent_id` | TEXT | NOT NULL, FK → agents(id) | Target agent |
| `server_id` | TEXT | NOT NULL | MCP Server ID (e.g. `tool.terminal`) |
| `tool_name` | TEXT | | Tool name (required only for `tool_grant`) |
| `permission` | TEXT | NOT NULL DEFAULT 'allow' | `allow` / `deny` |
| `granted_by` | TEXT | | Grantor (UI operator or `system`) |
| `granted_at` | TEXT | NOT NULL | ISO-8601 timestamp |
| `expires_at` | TEXT | | Expiration date (NULL = no expiration) |
| `justification` | TEXT | | Reason for allow/deny |
| `metadata` | TEXT | | JSON metadata |

**Indexes:**
- `(agent_id, server_id, tool_name)` — For access resolution
- `(server_id)` — Per-server listing
- `(entry_type)` — Type filtering

### 3.2 entry_type Definitions

| entry_type | Meaning | server_id | tool_name | Tree Level |
|------------|---------|-----------|-----------|-----------|
| `capability` | Agent capability request (equivalent to legacy `permission_requests`) | Requested server | NULL | Level 0 (root) |
| `server_grant` | Blanket allow/deny for entire server | Target server | NULL | Level 1 |
| `tool_grant` | Allow/deny for individual tool | Target server | Target tool name | Level 2 |

### 3.3 Access Resolution Logic (Priority Rule)

Permission determination when an agent invokes a tool:

```
1. If tool_grant exists → use its permission
2. If server_grant exists → use its permission
3. If neither exists → use the server's default_policy
     - default_policy = "opt-in"  → deny (default)
     - default_policy = "opt-out" → allow
```

**Priority: tool_grant > server_grant > default_policy**

### 3.4 `mcp_servers` Table Extension

Add a `default_policy` column to the existing MCP Server configuration:

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `default_policy` | TEXT | NOT NULL DEFAULT 'opt-in' | `opt-in` (deny by default) / `opt-out` (allow by default) |

---

## 4. API Design

### 4.1 Existing API (Maintained)

| Method | Route | Description |
|--------|-------|-------------|
| GET | `/api/mcp/servers` | MCP Server list (includes status, tools) |
| POST | `/api/mcp/servers` | Register MCP Server |
| DELETE | `/api/mcp/servers/:id` | Stop and delete MCP Server |

### 4.2 New API

#### Server Settings

| Method | Route | Description |
|--------|-------|-------------|
| GET | `/api/mcp/servers/:id/settings` | Get server settings (config, default_policy) |
| PUT | `/api/mcp/servers/:id/settings` | Update server settings |

#### Access Control

| Method | Route | Description |
|--------|-------|-------------|
| GET | `/api/mcp/servers/:id/access` | Access control list (tree structure) |
| PUT | `/api/mcp/servers/:id/access` | Batch update access controls |
| GET | `/api/mcp/access/by-agent/:agent_id` | Access list from agent perspective |

### 4.3 Server Lifecycle

| Method | Route | Description |
|--------|-------|-------------|
| POST | `/api/mcp/servers/:id/restart` | Restart MCP Server |
| POST | `/api/mcp/servers/:id/start` | Start MCP Server |
| POST | `/api/mcp/servers/:id/stop` | Stop MCP Server (without deletion) |

---

## 5. UI Design

### 5.1 Card Grid + Modal Detail (v0.5.3)

Since v0.5.3 introduced a persistent sidebar layout (`AppSidebar`),
the MCP page adopts a **Card Grid + Modal Detail** pattern instead of Master-Detail.

```
┌──────────────────────────────────────────────────────────────┐
│  🔌 MCP Servers   5 servers · 4 running     [↻] [+ Add Server] │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐           │
│  │ 🔌 tool.    │ │ 🔌 mind.    │ │ 🔌 mind.    │           │
│  │ terminal    │ │ deepseek    │ │ cerebras    │           │
│  │ ● Running   │ │ ● Running   │ │ ○ Stopped   │           │
│  │ 2 tools     │ │ 2 tools SDK │ │ 2 tools SDK │           │
│  └─────────────┘ └─────────────┘ └─────────────┘           │
│  ┌─────────────┐ ┌─────────────┐                            │
│  │ 🔌 memory.  │ │ 🔌 tool.    │                            │
│  │ ks22        │ │ embedding   │                            │
│  │ ● Running   │ │ ● Running   │                            │
│  │ 3 tools SDK │ │ 2 tools     │                            │
│  └─────────────┘ └─────────────┘                            │
│                                                              │
└──────────────────────────────────────────────────────────────┘

Click → Modal (16:9 large) to display details
```

**Card Information:**
- Server ID (font-mono, bold)
- Status dot (● Running / ○ Stopped / ● Error)
- Tool count
- Badges: SDK (ClotoSDK-compatible), CONFIG (loaded from config file)

**Modal Detail:**
- Display `McpServerDetail` in Modal (size="lg")
- 3-tab layout: Settings / Access / Logs
- Lifecycle action buttons (Start / Stop / Restart / Delete)

### 5.2 Add Server Modal

Display form in `Modal` (size="sm"):
- Server Name (validation: `^[a-z][a-z0-9._-]{0,62}[a-z0-9]$`)
- Command (default: `python3`)
- Arguments (space-separated)

### 5.3 Access Tab: Directory Hierarchy Tree

```
┌─────────────────────────────────────────────────────────────┐
│  Access Control — tool.terminal                              │
│  Default Policy: [opt-in ▼]                                  │
│                                                              │
│  ▼ agent.cloto_default                                        │
│    ├─ 📁 Server Grant: tool.terminal        [Allow ▼]        │
│    │   ├─ 🔧 execute_command                [Deny  ▼]        │
│    │   └─ 🔧 list_processes                 [Allow ▼]  (inherited)
│    └─ ...                                                    │
└─────────────────────────────────────────────────────────────┘
```

### 5.4 Settings Tab

Display and edit server settings (command, transport, auto-restart), environment variables, and manifest information.

---

## 6. Component Architecture (v0.5.3)

### 6.1 Current Component Structure

```
pages/
  McpServersPage.tsx              ← Root page (card grid + modal management)

components/
  Modal.tsx                       ← Shared modal (size: sm / lg)

components/mcp/
  McpServerDetail.tsx             ← In-modal: detail container (tab switching)
  McpServerSettingsTab.tsx        ← Settings tab
  McpAccessControlTab.tsx         ← Access tab (Tree + Summary Bar)
  McpAccessTree.tsx               ← Directory hierarchy tree
  McpAccessSummaryBar.tsx         ← Per-tool summary
  McpServerLogsTab.tsx            ← Logs tab
  McpServerList.tsx               ← Server list (legacy, integrated into McpServersPage)
```

### 6.2 Legacy Components (Removed in v0.5.3)

| Legacy Component | Replacement |
|-----------------|-------------|
| `ClotoPluginManager.tsx` | `McpServersPage.tsx` (card grid) |
| `AgentPluginWorkspace.tsx` | `McpAccessControlTab.tsx` |
| `PluginConfigModal.tsx` | `McpServerSettingsTab.tsx` |
| `McpAddServerModal.tsx` | `Modal` + inline form within `McpServersPage.tsx` |

---

## 7. Implementation Status

### Phase A: Backend — Partially Complete

- [x] MCP Server CRUD API (`/api/mcp/servers`)
- [x] Server Lifecycle API (start / stop / restart)
- [ ] Create `mcp_access_control` table (SQLite migration)
- [ ] Add `default_policy` column to `mcp_servers`
- [ ] Access control API (settings, access)
- [ ] Access resolution logic (`resolve_access()`)

### Phase B: Frontend — Partially Complete

- [x] `McpServersPage.tsx` — Card grid + Modal layout
- [x] `McpServerDetail.tsx` — Tabbed detail view
- [x] `McpServerSettingsTab.tsx` — Settings CRUD
- [x] `McpServerLogsTab.tsx` — Log display
- [ ] `McpAccessControlTab.tsx` — Tree UI + Summary Bar (awaiting backend API)

### Phase C: Cleanup — Complete

- [x] Removed legacy Plugin UI components
- [x] Removed legacy fields from `types.ts`
