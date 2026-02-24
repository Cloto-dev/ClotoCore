# WASM Plugin Design (Tier 3)

> **Status:** Superseded. MCP adapter now provides dynamic tool integration without WASM complexity.
> **Superseded by:** `plugins/mcp/` — Model Context Protocol adapter with runtime server management
> **Original Target Phase:** Post-E7 (after Evolution Engine YOLO mode stabilization)
>
> **Decision (2026-02-22):** Plugin system simplified from 4 formats (Rust/Python Bridge/WASM/MCP)
> to 2 formats (Rust + MCP). Python Bridge deleted, WASM plan superseded.
> MCP provides language-agnostic tool integration via industry-standard protocol,
> covering the same use cases with lower implementation cost.
> This document is retained as historical reference.

---

## 1. Motivation

ClotoCore's plugin system currently operates in two tiers:

- **Tier 1 (Compiled):** Rust plugins discovered at compile-time via `inventory`. Maximum performance, zero runtime overhead, but requires recompilation for changes.
- **Tier 2 (Script):** Python plugins loaded at runtime via the Python Bridge. Dynamic generation and registration (L5), AST-based security, subprocess isolation.

**Tier 3 (WASM)** addresses the gap: **sandboxed, near-native performance plugins that can be distributed and loaded without recompilation or a Python runtime.**

### Why not dlopen/libloading?

| Concern | dlopen | WASM |
|---------|--------|------|
| ABI Stability | Rust ABI is unstable; version mismatch = UB | Stable Component Model interface |
| Safety | Requires `unsafe`; full system access | Sandboxed by design |
| Bevy precedent | Deprecated as "unsound" in 0.14 | Increasingly adopted |
| Alignment with Principle #5 | Violates permission isolation | Perfect fit |

### Why WASM?

- **Complete sandboxing:** No filesystem, network, or syscall access unless explicitly granted via imports
- **Language-agnostic:** Plugins can be written in Rust, C, Go, AssemblyScript, etc.
- **Deterministic execution:** No UB, no data races across plugin boundaries
- **Portable distribution:** Single `.wasm` file works on all platforms

---

## 2. Architecture

```
┌────────────────────────────────────────────────────────┐
│ Tier 1: Compiled Plugins (Rust, inventory)              │
│   Performance-critical, official plugins                │
│   #[cloto_plugin] macro, Magic Seal validation           │
├────────────────────────────────────────────────────────┤
│ Tier 2: Script Plugins (Python Bridge)                  │
│   Runtime generation (L5), AST security inspection      │
│   DB persistence (runtime_plugins table)                │
│   Agent-driven code generation via Skill Manager        │
├────────────────────────────────────────────────────────┤
│ Tier 3: Sandboxed Plugins (WASM) [PROPOSED]             │
│   Third-party distribution, near-native performance     │
│   wasmtime Component Model, WIT interface               │
│   Capability-based security via WASI                    │
└────────────────────────────────────────────────────────┘
```

---

## 3. Component Interface (WIT)

WebAssembly Interface Types (WIT) define the plugin contract:

```wit
package cloto:plugin@0.1.0;

interface types {
    record plugin-manifest {
        id: string,
        name: string,
        description: string,
        version: string,
        category: string,
        service-type: string,
        tags: list<string>,
        required-permissions: list<string>,
        provided-capabilities: list<string>,
        provided-tools: list<string>,
    }

    record tool-result {
        success: bool,
        data: string,       // JSON-encoded result
        error: option<string>,
    }

    record event-data {
        event-type: string,
        payload: string,    // JSON-encoded payload
    }
}

interface plugin {
    use types.{plugin-manifest, tool-result, event-data};

    manifest: func() -> plugin-manifest;
    on-init: func(config: list<tuple<string, string>>) -> result<_, string>;
    on-event: func(event: event-data) -> option<event-data>;
}

interface tool {
    use types.{tool-result};

    name: func() -> string;
    description: func() -> string;
    parameters-schema: func() -> string;  // JSON schema
    execute: func(args: string) -> tool-result;
}

interface reasoning {
    think: func(agent-id: string, message: string, context: string) -> result<string, string>;
}

world cloto-plugin {
    import cloto:host/storage;
    import cloto:host/network;
    import cloto:host/events;

    export plugin;
    export tool;
}
```

