# Trigger Layer Design — Heartbeat & Cron for Autonomous Agent Execution

> Status: Research / Design Reference
> Date: 2026-02-28
> Based on: OpenClaw architecture analysis (v237K+ stars, Feb 2026)

## 1. Overview

Autonomous AI agents require a **Trigger Layer** — mechanisms that initiate agent
execution without human prompts. This document covers two complementary patterns:

| Pattern | Purpose | Frequency |
|---------|---------|-----------|
| **Heartbeat** | Periodic "wake up and check" signal | Fixed interval (e.g. 30 min) |
| **Cron** | Scheduled task execution | Cron expression / one-shot / interval |

Together they enable agents to act proactively — monitoring email, checking
calendars, running health checks, sending briefings — all without a human typing
a message.

## 2. Heartbeat Mechanism

### 2.1 Core Concept

A heartbeat is a **periodic signal** sent to the agent's LLM with a checklist
of tasks. The agent evaluates each task, takes action if needed, and reports
back. If nothing requires attention, the agent returns a sentinel token
(`HEARTBEAT_OK`) which the system **silently discards**.

```
Timer fires (internal, not OS cron)
  │
  ▼
Check HEARTBEAT.md existence & content
  ├── File missing or effectively empty → SKIP (no LLM call, save tokens)
  │
  ▼
Assemble prompt context:
  ・System prompt + persona
  ・Session history (if running in main session)
  ・HEARTBEAT.md content
  ・Pending system events
  │
  ▼
Default heartbeat prompt:
  "Read HEARTBEAT.md. Follow it strictly.
   Do not infer or repeat old tasks from prior chats.
   If nothing needs attention, reply HEARTBEAT_OK."
  │
  ▼
LLM processes context → generates response
  │
  ▼
Inspect response:
  ├── HEARTBEAT_OK at start/end AND remaining ≤ ackMaxChars (300)
  │   → Silent discard. Restore session lastUpdatedAt.
  │
  └── No HEARTBEAT_OK (actionable content)
      → Deliver to configured channel (WhatsApp/Slack/Dashboard/etc.)
```

### 2.2 HEARTBEAT.md Format

HEARTBEAT.md is a workspace-level file loaded into prompt context on every tick.
It serves as the agent's autonomous checklist.

```markdown
# Heartbeat Checks

## Cadence-Based Rotating Checks

On each heartbeat:
1. Read heartbeat-state.json
2. Calculate which check is most overdue (respect time windows)
3. Run that check only
4. Update timestamp in state file
5. Report only if check finds something actionable
6. Return HEARTBEAT_OK if nothing needs attention

## Check Schedule
- **Email**: every 30 min (9 AM - 9 PM only)
- **Calendar**: every 2 hours (8 AM - 10 PM only)
- **Tasks**: every 30 min (anytime)
- **Git**: every 24 hours (anytime)
- **System**: every 24 hours (3 AM only)

## Email Check
**Report ONLY if:** new email from authorized sender with actionable request
**Update:** email timestamp in state file

## Calendar Check
**Report ONLY if:** event starting in < 2 hours, or new since last check
**Update:** calendar timestamp in state file

## Git Check
**Report ONLY if:** uncommitted changes exist or unpushed commits found
**Update:** git timestamp in state file

## System Check
**Report ONLY if:** failed cron jobs found or recent errors in logs
**Update:** system timestamp in state file
```

Companion state file (`heartbeat-state.json`):

```json
{
  "lastChecks": {
    "email": 1703275200000,
    "calendar": 1703260800000,
    "tasks": 1703270000000,
    "git": 1703250000000,
    "system": 1703240000000
  }
}
```

### 2.3 Design Rules

