# ClotoCore Dashboard Component Map

GUI source root: `dashboard/src/`

Use `gui.read` to read any file listed below (path relative to `dashboard/src/`).

---

## Routes

| Path | Page Component | Description |
|------|---------------|-------------|
| `/` | `pages/AgentPage.tsx` | Agent chat (default page) |
| `/mcp-servers` | `pages/McpServersPage.tsx` | MCP server management (servers tab + marketplace tab) |
| `/dashboard` | `components/MemoryCore.tsx` | Memory & episode viewer |
| `/cron` | `components/CronJobs.tsx` | Scheduled job management |
| `/vrm-viewer/:agentId` | `vrm/VrmViewerPage.tsx` | VRM 3D avatar viewer (separate window) |

System views (non-route):
- `components/KernelMonitor.tsx` — System kernel monitor (accessed via AgentPage system mode)
- `components/SettingsView.tsx` — Settings modal (opened from sidebar)

---

## Directory Structure

### `pages/` — Route entry points
- `AgentPage.tsx` — Renders AgentTerminal as the main agent conversation view.
- `McpServersPage.tsx` — Tab layout: Servers tab (card grid + detail modal) + Marketplace tab.

### `components/` — Core UI components

#### Agent Interaction
- `AgentTerminal.tsx` — **Main agent UI**. Agent card grid (create, select, power toggle), conversation area, chat history, plugin workspace toggle.
- `AgentConsole.tsx` — Chat message display. Renders messages, thinking steps, tool calls, streaming responses. Handles SSE events (AgentThinking, ToolExecuted, etc).
- `ChatInputBar.tsx` — Message input field with send button. Supports multiline input.
- `AgentPowerButton.tsx` — Green/gray power button for toggling agent on/off.
- `PowerToggleModal.tsx` — Confirmation modal when toggling agent power (with optional password).
- `AgentPluginWorkspace.tsx` — Agent configuration screen: avatar/VRM management, profile editing, MCP server access control. Deferred save pattern.
- `EngineSelector.tsx` — LLM engine dropdown selector. Shows available MCP engine servers.
- `ServerAccessSection.tsx` — Displays and manages MCP server access grants for an agent.

#### Agent Creation & Identity
- `AvatarSection.tsx` — Agent avatar display, upload, and deletion. VRM 3D model upload.
- `ProfileSection.tsx` — Agent name/description editing.
- `SetupWizard.tsx` — First-run setup flow (7 steps): welcome, API key, language, presets, server installation, quick guide, completion.

#### Memory & Episodes
- `MemoryCore.tsx` — **Dashboard view**. Displays agent memories (long-term) and episodes (episodic summaries). Filterable by agent. Shows metrics (memory count, RAM usage).

#### Cron Scheduler
- `CronJobs.tsx` — Create, view, and manage scheduled autonomous jobs for agents. Cron expression input, execution history, enable/disable toggles.

#### MCP Server Management (`components/mcp/`)
- `McpServerList.tsx` — Sidebar list of MCP servers with status indicators and filter.
- `McpServerDetail.tsx` — Detail modal for selected server (tabs: settings, access, logs).
- `McpServerSettingsTab.tsx` — Server configuration: command, args, env vars, restart policy.
- `McpServerLogsTab.tsx` — Real-time log output from MCP server.
- `McpAccessControlTab.tsx` — Manage access control entries (server_grant, tool_grant, capability).
- `McpAccessTree.tsx` — Tree visualization of access control hierarchy.
- `McpAccessSummaryBar.tsx` — Summary bar showing access permission counts.
- `MarketplaceTab.tsx` — Marketplace browser with search, category filter, install/uninstall.
- `MarketplaceCard.tsx` — Individual marketplace server card (name, description, tags, status, action buttons).
- `InstallDialog.tsx` — Server installation progress dialog.