---

## 4. Host Capabilities (Imports)

WASM plugins access host capabilities through explicit imports:

```wit
interface storage {
    get: func(key: string) -> option<string>;
    set: func(key: string, value: string) -> result<_, string>;
    delete: func(key: string) -> result<_, string>;
}

interface network {
    http-get: func(url: string, headers: list<tuple<string, string>>) -> result<string, string>;
    http-post: func(url: string, body: string, headers: list<tuple<string, string>>) -> result<string, string>;
}

interface events {
    emit: func(event-type: string, payload: string);
}
```

Each import is gated by the plugin's `required_permissions`. If `NetworkAccess` is not granted, the `network` import traps on call.

---

## 5. Loading & Registration

### Plugin Discovery

```
plugins/wasm/
├── my_plugin.wasm          # Compiled WASM component
├── my_plugin.wasm.sig      # Ed25519 signature (optional, for verified plugins)
└── my_plugin.toml          # Metadata override (optional)
```

### Registration Flow

1. Kernel scans `plugins/wasm/` directory on startup
2. For each `.wasm` file:
   a. Verify signature if `.sig` exists (reject unsigned in production mode)
   b. Instantiate with wasmtime, call `manifest()` to get metadata
   c. Validate Magic Seal equivalent (SDK version check)
   d. Register in PluginRegistry with scoped permissions
3. Runtime loading via API: `POST /api/plugins/wasm/load`

### WasmPluginLoader

```rust
// crates/core/src/managers/wasm_loader.rs (proposed)
pub struct WasmPluginLoader {
    engine: wasmtime::Engine,
    linker: wasmtime::component::Linker<HostState>,
}

impl WasmPluginLoader {
    pub async fn load(&self, wasm_path: &Path, permissions: &[Permission])
        -> anyhow::Result<Arc<dyn Plugin>>;

    pub async fn load_from_bytes(&self, bytes: &[u8], permissions: &[Permission])
        -> anyhow::Result<Arc<dyn Plugin>>;
}
```

---

## 6. Security Model

| Layer | Mechanism |
|-------|-----------|
| Compilation | WASM validation (type safety, memory safety) |
| Loading | Signature verification + SDK version check |
| Runtime | Capability-based imports (no implicit access) |
| Network | Routed through SafeHttpClient (host whitelist) |
| Storage | ScopedDataStore (plugin namespace isolation) |
| Execution | Fuel-based limits (prevent infinite loops) |
| Evolution | SafetyGate fitness tracking + rollback |

---

## 7. Performance Considerations

- **Overhead:** 5-25% vs native Rust (acceptable for most plugins)
- **Serialization:** JSON encoding at boundaries (main bottleneck)
- **Mitigation:** Cache frequently-used plugins, batch operations
- **Not suitable for:** High-frequency event handlers, vision processing

---

## 8. Implementation Roadmap

| Step | Description | Dependency |
|------|-------------|------------|
| W1 | Add `wasmtime` dependency to Cargo.toml | None |
| W2 | Define WIT interfaces in `wit/` directory | None |
| W3 | Implement `WasmPluginLoader` | W1, W2 |
| W4 | Implement host capability imports (storage, network, events) | W3 |
| W5 | Add `plugins/wasm/` directory scanning at startup | W3 |
| W6 | Add `POST /api/plugins/wasm/load` endpoint | W3 |
| W7 | Signature verification for production mode | W5 |
| W8 | Dashboard integration (WASM plugin management UI) | W6 |

---

## 9. Dependencies

```toml
[dependencies]
wasmtime = "29"
wasmtime-wasi = "29"
```

Estimated binary size increase: ~5-10MB (wasmtime runtime).

---

## 10. Alternatives Considered

| Approach | Decision | Reason |
|----------|----------|--------|
| `libloading` + `repr(C)` | Rejected | Requires `unsafe`, no sandboxing, ABI fragility |
| `abi_stable` | Rejected | High complexity, no sandboxing, version coupling |
| `dlopen2` | Rejected | Same issues as libloading |
| Lua/Rhai scripting | Not considered | Python Bridge already fills this niche |
| **wasmtime** | **Selected** | Best alignment with security principles |