| Rule | Rationale |
|------|-----------|
| Keep HEARTBEAT.md small | Injected every tick — prompt bloat costs tokens |
| Never put secrets in HEARTBEAT.md | Becomes part of LLM prompt context |
| Skip effectively empty files | Blank lines + headings only → no LLM call |
| One check per tick (rotating cadence) | Spreads load, prevents simultaneous firing |
| Two-tier evaluation | Tier 1: cheap deterministic checks (file timestamps, API counts). Tier 2: LLM judgment only if Tier 1 finds something |

### 2.4 HEARTBEAT_OK Suppression Logic

```
Gateway receives agent reply:
  │
  ▼
Strip HEARTBEAT_OK from start/end of text
  │
  ▼
Remaining content ≤ ackMaxChars (default: 300 chars)?
  ├── YES → Drop reply silently
  │         Restore session lastUpdatedAt (idle-expiry unaffected)
  │
  └── NO  → Deliver full alert content to user
```

Per-channel configuration:
- `showOk: true` — send HEARTBEAT_OK acknowledgment to user (default: false)
- `showAlerts: true` — deliver non-OK alert content (default: true)

### 2.5 Interval Configuration

| Setting | Value |
|---------|-------|
| Default interval | `30m` (30 minutes) |
| Minimum | No hard minimum (shorter = higher token cost) |
| Disable | `0m` |
| Format | Duration string: `5m`, `30m`, `1h`, `2h30m` |

Configuration paths:
- Global: `agents.defaults.heartbeat.every`
- Per-agent: `agents.list[].heartbeat.every`

Example configuration:

```json
{
  "agents": {
    "defaults": {
      "heartbeat": {
        "every": "30m",
        "model": "anthropic/claude-sonnet-4-5",
        "target": "last",
        "prompt": "Read HEARTBEAT.md if it exists. Follow it strictly. If nothing needs attention, reply HEARTBEAT_OK.",
        "directPolicy": "allow"
      }
    }
  }
}
```

### 2.6 Concurrency & Session Isolation

**Mid-task behavior**: When the heartbeat timer fires while the agent is already
executing, the Lane Queue serialization prevents conflict:

- Heartbeat request is enqueued on the per-session lane
- Per-session concurrency cap = 1 → heartbeat waits for current run to complete
- Heartbeat is **deferred, not dropped**
- Coalescing (250ms) prevents redundant scheduling if multiple ticks pile up

**Session modes**:

| Mode | Context | Token Cost | Use Case |
|------|---------|-----------|----------|
| Main session | Full conversation history | High (~170K-210K input tokens) | Contextual decisions |
| Isolated session | Clean, no prior history | Low | Deterministic checks |

**Production recommendation**: Use isolated cron-based heartbeat to avoid the
170K+ token cost of main session context loading on every tick.

### 2.7 Decision Tree for Heartbeat Responses

```
Heartbeat fires
  │
  ▼
Agent reads HEARTBEAT.md checklist
  │
  ▼
Agent runs checks (email, calendar, tasks, etc.)
  │
  ├── Nothing actionable found
  │   → Reply: HEARTBEAT_OK
  │   → Gateway suppresses silently
  │
  └── Something actionable found
      │
      ├── Can be handled autonomously?
      │   → Take action (send email, create task, etc.)
      │   → Report what was done to user
      │
      └── Needs user input/decision?
          → Message user with summary + question
```

## 3. Cron Job System

### 3.1 Core Concept

Cron jobs are **scheduled tasks** that fire at specific times, intervals, or as
one-shot events. Unlike heartbeat (which is a periodic "check everything" signal),
cron jobs execute **specific messages** in **isolated sessions**.

### 3.2 Job Definition Format

