# E7: CapabilityGain Trigger Design

## Overview

The `CapabilityGain` trigger fires a generation transition when an agent
acquires new capabilities. Unlike other triggers which are based on fitness
metrics (numerical thresholds), CapabilityGain detects **structural changes**
in the agent's plugin/capability configuration.

This document records the design decisions made before implementation so that
future contributors can understand *why* the system works the way it does.

---

## Capability Layers in Exiv

Exiv represents "capabilities" at three layers:

```
Layer 3:  CapabilityType   (Reasoning, Memory, Communication, Tool, Vision, HAL, Web)
Layer 2:  Plugin           (each plugin declares capabilities + tools in its manifest)
Layer 1:  Tool             (concrete executable functions implementing the Tool trait)
```

### What constitutes a "capability gain"

| Detection Level | Example | Granularity |
|-----------------|---------|-------------|
| New CapabilityType appears | First Vision plugin activated | Coarse (7 categories) |
| New plugin activated | `vision_yolo` added to active_plugins | Medium |
| New tool available | `image_gen` added to provided_tools | Fine |

**Decision: Composite detection (Layer 3 + Layer 2)**

- **Major gain**: A new `CapabilityType` appears that was absent in the
  previous generation. Emits `EvolutionCapability` with severity "major".
- **Minor gain**: A new plugin becomes active that provides only already-known
  `CapabilityType`s. Emits `EvolutionCapability` with severity "minor".
- Layer 1 (individual tools) is intentionally excluded to avoid excessive
  trigger noise from routine plugin updates.

---

## Detection Architecture

### Separation of concerns

The evolution engine has two independent detection paths:

```
evaluate(scores, snapshot)
    │
    ├─ check_triggers(scores, prev_scores, params)   ← metric-based (pure function)
    │      Returns: SafetyBreach | Regression | Evolution | AutonomyUpgrade | Rebalance
    │
    ├─ detect_capability_gain(prev_snapshot, snapshot) ← structure-based (set diff)
    │      Returns: Vec<CapabilityChange>
    │
    └─ resolve(metric_trigger, capability_changes)     ← integration
           Returns: final GenerationTrigger + events
```

**Rationale**: `check_triggers()` is a pure function operating on numerical
inputs (f64 scores). Capability detection operates on set-valued inputs
(`Vec<String>` plugin lists). Mixing these in one function would conflate two
fundamentally different kinds of analysis:

| | Metric detection | Structure detection |
|---|---|---|
| Input type | f64 | Vec\<String\> |
| Operation | Threshold comparison | Set difference (A ∖ B ≠ ∅) |
| Determinism | Same scores → same result | Depends on plugin registry state |

### Detection method

```rust
fn detect_capability_gain(
    prev_snapshot: &AgentSnapshot,
    curr_snapshot: &AgentSnapshot,
    plugin_registry: &PluginRegistry,  // to resolve plugin_id → capabilities
) -> Vec<CapabilityChange>
```

1. Compute `new_plugins = curr.active_plugins ∖ prev.active_plugins`
2. For each new plugin, look up its `provided_capabilities` from the registry
3. Compute `prev_capabilities = ∪ { capabilities(p) | p ∈ prev.active_plugins }`
4. For each new plugin's capabilities:
   - If capability ∉ prev_capabilities → **major** gain
   - Otherwise → **minor** gain
5. Return the list of changes

---

## Trigger Priority

When `evaluate()` detects both a metric-based trigger and capability changes,
the final trigger is resolved according to a strict priority order.

### Priority table

```
Priority  Trigger           Category     Rationale
────────  ────────────────  ───────────  ──────────────────────────────────
1 (highest) SafetyBreach    Defensive    Safety invariant violation is unconditional
2         Regression        Defensive    New capability may be causing the regression
─── defensive / growth boundary ───
3         CapabilityGain    Growth       Structural change has higher explanatory value
4         AutonomyUpgrade   Growth       Autonomy level increase
5         Rebalance         Growth       Axis balance shift
6 (lowest)  Evolution       Growth       Default positive growth
```

### Resolution logic

```
metric_trigger = check_triggers(...)
cap_changes    = detect_capability_gain(...)

final_trigger =
    if metric_trigger ∈ {SafetyBreach, Regression}:
        metric_trigger                    // safety-first: defensive triggers always win
    elif cap_changes ≠ ∅:
        CapabilityGain                    // structural change overrides generic growth
    else:
        metric_trigger                    // use metric-based trigger as-is
```

### Why defensive triggers override CapabilityGain

Consider: a new plugin is activated but causes safety score to drop below
`theta_min`.

```
Gen N:   plugins = [A, B],    safety = 0.8
Gen N+1: plugins = [A, B, C], safety = 0.3   (theta_min = 0.5)
```

If CapabilityGain took priority, the generation would be recorded as
"capability gained" and plugin C would remain active. With SafetyBreach
taking priority, the engine can roll back to Gen N, removing plugin C.

### Why CapabilityGain overrides other growth triggers

When a new plugin is activated and fitness improves simultaneously, recording
the trigger as "Evolution" loses information. "CapabilityGain" explains
*why* the improvement occurred, making generation history more useful for
debugging and analysis.

---

## Event Emission

`EvolutionCapability` events are emitted **independently of the final trigger**.
This means capability changes are always observable in the event stream, even
when a defensive trigger takes priority.

```
Case 1: CapabilityGain trigger
  → EvolutionCapability × N  (one per new capability)
  → EvolutionGeneration × 1  (generation transition)

Case 2: SafetyBreach trigger + capability changes present
  → EvolutionCapability × N  (informational: capabilities were gained)
  → EvolutionBreach × 1      (generation transition / rollback)
```

This separation reflects the principle that **triggers label generation
transitions** while **events record what happened**. These are orthogonal
concerns.

---

## Data Flow

```
                    PluginRegistry
                         │
                         │ resolve plugin_id → capabilities
                         ▼
  prev_snapshot ──→ detect_capability_gain() ──→ Vec<CapabilityChange>
  curr_snapshot ──┘                                     │
                                                        │
  scores ─────────→ check_triggers() ──→ Option<Trigger>│
  prev_scores ───┘                            │         │
  params ────────┘                            ▼         ▼
                                       resolve_trigger()
                                              │
                                    ┌─────────┴─────────┐
                                    ▼                   ▼
                             final_trigger     EvolutionCapability
                                    │              events
                                    ▼
                            handle_*() methods
                                    │
                                    ▼
                          GenerationRecord
                          (trigger = CapabilityGain)
```

---

## Future Considerations

- **Tool-level detection (Layer 1)**: Currently excluded to avoid noise. If a
  finer-grained trigger is needed, a separate `ToolAcquisition` trigger could
  be introduced without modifying CapabilityGain.
- **Capability loss detection**: The inverse case (plugin removal) is not
  covered. This could be handled by Regression (fitness drops after removal)
  or a future `CapabilityLoss` trigger.
- **External trigger injection**: Systems outside `evaluate()` (e.g. plugin
  manager) could emit CapabilityGain directly. The current design allows this
  since `EvolutionCapability` events are independent of the trigger resolution.
