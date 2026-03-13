# Database Schema

ClotoCore uses SQLite for persistent storage. Schema is managed via sequential migrations
in `crates/core/migrations/`.

**Database path:** Configured via `DATABASE_URL` (default: `sqlite:{exe_dir}/data/cloto_memories.db`)

---

## Tables

### plugin_settings

Plugin activation state and granted permissions.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `plugin_id` | TEXT | PRIMARY KEY | Plugin identifier (e.g. `mind.deepseek`) |
| `is_active` | BOOLEAN | NOT NULL DEFAULT 1 | Whether the plugin is enabled |
| `allowed_permissions` | TEXT | DEFAULT '[]' | JSON array of granted permission types |

### plugin_configs

Key-value configuration for individual plugins.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `plugin_id` | TEXT | PK (composite) | Plugin identifier |
| `config_key` | TEXT | PK (composite) | Configuration key |
| `config_value` | TEXT | | Configuration value |

### agents

Agent definitions with heartbeat-based status resolution.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | TEXT | PRIMARY KEY | Agent identifier (e.g. `agent.cloto_default`) |
| `name` | TEXT | NOT NULL | Display name |
| `description` | TEXT | | Human-readable description |
| `default_engine_id` | TEXT | NOT NULL | Default reasoning engine plugin ID |
| `status` | TEXT | DEFAULT 'offline' | Legacy status field (superseded by `enabled`) |
| `metadata` | TEXT | DEFAULT '{}' | JSON metadata |
| `required_capabilities` | TEXT | DEFAULT '["Reasoning", "Memory"]' | JSON array of required capability types |
| `enabled` | BOOLEAN | NOT NULL DEFAULT 1 | Whether the agent is active |
| `last_seen` | INTEGER | NOT NULL DEFAULT 0 | Last heartbeat timestamp (Unix ms) |
| `power_password_hash` | TEXT | DEFAULT NULL | Optional password hash for power toggle |
| `avatar_path` | TEXT | | Path to agent avatar image file |
| `avatar_description` | TEXT | | Vision-generated avatar description |

### plugin_data

Generic key-value storage for plugin state (SAL).

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `plugin_id` | TEXT | PK (composite) | Plugin identifier |
| `key` | TEXT | PK (composite) | Storage key |
| `value` | TEXT | | Stored value |
| `updated_at` | DATETIME | DEFAULT CURRENT_TIMESTAMP | Last update time |

### audit_logs

Security event audit trail with automatic 90-day cleanup.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | INTEGER | PRIMARY KEY AUTOINCREMENT | Auto-incrementing ID |
| `timestamp` | TEXT | NOT NULL | ISO-8601 timestamp |
| `event_type` | TEXT | NOT NULL | Event classification |
| `actor_id` | TEXT | | Who triggered the event |
| `target_id` | TEXT | | What was affected |
| `permission` | TEXT | | Related permission type |
| `result` | TEXT | NOT NULL | Outcome (e.g. `allowed`, `denied`) |
| `reason` | TEXT | | Human-readable reason |
| `metadata` | TEXT | | JSON metadata |
| `trace_id` | TEXT | | Request trace identifier |

**Indexes:** `timestamp`, `actor_id`, `event_type`, `trace_id`
**Trigger:** `audit_log_cleanup` — auto-deletes entries older than 90 days on INSERT.

### permission_requests

Human-in-the-loop permission workflow.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | INTEGER | PRIMARY KEY AUTOINCREMENT | Auto-incrementing ID |
| `request_id` | TEXT | NOT NULL UNIQUE | Unique request identifier |
| `created_at` | TEXT | NOT NULL | ISO-8601 creation time |
| `plugin_id` | TEXT | NOT NULL | Requesting plugin |
| `permission_type` | TEXT | NOT NULL | Permission being requested |
| `target_resource` | TEXT | | Target of the permission |
| `justification` | TEXT | NOT NULL | Why the permission is needed |
| `status` | TEXT | NOT NULL DEFAULT 'pending' | `pending` / `approved` / `denied` |
| `approved_by` | TEXT | | Who approved/denied |
| `approved_at` | TEXT | | When approved/denied |
| `expires_at` | TEXT | | Permission expiration time |
| `metadata` | TEXT | | JSON metadata |

**Indexes:** `status`, `plugin_id`, `created_at`, `request_id`

### chat_messages

Server-side chat message persistence with rich content support.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | TEXT | PRIMARY KEY | Message identifier |
| `agent_id` | TEXT | NOT NULL, FK → agents(id) | Target agent |
| `user_id` | TEXT | NOT NULL DEFAULT 'default' | User identifier |
| `source` | TEXT | NOT NULL, CHECK IN ('user','agent','system') | Message origin |
| `content` | TEXT | NOT NULL | JSON array of ContentBlock[] |
| `metadata` | TEXT | | Optional JSON metadata |
| `created_at` | INTEGER | NOT NULL | Unix timestamp (ms) |