```json
{
  "settings": {
    "enabled": true,
    "maxConcurrentRuns": 2,
    "sessionRetention": "24h",
    "runLog": {
      "maxBytes": 10485760,
      "keepLines": 1000
    }
  },
  "jobs": [
    {
      "id": "morning-brief",
      "name": "Morning brief",
      "schedule": {
        "kind": "cron",
        "expr": "0 7 * * *",
        "tz": "Asia/Tokyo"
      },
      "sessionTarget": "isolated",
      "payload": {
        "kind": "agentTurn",
        "message": "Summarize overnight updates, new emails, and today's calendar."
      },
      "delivery": {
        "mode": "announce",
        "channel": "slack",
        "to": "channel:C1234567890",
        "bestEffort": true
      },
      "model": "anthropic/claude-sonnet-4-5",
      "thinking": "medium",
      "enabled": true
    },
    {
      "id": "weekly-review",
      "name": "Weekly code review",
      "schedule": {
        "kind": "cron",
        "expr": "0 9 * * 1"
      },
      "sessionTarget": "isolated",
      "payload": {
        "kind": "agentTurn",
        "message": "Review open PRs and summarize status."
      },
      "delivery": { "mode": "announce" }
    },
    {
      "id": "reminder-dentist",
      "name": "Dentist appointment reminder",
      "schedule": {
        "kind": "at",
        "at": "2026-03-15T09:00:00+09:00"
      },
      "sessionTarget": "main",
      "payload": {
        "kind": "systemEvent",
        "text": "Reminder: Dentist appointment at 10:00 AM today."
      },
      "deleteAfterRun": true
    }
  ]
}
```

### 3.3 Schedule Types

| Type | `schedule.kind` | Example | Use Case |
|------|----------------|---------|----------|
| Cron expression | `"cron"` | `"0 7 * * *"` (daily 7 AM) | Recurring tasks |
| Fixed interval | `"every"` | `"everyMs": 7200000` (2h) | Periodic polling |
| One-shot | `"at"` | `"2026-03-15T09:00:00"` | Reminders, deadlines |

### 3.4 Creation Methods

**CLI**:
```bash
# Recurring cron job
openclaw cron add \
  --name "Morning brief" \
  --cron "0 7 * * *" \
  --tz "Asia/Tokyo" \
  --session isolated \
  --message "Summarize overnight updates."

# One-shot reminder
openclaw cron add \
  --name "Dentist reminder" \
  --at "2026-03-15T09:00:00" \
  --session main \
  --message "Dentist at 10 AM."

# Fixed interval
openclaw cron add \
  --name "Health check" \
  --every 2h \
  --message "Run system health check."
```

**Management**:
```bash
openclaw cron list                # List all jobs
openclaw cron edit <jobId>        # Update job
openclaw cron delete <jobId>      # Remove job
openclaw cron run <jobId>         # Trigger immediate execution
```

**Agent self-creation**: The agent can autonomously create cron jobs through:
1. Natural language → cron definition parsing (cron-creator skill)
2. Direct exec tool invocation of `openclaw cron add`
3. Within a heartbeat/cron run, creating follow-up jobs

This enables **self-reinforcing autonomy** — the agent manages its own schedule.

### 3.5 Cron vs Heartbeat

| Aspect | Heartbeat | Cron Job |
|--------|-----------|---------|
| Session | Main session (default) | Isolated (default) |
| Context | Full conversation history | Clean session (no prior history) |
| Model | Agent's default | Overridable per-job |
| Delivery | Reply to heartbeat prompt | `announce` mode (summary) |
| History | Adds to main history | Does not pollute main history |
| Thinking | Agent default | Configurable per-job |
| Cost | High (~170K tokens/tick) | Low (no history loading) |
| Use case | Contextual checks | Deterministic task execution |

### 3.6 Session Isolation

**`sessionTarget: "isolated"` (default)**:
- Fresh, ephemeral session per run
- No prior conversation history loaded
- Cleaned up after `sessionRetention` (default: 24h)
- Results delivered via `announce` mode

**`sessionTarget: "main"`**:
- Runs within the agent's main session
- Full conversation context available
- Payload kind must be `"systemEvent"` (not `"agentTurn"`)
- Use sparingly — pollutes main session history

### 3.7 Error Handling

