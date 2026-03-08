# ClotoCore Dashboard Component Map

GUI source root: `dashboard/src/`

Use `gui.read` to read any file listed below (path relative to `dashboard/src/`).

---

## Routes

| Path | Page Component | Description |
|------|---------------|-------------|
| `/` | `pages/AgentPage.tsx` | Agent chat (default page) |
| `/mcp-servers` | `pages/McpServersPage.tsx` | MCP server management |
| `/dashboard` | `components/MemoryCore.tsx` | Memory & episode viewer |
| `/cron` | `components/CronJobs.tsx` | Scheduled job management |
| `/status` | `components/KernelMonitor.tsx` | System kernel monitor |

---

## Directory Structure

### `pages/` — Route entry points
- `AgentPage.tsx` — Renders AgentTerminal as the main agent conversation view.
- `McpServersPage.tsx` — Renders MCP server list + detail panel (Master-Detail layout).

### `components/` — Core UI components

#### Agent Interaction
- `AgentTerminal.tsx` (27KB) — **Main agent UI**. Agent sidebar (list, create, delete, power toggle), conversation area, chat history, plugin workspace toggle. Central hub of the agent experience.
- `AgentConsole.tsx` (33KB) — Chat message display. Renders messages, thinking steps, tool calls, streaming responses. Handles SSE events (AgentThinking, ToolExecuted, etc).
- `ChatInputBar.tsx` — Message input field with send button. Supports multiline input.
- `AgentPowerButton.tsx` — Green/gray power button for toggling agent on/off.
- `PowerToggleModal.tsx` — Confirmation modal when toggling agent power (with optional password).
- `AgentPluginWorkspace.tsx` — Plugin/tool management panel for an agent.
- `EngineSelector.tsx` — LLM engine dropdown selector. Shows available MCP engine servers.
- `ServerAccessSection.tsx` — Displays and manages MCP server access grants for an agent.

#### Agent Creation & Identity
- `AvatarSection.tsx` — Agent avatar display and generation.
- `ProfileSection.tsx` — User/agent profile display.
- `SetupWizard.tsx` (8.5KB) — First-run setup flow. API key entry, default agent configuration.

#### Memory & Episodes
- `MemoryCore.tsx` (11KB) — **Dashboard view**. Displays agent memories (long-term) and episodes (episodic summaries). Filterable by agent. Shows metrics (memory count, RAM usage).

#### Cron Scheduler
- `CronJobs.tsx` (13KB) — Create, view, and manage scheduled autonomous jobs for agents. Cron expression input, execution history, enable/disable toggles.

#### MCP Server Management (`components/mcp/`)
- `McpServerList.tsx` — List of all MCP servers with status indicators (running/stopped/error).
- `McpServerDetail.tsx` — Detail panel for selected server (tabs: settings, logs, access control).
- `McpServerSettingsTab.tsx` (11KB) — Server configuration: command, args, env vars, restart policy.
- `McpServerLogsTab.tsx` — Real-time log output from MCP server.
- `McpAccessControlTab.tsx` (4.9KB) — Manage access control entries (server_grant, tool_grant, capability).
- `McpAccessTree.tsx` — Tree visualization of access control hierarchy.
- `McpAccessSummaryBar.tsx` — Summary bar showing access permission counts.

#### Settings (`components/settings/`)
- `SettingsView.tsx` — Main settings container with tabbed navigation.
- `GeneralSection.tsx` — General settings (language, default agent).
- `DisplaySection.tsx` — Display/theme preferences.
- `LlmProvidersSection.tsx` — LLM API key configuration (OpenAI, Anthropic, etc).
- `SecuritySection.tsx` (4.3KB) — Security settings, password management, API key rotation.
- `AdvancedSection.tsx` — Advanced configurations (YOLO mode, debug options).
- `LogSection.tsx` — System log viewer.
- `AboutSection.tsx` (8.7KB) — Version info, license, update checker.
- `common.tsx` — Shared setting UI primitives (section headers, toggles).

#### Authorization & Security
- `ApiKeyGate.tsx` — API key validation gate. Blocks access until valid key is entered.
- `SecurityGuard.tsx` — Permission request UI, security context provider.
- `CommandApprovalCard.tsx` — UI card for approving/denying agent command execution requests.

#### Chat Content Rendering
- `ContentBlockView.tsx` — Renders different message content types (text, images, code blocks, tool results).
- `CodeBlock.tsx` — Syntax-highlighted code rendering with copy button.
- `MarkdownRenderer.tsx` — Markdown to JSX converter.
- `TypewriterMessage.tsx` — Animated typing effect for message display.