**Index:** `(agent_id, user_id, created_at DESC)`

### chat_attachments

File attachments for chat messages. Small files (<=64KB) stored inline, larger files on disk.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | TEXT | PRIMARY KEY | Attachment identifier |
| `message_id` | TEXT | NOT NULL, FK → chat_messages(id) ON DELETE CASCADE | Parent message |
| `filename` | TEXT | NOT NULL | Original filename |
| `mime_type` | TEXT | NOT NULL | MIME content type |
| `size_bytes` | INTEGER | NOT NULL | File size in bytes |
| `storage_type` | TEXT | NOT NULL, CHECK IN ('inline','disk') | Storage strategy |
| `inline_data` | BLOB | | Binary data for <=64KB files |
| `disk_path` | TEXT | | File path for >64KB files |
| `created_at` | INTEGER | NOT NULL | Unix timestamp (ms) |

### runtime_plugins

Runtime plugin persistence for L5 Self-Extension. Stores plugins generated by Skill Manager at runtime.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `plugin_id` | TEXT | PRIMARY KEY | Plugin identifier |
| `script_name` | TEXT | NOT NULL | Script filename |
| `description` | TEXT | | Human-readable description |
| `code_content` | TEXT | NOT NULL | Generated code content |
| `permissions` | TEXT | DEFAULT '[]' | JSON array of required permissions |
| `created_at` | INTEGER | NOT NULL | Unix timestamp |
| `created_by` | TEXT | | Agent/user that created the plugin |
| `generation_number` | INTEGER | | Generation counter |
| `is_active` | BOOLEAN | NOT NULL DEFAULT 1 | Whether the plugin is enabled |

**Index:** `is_active`

### revoked_keys

Revoked API key hashes. A revoked key is permanently blocked even if still present in `.env`.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `key_hash` | TEXT | PRIMARY KEY | SHA-256 hash of the revoked key |
| `revoked_at` | INTEGER | NOT NULL | Unix timestamp of revocation |

### agent_plugins

Per-agent plugin assignment. Controls which tools are available to each agent.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `agent_id` | TEXT | PK (composite), FK → agents(id) ON DELETE CASCADE | Target agent |
| `plugin_id` | TEXT | PK (composite) | Plugin identifier |
| `pos_x` | INTEGER | NOT NULL DEFAULT 0 | Layout position X |
| `pos_y` | INTEGER | NOT NULL DEFAULT 0 | Layout position Y |

**Index:** `agent_id`

### mcp_servers

Dynamic MCP server persistence for restart restoration.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `name` | TEXT | PRIMARY KEY | Server identifier |
| `command` | TEXT | NOT NULL | Executable command |
| `args` | TEXT | NOT NULL DEFAULT '[]' | JSON array of arguments |
| `script_content` | TEXT | | Auto-generated script content |
| `description` | TEXT | | Server description |
| `created_at` | INTEGER | NOT NULL | Unix timestamp |
| `is_active` | BOOLEAN | NOT NULL DEFAULT 1 | Active state |
| `env` | TEXT | NOT NULL DEFAULT '{}' | JSON map of environment variables |
| `default_policy` | TEXT | NOT NULL DEFAULT 'opt-in' | `opt-in` (deny by default) / `opt-out` (allow by default) |

### mcp_access_control

Unified access control for MCP tool-level permissions.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | INTEGER | PRIMARY KEY AUTOINCREMENT | Auto-incrementing ID |
| `entry_type` | TEXT | NOT NULL, CHECK IN ('capability','server_grant','tool_grant') | Entry type |
| `agent_id` | TEXT | NOT NULL | Target agent |
| `server_id` | TEXT | NOT NULL | MCP Server ID |
| `tool_name` | TEXT | | Tool name (required for `tool_grant`) |
| `permission` | TEXT | NOT NULL DEFAULT 'allow' | `allow` / `deny` |
| `granted_by` | TEXT | | Who granted (user or system) |
| `granted_at` | TEXT | NOT NULL | ISO-8601 timestamp |
| `expires_at` | TEXT | | Expiration time |
| `justification` | TEXT | | Reason for grant/deny |
| `metadata` | TEXT | | JSON metadata |

**Indexes:** `(agent_id, server_id, tool_name)`, `(server_id)`, `(entry_type)`

**Access Resolution Priority:** `tool_grant` > `server_grant` > `default_policy`