| Failure Mode | Behavior |
|-------------|----------|
| Task execution failure | Retry up to 3 times (announce retry) |
| Announce delivery timeout | Hardcoded 60s timeout |
| `bestEffort: true` | Job not marked failed if announce fails |
| Infinite retry loop | Known issue: isolated tasks without failure limit cause runaway retries |

Run history: Stored as JSONL at `~/.openclaw/cron/runs/<jobId>.jsonl`,
auto-pruned by `runLog.maxBytes` and `runLog.keepLines`.

### 3.8 State Persistence Between Runs

For **isolated** cron jobs, no automatic state persistence exists. The agent must
use external mechanisms:
- Workspace files (e.g., `heartbeat-state.json`)
- Memory system (long-term memory store)
- Database or external service

For **main-session** cron jobs, state persists naturally through session history.

## 4. Gateway Daemon Architecture

### 4.1 Overview

The Gateway is a **long-lived process** that maintains:
- Channel connections (messaging platforms)
- Session state
- WebSocket control plane
- Heartbeat/cron schedulers

It binds to `127.0.0.1:18789` (loopback only, for security) serving both
WebSocket and HTTP on the same port.

### 4.2 Service Configuration

**Linux (systemd)**:

```ini
[Unit]
Description=Agent Gateway
After=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/agent-gateway --port 18789
Restart=always
RestartSec=5
MemoryMax=2G

[Install]
WantedBy=default.target
```

Enable with `systemctl --user enable --now` + `loginctl enable-linger`.

**macOS (launchd)**:

```xml
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><true/>
```

**Windows**: Foreground process or wrapped with NSSM as a Windows Service.

### 4.3 Lane Queue — Concurrency Control

```
Inbound message/heartbeat/cron trigger
  │
  ▼
Access control check
  │
  ▼
Session resolution (agent:<id>:<key>)
  │
  ▼
Per-session lane (concurrency cap: 1)
  │   ↑ Guarantees serial execution per session
  ▼
Global lane (main=4, subagent=8)
  │   ↑ Caps total parallelism to prevent rate limits
  ▼
Agent Runner executes
  │
  ▼
Response delivered to originating channel
```

Key properties:
- **Per-session serialization**: 1 session = 1 concurrent execution
- **Global throttle**: Prevents upstream LLM rate-limit hits
- **Heartbeat lane**: Separate command queue — never blocks real-time messages
- **Typing indicators**: Fire immediately on enqueue (before run starts)

## 5. Self-Prompting Patterns

### 5.1 Two-Tier Evaluation

Optimizes cost by separating cheap checks from expensive LLM reasoning:

| Tier | Method | Cost | Example |
|------|--------|------|---------|
| **Tier 1** | Deterministic scripts/tools | Near-zero | Check inbox count, file timestamps, API status codes |
| **Tier 2** | LLM judgment | Token cost | Evaluate urgency, draft response, decide action |

Tier 2 is only invoked if Tier 1 finds something actionable.

### 5.2 Rotating Cadence Pattern

Instead of running all checks every tick, rotate through them based on
last-run timestamps:

```
Heartbeat fires
  │
  ▼
Read heartbeat-state.json
  │
  ▼
Calculate most overdue check:
  email:    last=30min ago, cadence=30min → 0min overdue
  calendar: last=3h ago,   cadence=2h    → 1h overdue ← WINNER
  git:      last=20h ago,  cadence=24h   → not yet due
  │
  ▼
Run calendar check only
  │
  ▼
Update calendar timestamp in state file
```

Benefits:
- Spreads load across ticks
- Prevents all checks firing simultaneously
- Each tick costs ~1 check instead of N checks

### 5.3 Real-World Autonomous Actions