#### Layout & Navigation
- `AppLayout.tsx` — Master layout wrapper (sidebar + header + content area).
- `AppSidebar.tsx` — Left sidebar with agent list and navigation links.
- `ViewHeader.tsx` — Top header bar with title, navigation, status info.
- `BranchNavigator.tsx` — Conversation branching/fork navigation.

#### System & Status
- `KernelMonitor.tsx` — System kernel monitoring view (CPU, memory, MCP server status).
- `SystemAlertCard.tsx` — System alert notification cards.
- `SkeletonThinking.tsx` — Loading/thinking animation placeholder.
- `ErrorBoundary.tsx` — React error boundary wrapper.

#### Visual & Theme
- `ThemeProvider.tsx` — Dark/light theme context provider.
- `ThemeToggle.tsx` — Theme switcher button.
- `InteractiveGrid.tsx` — Animated background grid effect.
- `CustomCursor.tsx` (9.4KB) — Custom animated cursor.
- `Modal.tsx` — Generic modal dialog wrapper.
- `HelpContent.tsx` — Help documentation modal content.
- `ArtifactPanel.tsx` — File/artifact display panel.

### `components/ui/` — Primitive UI components
- `StatusDot.tsx` — Status indicator dot (connected/offline/error colors).
- `AlertCard.tsx` — Alert card primitive.
- `GridBackground.tsx` — Animated background grid.

### `hooks/` — Custom React hooks
- `useAgents.ts` — Fetch and manage agent list.
- `useAgentCreation.ts` — Agent creation workflow (form state, validation, submission).
- `useMcpServers.ts` — Fetch MCP server list and status.
- `useApi.ts` — API client instance provider.
- `useApiKey.ts` — API key validation hook.
- `useConnectionStatus.ts` — Monitor backend WebSocket/SSE connection.
- `useEventStream.ts` — SSE event streaming subscription.
- `useRemoteData.ts` — Generic remote data fetching with loading/error states.
- `useAsyncAction.ts` — Async action wrapper with loading/error handling.
- `useTheme.ts` — Theme switching (dark/light/system).
- `useStorage.ts` — localStorage/sessionStorage utilities.
- `useMetrics.ts` — System metrics polling.
- `useArtifacts.ts` — Artifact management.
- `useTypewriter.ts` — Typewriter animation effect.
- `useLongPress.ts` — Long-press gesture detection.
- `useUserIdentity.ts` — User identity management.

### `contexts/` — React context providers
- `AgentContext.tsx` — Agent list, selected agent, system active state.
- `ApiKeyContext.tsx` — API key storage and validation.
- `ConnectionContext.tsx` — Backend connection status.
- `UserIdentityContext.tsx` — User identity and profile info.

### `services/` — API client
- `api.ts` — Central API client. Methods for agents, memories, MCP servers, cron jobs, settings.

### `lib/` — Utility libraries
- `tauri.ts` — Tauri desktop integration (window management, native APIs).
- `agentIdentity.tsx` — Agent avatar color/icon generation.
- `markdown.ts` — Markdown parsing utilities.
- `conversationTree.ts` — Conversation branching/fork logic.
- `errors.ts` — Error extraction and formatting.
- `json.ts` — JSON parsing utilities.
- `notifications.ts` — Toast notification system.
- `canvasUtils.ts` — Canvas drawing utilities.
- `Spinner.tsx` — Loading spinner component.

### `locales/en/` — Internationalization (English)
- `common.json`, `agents.json`, `mcp.json`, `memory.json`, `nav.json`, `settings.json`, `cron.json`, `wizard.json`

### Root files
- `main.tsx` — React bootstrap, router setup, lazy loading.
- `types.ts` — Core TypeScript interfaces (ClotoMessage, AgentMetadata, McpServerInfo, etc).
- `i18n.ts` — i18n configuration.
- `globals.d.ts` — TypeScript global declarations.

---

## Component Hierarchy

```
AppLayout
├── ViewHeader (top bar)
├── AppSidebar (left nav: agent list + nav links)
└── Router Outlet
    ├── AgentPage → AgentTerminal
    │   ├── Agent sidebar (create, select, delete, power)
    │   ├── AgentConsole (messages, thinking, tool calls)
    │   ├── ChatInputBar (user input)
    │   └── AgentPluginWorkspace (optional panel)
    │
    ├── McpServersPage
    │   ├── McpServerList (left panel)
    │   └── McpServerDetail (right panel)
    │       ├── McpServerSettingsTab
    │       ├── McpServerLogsTab
    │       └── McpAccessControlTab → McpAccessTree
    │
    ├── MemoryCore (dashboard)
    │   ├── Memory cards (long-term memories)
    │   └── Episode timeline (episodic summaries)
    │
    └── CronJobs (scheduler)

Global Contexts: ThemeProvider > ApiKeyProvider > ConnectionProvider > AgentProvider
```