#### Settings (`components/settings/`)
- `SettingsView.tsx` — Main settings container with sidebar navigation. Accepts `initialSection` prop.
- `GeneralSection.tsx` — Theme, language, user identity (display name with onBlur pattern).
- `DisplaySection.tsx` — Custom cursor toggle (localStorage).
- `LlmProvidersSection.tsx` — LLM API key configuration (OpenAI, Anthropic, etc).
- `SecuritySection.tsx` — Security settings, API key rotation.
- `AdvancedSection.tsx` — YOLO mode, cron recursion limit.
- `LogSection.tsx` — System event log viewer.
- `AboutSection.tsx` — Version info, auto-update toggle, manual update check/apply, license, setup rerun.
- `common.tsx` — Shared setting UI primitives: `SectionCard`, `Toggle`.
- `index.ts` — Barrel export for all setting sections.

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
- `AppLayout.tsx` — Master layout wrapper (header + sidebar + content). Manages settings modal with initialSection routing.
- `AppSidebar.tsx` — Left sidebar with agent list and navigation links.
- `ViewHeader.tsx` — Top header bar with title, navigation, connection status, update indicator (green arrow), window controls.
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
- `CustomCursor.tsx` — Custom animated cursor (toggleable in Display settings).
- `Modal.tsx` — Generic modal dialog wrapper.
- `HelpContent.tsx` — Help documentation modal content.
- `ArtifactPanel.tsx` — File/artifact display panel.

### `components/ui/` — Primitive UI components
- `StatusDot.tsx` — Status indicator dot (connected/offline/error colors).
- `AlertCard.tsx` — Alert card primitive.
- `ConfirmDialog.tsx` — Modal-based confirmation dialog with danger variant support.
- `EnvVariableEditor.tsx` — Key-value environment variable editor with visibility toggle.
- `SecretInput.tsx` — Password input with eye/eye-off visibility toggle.
- `SectionHeader.tsx` — Standardized section header with icon, title, and optional trailing content.
- `GridBackground.tsx` — Animated background grid.

### `vrm/` — VRM 3D Avatar System
- `VrmViewerPage.tsx` — Standalone VRM viewer page (opened in separate Tauri window).
- `VrmViewer.tsx` — VRM model display component (three.js canvas).
- `VrmContext.tsx` — VRM state context provider.
- `useVrmAvatar.ts` — Hook for VRM avatar lifecycle management.
- `useGazeBroadcast.ts` — Mouse gaze position broadcasting for VRM eye tracking.

#### `vrm/engine/` — VRM Animation Engine (Layered Motion Architecture)
- `VrmSceneManager.ts` — three.js scene setup (renderer, camera, lighting).
- `VrmModelLoader.ts` — VRM file loading and initialization via @pixiv/three-vrm.
- `VrmAnimationController.ts` — Master controller for layered procedural animation.
- `ProceduralBreathing.ts` — Layer 1: breathing animation (sine wave on spine/chest).
- `ProceduralBlinking.ts` — Layer 1: randomized blink animation.
- `ProceduralMicroSway.ts` — Layer 1: micro-sway via Perlin noise.
- `ProceduralGazeDrift.ts` — Layer 1: saccade simulation for eye movement.
- `AgentStateAnimator.ts` — SSE event integration (thinking/responding state transitions).
- `VrmExpressionMapper.ts` — Emotion-to-BlendShape mapping (ARKit + VRM preset fallback).
- `VisemePlayer.ts` — Lip sync viseme scheduling and playback.
- `AudioPlaybackManager.ts` — Audio playback for TTS synchronization.
- `DefaultPoseApplicator.ts` — Default T-pose correction for VRM models.
- `VrmaLoader.ts` — VRM Animation (.vrma) file loader.
- `types.ts` — VRM engine type definitions.