| Action | Trigger | Behavior |
|--------|---------|----------|
| Morning briefing | Cron `0 7 * * *` | Summarize email, calendar, tasks → WhatsApp |
| Email monitoring | Heartbeat (30 min) | Check for actionable emails → report if found |
| Calendar alerts | Heartbeat (2h) | Warn if event starting in < 2 hours |
| Git workspace | Heartbeat (24h) | Report uncommitted changes |
| System health | Heartbeat (24h, 3 AM) | Check for failed cron jobs, log errors |
| Content publishing | Cron (weekly) | Draft, review, and schedule blog posts |
| Data backup | Cron (daily, 2 AM) | Run backup script, report status |

## 6. Security Considerations

### 6.1 Attack Surface

| Vector | Risk | Mitigation |
|--------|------|------------|
| HEARTBEAT.md poisoning | Attacker writes malicious instructions into checklist | Integrity checks, file permissions, read-only mount |
| Cron persistence | Attacker creates scheduled tasks that re-inject malicious logic | Audit cron creation, require approval for new jobs |
| Token exhaustion | Short intervals drain API budget | Minimum interval enforcement, cost monitoring |
| Session history leakage | Main-session heartbeats expose conversation history to LLM | Use isolated sessions for sensitive checks |
| Self-modifying schedules | Agent creates unbounded cron jobs | Max job count limit, human approval for new jobs |

### 6.2 Recommended Safeguards

1. **Approval gate for cron creation**: Agent-created cron jobs require human
   confirmation before activation
2. **Cost budgets**: Per-agent daily token limit for heartbeat/cron executions
3. **Isolated sessions**: Default to isolated for all scheduled tasks
4. **Audit logging**: All heartbeat/cron executions logged with timestamps,
   token usage, and actions taken
5. **Heartbeat file integrity**: Hash-based verification of HEARTBEAT.md to
   detect unauthorized modifications
6. **Rate limiting**: Minimum interval enforcement (e.g., 5 minutes) to prevent
   accidental token drain

## 7. Implementation Notes (OpenClaw Reference)

### 7.1 Key Source Files

| File | Responsibility |
|------|---------------|
| `src/infra/heartbeat-runner.ts` | Core execution, empty-file check, LLM invocation |
| `src/infra/heartbeat-wake.ts` | Timer scheduling, coalescing (`DEFAULT_COALESCE_MS = 250`) |
| `src/cron/service/timer.ts` | Cron job scheduling, system event integration |
| `src/gateway/server-methods.ts` | Health check endpoints, HTTP interface |

### 7.2 Known Issues (OpenClaw)

| Issue | Impact | Status |
|-------|--------|--------|
| Heartbeat interval > ~24.85 days causes infinite CPU loop | Node.js `setTimeout` 32-bit limit | Open (#28405) |
| Isolated cron without failure limit → infinite retry loop | Runaway API calls, rate limit exhaustion | Open (#8520) |
| Main-session heartbeat costs ~170K-210K tokens/tick | Significant API cost at 30-min intervals | By design |
| Announce delivery hardcoded 60s timeout | Long-running announcements fail silently | Open (#14540) |

## 8. References

- [OpenClaw Heartbeat Documentation](https://docs.openclaw.ai/gateway/heartbeat)
- [OpenClaw Cron Jobs Documentation](https://docs.openclaw.ai/automation/cron-jobs)
- [OpenClaw Cron vs Heartbeat](https://docs.openclaw.ai/automation/cron-vs-heartbeat)
- [OpenClaw Command Queue Concepts](https://docs.openclaw.ai/concepts/queue)
- [OpenClaw Architecture Part 1: Control Plane & Sessions](https://theagentstack.substack.com/p/openclaw-architecture-part-1-control)
- [OpenClaw Architecture Part 2: Concurrency & Isolation](https://theagentstack.substack.com/p/openclaw-architecture-part-2-concurrency)
- [Heartbeats: Cheap Checks First (DEV)](https://dev.to/damogallagher/heartbeats-in-openclaw-cheap-checks-first-models-only-when-you-need-them-4bfi)
- [3 Superpowers of OpenClaw (Kryll)](https://blog.kryll.io/openclaw-hooks-cron-heartbeat-ai-agent-automation/)