### cron_jobs

Scheduled job definitions for periodic agent tasks.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | INTEGER | PRIMARY KEY AUTOINCREMENT | Auto-incrementing ID |
| `agent_id` | TEXT | NOT NULL | Target agent |
| `schedule_type` | TEXT | NOT NULL | Schedule type (e.g. `cron`, `interval`) |
| `schedule_value` | TEXT | NOT NULL | Schedule expression |
| `next_run_at` | INTEGER | | Next run Unix timestamp |
| `last_run_at` | INTEGER | | Last run Unix timestamp |
| `last_status` | TEXT | | Last execution status |
| `last_error` | TEXT | | Last error message |
| `max_iterations` | INTEGER | | Maximum iterations (NULL = unlimited) |

**Indexes:** `next_run_at`, `agent_id`

### llm_providers

Centralized LLM provider API key management.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | TEXT | PRIMARY KEY | Provider identifier (e.g. `deepseek`, `cerebras`) |
| `display_name` | TEXT | NOT NULL | Human-readable name |
| `api_url` | TEXT | NOT NULL | API endpoint URL |
| `api_key` | TEXT | NOT NULL DEFAULT '' | API key |
| `model_id` | TEXT | NOT NULL | Default model identifier |
| `timeout_secs` | INTEGER | NOT NULL DEFAULT 120 | Request timeout |
| `enabled` | BOOLEAN | NOT NULL DEFAULT 1 | Whether the provider is active |

### trusted_commands

Per-agent command approval for the terminal security system.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | INTEGER | PRIMARY KEY AUTOINCREMENT | Auto-incrementing ID |
| `agent_id` | TEXT | NOT NULL | Target agent |
| `pattern` | TEXT | NOT NULL | Command pattern (exact match) |
| `pattern_type` | TEXT | NOT NULL DEFAULT 'exact' | Match type |

**Index:** `agent_id`

---

## Migration History

| File | Description |
|------|-------------|
| `20260205000000_init.sql` | Initial schema (plugin_settings, plugin_configs, agents) |
| `20260206000000_add_permissions.sql` | Add `allowed_permissions` to plugin_settings |
| `20260207000000_add_sal_storage.sql` | Add plugin_data table (SAL) |
| `20260208100000_add_agent_capabilities.sql` | Add `required_capabilities` to agents |
| `20260208200000_optimize_plugin_data.sql` | Add index on plugin_data |
| `20260212000000_final_seeds.sql` | Seed default plugins, configs, agents |
| `20260213000000_add_audit_logs.sql` | Add audit_logs table |
| `20260213120000_add_permission_requests.sql` | Add permission_requests table |
| `20260214000000_add_constraints.sql` | Drop redundant index, add audit cleanup trigger |
| `20260217000000_add_chat_persistence.sql` | Add chat_messages and chat_attachments tables |
| `20260217100000_rename_python_analyst.sql` | Rename agent to "Python Bridge" |
| `20260218000000_agent_heartbeat_power.sql` | Add enabled/last_seen/power_password_hash to agents |
| `20260219000000_add_runtime_plugins.sql` | Add runtime plugin registration |
| `20260219120000_add_revoked_keys.sql` | Add revoked API keys table |
| `20260220000000_add_agent_plugins.sql` | Add agent_plugins table |
| `20260220000001_fix_agent_engine_type.sql` | Fix agent engine type |
| `20260220000002_grant_memory_permissions.sql` | Grant memory permissions |
| `20260222000000_add_mcp_servers.sql` | Add mcp_servers table |
| `20260223000000_add_mcp_access_control.sql` | Add mcp_access_control + mcp_servers.default_policy |
| `20260225000000_rename_exiv_to_cloto.sql` | Rename exiv_default → cloto_default |
| `20260227000000_add_mcp_server_env.sql` | Add `env` column to mcp_servers |
| `20260228000000_add_cron_jobs.sql` | Add cron_jobs table |
| `20260228100000_add_llm_providers.sql` | Add llm_providers table with default providers |
| `20260301000000_default_policy_opt_out.sql` | Change MCP default_policy to opt-out |
| `20260302000000_cerebras_model_update.sql` | Update Cerebras model to gpt-oss-120b |
| `20260302100000_add_agent_avatar.sql` | Add avatar_path/avatar_description to agents |
| `20260303000000_sanitize_agent_ids.sql` | Replace '/' with '_' in agent IDs |
| `20260303100000_update_default_agent_description.sql` | Update default agent description |
| `20260304000000_add_trusted_commands.sql` | Add trusted_commands table |
| `20260304100000_add_claude_provider.sql` | Add Claude (Anthropic) LLM provider |