### `hooks/` — Custom React hooks
- `useAgents.ts` — Fetch and manage agent list.
- `useAgentCreation.ts` — Agent creation workflow (form state, validation, submission).
- `useMcpServers.ts` — Fetch MCP server list and status.
- `useMarketplace.ts` — Marketplace catalog fetching and state management.
- `useApi.ts` — API client instance provider (wraps api.ts with API key injection).
- `useApiKey.ts` — API key validation hook.
- `useConnectionStatus.ts` — Monitor backend WebSocket/SSE connection.
- `useEventStream.ts` — SSE event streaming subscription.
- `usePolling.ts` — Interval-based polling with automatic cleanup.
- `useRemoteData.ts` — Generic remote data fetching with loading/error states.
- `useAsyncAction.ts` — Async action wrapper with loading/error handling.
- `useTheme.ts` — Theme switching (dark/light/system).
- `useStorage.ts` — localStorage/sessionStorage utilities (`useLocalStorage`, `useSessionStorage`).
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
- `api.ts` — Central API client. Methods for agents, memories, MCP servers, cron jobs, settings, marketplace, avatars, VRM.

### `lib/` — Utility libraries
- `tauri.ts` — Tauri desktop integration (window management, update checker, native APIs).
- `agentIdentity.tsx` — Agent avatar color/icon generation (`AgentIcon` component).
- `presets.ts` — MCP server preset definitions for setup wizard.
- `markdown.ts` — Markdown parsing utilities.
- `conversationTree.ts` — Conversation branching/fork logic.
- `errors.ts` — Error extraction and formatting.
- `format.ts` — Display formatting utilities (`displayServerId`, etc).
- `json.ts` — JSON parsing utilities.
- `notifications.ts` — Toast notification system.
- `canvasUtils.ts` — Canvas drawing utilities.
- `Spinner.tsx` — Loading spinner component.

### `lib/__tests__/` — Unit tests
- `errors.test.ts` — Error extraction tests.
- `format.test.ts` — Display formatting tests.
- `json.test.ts` — JSON parsing tests.

### `locales/en/` — Internationalization (English)
- `common.json`, `agents.json`, `mcp.json`, `memory.json`, `nav.json`, `settings.json`, `cron.json`, `wizard.json`

### Root files
- `main.tsx` — React bootstrap, router setup, lazy loading, auto-update check on startup.
- `types.ts` — Core TypeScript interfaces (ClotoMessage, AgentMetadata, McpServerInfo, MarketplaceCatalogEntry, etc).
- `i18n.ts` — i18n configuration with external language pack loading.
- `globals.d.ts` — TypeScript global declarations.

### Build & Test
- `test/setup.ts` — Vitest test setup.

---

## Component Hierarchy

```
AppLayout
├── ViewHeader (top bar: nav, title, update indicator, connection status, window controls)
├── AppSidebar (left nav: agent list + nav links + settings button)
└── Router Outlet
    ├── AgentPage → AgentTerminal
    │   ├── Agent card grid (create, select, delete, power)
    │   ├── AgentConsole (messages, thinking, tool calls)
    │   ├── ChatInputBar (user input)
    │   └── AgentPluginWorkspace (config: avatar, profile, MCP access)
    │       ├── AvatarSection (avatar + VRM upload/delete)
    │       ├── ProfileSection (name, description)
    │       └── ServerAccessSection (MCP server grants)
    │
    ├── McpServersPage (tab layout)
    │   ├── Servers Tab → Server card grid → McpServerDetail (modal)
    │   │   ├── McpServerSettingsTab
    │   │   ├── McpServerLogsTab
    │   │   └── McpAccessControlTab → McpAccessTree
    │   └── Marketplace Tab → MarketplaceCard grid
    │       └── InstallDialog (progress)
    │
    ├── MemoryCore (dashboard)
    │   ├── Memory cards (long-term memories)
    │   └── Episode timeline (episodic summaries)
    │
    └── CronJobs (scheduler)

Settings (modal, opened from sidebar or update button)
├── GeneralSection (theme, language, identity)
├── SecuritySection (API keys)
├── DisplaySection (cursor toggle)
├── AdvancedSection (YOLO, cron limits)
├── LogSection (event viewer)
└── AboutSection (version, auto-update toggle, update check, license, setup rerun)

VrmViewerPage (separate Tauri window)
├── VrmViewer (three.js canvas)
└── VRM Engine (procedural animation layers)

Global Contexts: ThemeProvider > ApiKeyProvider > UserIdentityProvider > ConnectionProvider > AgentProvider
```
