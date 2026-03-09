# VRM Avatar Motion Design

> Extends [PROJECT_VISION.md §9](PROJECT_VISION.md#9-layer-5-avatar-system-sapphy-v2)
> with detailed motion architecture, lip sync strategy, and implementation roadmap.

**Version:** 0.1.0-draft
**Status:** Draft
**Date:** 2026-03-09

---

## 1. Overview

ClotoCore's VRM avatar system renders a 3D character in the Tauri WebView using
three.js + `@pixiv/three-vrm`. The avatar must feel "alive" — breathing, blinking,
reacting to conversation context, and synchronizing lip movement with speech.

This document defines the **Layered Motion Architecture** that achieves natural
movement through three independent, additively composited motion layers, driven
by MGP (Model General Protocol) rather than embedded in the kernel.

### 1.1 Design Principles

1. **Additive Composition** — Each layer contributes deltas to bone transforms;
   layers blend naturally without explicit state machines
2. **Procedural First** — Prefer code-generated motion over baked animation clips;
   eliminates visible loop seams and enables parametric variation
3. **MGP-Driven Control** — Motion parameters flow through MGP tools/streams;
   the kernel relays events, never interprets VRM-specific logic
4. **Graceful Degradation** — Each layer is independently optional; the avatar
   remains functional with any subset of layers active

---

## 2. Layered Motion Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Layer 3: Gesture                                        │
│  Event/speech-driven discrete actions                    │
│  Nod, head tilt, hand wave                               │
│  D-3 Hybrid: procedural (head) + animation clips (body)  │
├─────────────────────────────────────────────────────────┤
│  Layer 2: Emotional Posture  ★ Research Required ★       │
│  Continuous mapping: emotion vector → body tendencies     │
│  cpersona v3 parameters → IK constraints + motion bias    │
│  MGP: set_idle_behavior({ emotion_params })               │
├─────────────────────────────────────────────────────────┤
│  Layer 1: Vitality (Always Active)                       │
│  Procedural: breathing, blinking, micro-sway, gaze drift │
│  Spring Bone physics (automatic via three-vrm)            │
│  AgentThinking/Responding SSE integration                 │
└─────────────────────────────────────────────────────────┘
```

Final bone transform = Layer 1 + Layer 2 + Layer 3 (additive blending)

### 2.1 Layer 1: Vitality (生命感)

Always-running procedural animation computed locally in the dashboard at 60fps.
No per-frame MGP communication required.

| Element | Technique | Parameters |
|---------|-----------|------------|
| **Breathing** | Spine/Chest bone rotation + scale (sin wave) | Period: 3-4s, amplitude: 0.5-1.0° |
| **Micro-sway** | Head/Spine micro-rotation (Perlin noise) | Amplitude: 0.3-0.8°, speed: 0.2-0.5Hz |
| **Blinking** | BlendShape `eyeBlinkLeft/Right` (random interval) | Interval: 2-6s, close duration: 0.1-0.15s |
| **Gaze drift** | Eye bone micro-movement (saccade simulation) | Interval: 0.5-2s, micro-jump |
| **Spring Bone** | Hair/clothing physics (automatic) | `vrm.update(deltaTime)` per frame |

#### AgentThinking Integration (Low-Cost, High-Impact)

Existing SSE events drive natural state transitions with no new MGP tools required:

| SSE Event | Avatar Behavior |
|-----------|-----------------|
| `AgentThinking` | Gaze drifts upward/sideways, subtle head tilt |
| `AgentResponding` | Gaze returns to user, slight forward lean |
| `AgentIdle` | Return to normal idle state |

These transitions use smooth interpolation (lerp/slerp, ~0.3-0.5s).

#### MGP Control

Parameters are dynamically adjustable via MGP:

```json
{
  "name": "set_idle_behavior",
  "arguments": {
    "mode": "relaxed",
    "breathing_rate": 0.3,
    "sway_amplitude": 1.0,
    "blink_frequency": 1.0
  }
}
```

The dashboard updates its procedural generation parameters; subsequent frames
use the new values. No per-frame MGP messages are sent.

### 2.2 Layer 2: Emotional Posture (感情姿勢) ★ Research Area ★

Maps cpersona v3 emotion vectors to continuous body parameter adjustments.
This is neither discrete animation switching (predictable) nor full motion
generation (too heavy for real-time) — it is a novel intermediate approach.

#### Mapping Function

```
cpersona v3 emotion vector:
  { joy: 0.8, sadness: 0.0, anger: 0.1, surprise: 0.2, ... }
                    │
                    ▼
          f(emotion) → body_params
                    │
                    ▼
  Body Parameters:
  {
    center_of_gravity:  +0.02    // vertical offset (joy → slightly up)
    sway_amplitude:      1.4     // sway scale (joy → larger)
    sway_speed:          1.2     // sway speed (joy → slightly faster)
    breathing_depth:     1.1     // breath depth (joy → slightly deeper)
    shoulder_tension:   -0.3     // shoulder tension (joy → relaxed)
    head_tilt_bias:     +2°      // head tilt bias
    blink_frequency:     0.8     // blink rate scale
    motion_range:        1.3     // overall motion range scale
  }
                    │
                    ▼
  Perlin noise × body_params → per-frame bone displacement
  → Same joy=0.8 produces subtly different motion each time
```

#### Research Approaches

| Approach | Method | Timeline |
|----------|--------|----------|
| **Hand-designed** | Per-emotion-dimension parameter curves, manually tuned | v0.7.x initial |
| **Learning-based** | Regression from mocap/animation data → parameter mapping | v1.x+ evolution |

Hand-designed approach is preferred initially for controllability and low compute
overhead. Learning-based refinement can follow once training data is available.

#### Industry Context

Three existing approaches in the field, none fully solving the problem:

1. **Preset animation switching** (Neuro-Sama, most AI vtubers) — finite
   variation, presence breaks when patterns become predictable
2. **Procedural animation** (noise/physics-based) — good idle presence,
   weak emotion/speech coupling
3. **Generative model motion** (MotionDiffuse, MDM) — too heavy for real-time
   inference currently

ClotoCore's layered approach combines all three: procedural base (Layer 1),
continuous emotion mapping (Layer 2), and hybrid gestures (Layer 3). The
"invention" lies in Layer 2's continuous emotion-to-body mapping.

### 2.3 Layer 3: Gesture (ジェスチャー)

Discrete actions triggered by events, speech, or agent decisions.
Uses D-3 Hybrid approach: procedural for head/neck, animation clips for arms/body.

#### Procedural Gestures (Initial Scope)

| Gesture | Trigger | Technique |
|---------|---------|-----------|
| **Nod** | Affirmation in response | Head bone X-rotation, single sin cycle |
| **Head tilt** | Question, curiosity | Head bone Z-rotation, hold + return |
| **Lean forward** | Interest, emphasis | Spine bone X-rotation, gradual |

#### Animation Clip Gestures (Future Extension)

| Gesture | Format | Source |
|---------|--------|--------|
| Hand wave | `.vrma` (VRM Animation) | Pre-authored per model |
| Pointing | `.vrma` | Pre-authored per model |
| Shrug | `.vrma` | Pre-authored per model |

Clips are model-specific (bone structure dependent) and loaded from
`data/avatar/animations/` alongside the VRM file.

#### MGP Control

```json
{
  "name": "play_gesture",
  "arguments": {
    "gesture": "nod",
    "intensity": 0.7,
    "duration_ms": 400
  }
}
```

---

## 3. Expression System

### 3.1 Dual-Mode with Fallback

VRM avatar-specific BlendShapes take priority when available;
VRM preset expressions serve as universal fallback.

```
Expression request: { emotion: "happy", intensity: 0.8 }
         │
         ├─ Avatar has ARKit BlendShapes?
         │   YES → Map to individual BlendShapes:
         │          mouthSmileLeft: 0.8, mouthSmileRight: 0.8,
         │          cheekSquintLeft: 0.4, eyeSquintLeft: 0.3, ...
         │
         └─ NO  → Use VRM preset:
                   vrm.expressionManager.setValue("happy", 0.8)
```

### 3.2 ARKit BlendShape Groups (52 Shapes)

| Region | BlendShapes | Count |
|--------|-------------|-------|
| Brow | browDownLeft/Right, browInnerUp, browOuterUpLeft/Right | 5 |
| Eye | eyeBlinkLeft/Right, eyeSquintLeft/Right, eyeWideLeft/Right | 6 |
| Cheek | cheekPuff, cheekSquintLeft/Right | 3 |
| Nose | noseSneerLeft/Right | 2 |
| Mouth | mouthSmileLeft/Right, mouthFrownLeft/Right, jawOpen, ... | ~28 |
| Jaw | jawForward, jawLeft, jawRight, jawOpen | 4 |
| Tongue | tongueOut | 1 |
| Other | eyeLookDown/Up/In/OutLeft/Right | ~3 |

### 3.3 VRM Preset Expressions (Fallback)

| Preset | Description |
|--------|-------------|
| `happy` | Joy, smile |
| `angry` | Anger, frown |
| `sad` | Sadness, downcast |
| `relaxed` | Calm, peaceful |
| `surprised` | Surprise, wide eyes |
| `neutral` | Default resting face |

---

## 4. Lip Sync

### 4.1 Viseme-Based Approach

Phonemes from TTS output are mapped to mouth shapes (visemes) applied via
BlendShapes.

#### Supported Viseme Sets

| Language | Visemes | BlendShapes Used |
|----------|---------|-----------------|
| Japanese | あ, い, う, え, お + closed | 6 basic mouth shapes |
| English (OVR-15) | aa, CH, DD, E, FF, ih, kk, nn, oh, ou, PP, RR, sil, SS, TH | 15 viseme shapes |

Sapphy V2 supports 15 OVR visemes, covering both Japanese and English.

### 4.2 Data Flow via MGP Streaming

Lip sync uses MGP §12 streaming (`notifications/mgp.stream.chunk`) for
real-time phoneme delivery. The TTS MCP server generates a phoneme timeline
alongside audio synthesis.

```
voice.tts MCP (Rust)
  │
  ├─ Audio data → sent to dashboard for playback
  │
  └─ Phoneme stream via mgp.stream.chunk:
     { index: 0, content: { viseme: "aa", start_ms: 0,   duration_ms: 120 } }
     { index: 1, content: { viseme: "ih", start_ms: 120, duration_ms: 80  } }
     { index: 2, content: { viseme: "sil", start_ms: 200, duration_ms: 50 } }
     ...
         │
         ▼
     Kernel (relay only) → SSE → Dashboard
         │
         ▼
     Dashboard: schedule viseme application synchronized to audio clock
```

### 4.3 Pre-Computed vs Real-Time

| | Pre-Computed | Real-Time Streaming |
|--|-------------|---------------------|
| Flow | TTS generates full phoneme timeline + audio → Dashboard syncs both | TTS streams phonemes as chunks → Dashboard applies progressively |
| Sync accuracy | High (timestamp-based) | Medium (network jitter dependent) |
| First-response latency | Higher (waits for full generation) | Lower (starts immediately) |
| Best for | Short utterances | Long utterances, streaming TTS |

**Decision**: Use **pre-computed for short utterances** (< 2 seconds) and
**streaming for longer speech**, selected automatically by the TTS MCP server.

### 4.4 Dashboard-Side Synchronization

The dashboard maintains a viseme schedule buffer:

1. Receive phoneme timeline (pre-computed or streamed)
2. Start audio playback
3. On each animation frame: check `audioContext.currentTime` against schedule
4. Apply current viseme via BlendShape with smoothstep interpolation
5. Cross-fade between consecutive visemes (~30ms overlap)

---

## 5. Look At (視線・頭部追跡)

### 5.1 Three-Tier Fallback

```
Priority 1: vision.gaze_webcam MCP → user face coordinates (30fps)
Priority 2: Mouse cursor position → screen gaze point
Priority 3: Virtual target → random / fixed near-camera target
```

Each tier activates when the higher-priority source is unavailable.

### 5.2 VRM LookAt Modes

| Mode | Control Target | Characteristics |
|------|---------------|-----------------|
| **Bone** | Eye bones + Head bone rotation | Natural, model-dependent |
| **Expression** | BlendShapes (lookLeft, lookRight, etc.) | VRM standard, high compatibility |

Use Bone mode when available; fall back to Expression mode.

### 5.3 Interpolation

Raw input data (30fps from webcam, variable from mouse) is interpolated
to 60fps rendering:

```
MGP input: { x: 0.3, y: -0.1 } at 30fps
Dashboard: current → target via smoothstep interpolation at 60fps
```

| Body Part | Response Time | Range Limit |
|-----------|--------------|-------------|
| **Eyes** | ~0.05s (fast follow) | ±15° horizontal, ±10° vertical |
| **Head** | ~0.3-0.5s (delayed follow) | ±30° horizontal, ±20° vertical |

Head follows eyes with natural delay, creating realistic gaze behavior.

---

## 6. MGP Integration

### 6.1 avatar.vrm MGP Server (Rust)

Implemented as a Rust MGP server for minimal latency (< 0.5ms per message).

#### Tool Definitions

| Tool | Layer | Input | Purpose |
|------|-------|-------|---------|
| `set_expression` | Expression | `{ emotion, intensity, blend_shapes? }` | Set facial expression |
| `set_mouth_shape` | Lip Sync | `{ viseme, intensity }` | Single viseme (fallback) |
| `set_gaze` | Look At | `{ x, y, source? }` | Set gaze target |
| `set_idle_behavior` | Layer 1+2 | `{ mode, breathing_rate, sway_amplitude, emotion_params? }` | Adjust idle parameters |
| `play_gesture` | Layer 3 | `{ gesture, intensity, duration_ms }` | Trigger gesture |
| `play_animation` | Layer 3 | `{ name, loop?, speed? }` | Play animation clip |

#### Streaming (MGP §12)

| Stream | Source | Content |
|--------|--------|---------|
| Phoneme stream | voice.tts → Kernel → SSE | `{ viseme, start_ms, duration_ms }` per chunk |
| Emotion stream | persona.emotion → Kernel → SSE | `{ joy, sadness, anger, ... }` periodic updates |

### 6.2 Kernel Role

The kernel acts as a **relay only** — it does not interpret or transform
VRM-specific data. Events flow through the existing SSE infrastructure:

```
MGP Server ──stdio──→ Kernel ──SSE──→ Dashboard (three.js renderer)
```

---

## 7. Implementation Roadmap

| Version | Scope | Deliverables |
|---------|-------|-------------|
| **v0.6.2** | Layer 1: Vitality | Procedural breathing, blinking, micro-sway, gaze drift; Spring Bone physics; Look At 3-tier fallback; AgentThinking/Responding SSE integration; Basic VRM loading + WebGL rendering |
| **v0.6.x** | Layer 3 Basics | Procedural gestures (nod, head tilt); Lip sync MVP (pre-computed viseme timeline) |
| **v0.7.x** | Layer 2: Emotional Posture | Hand-designed emotion→body parameter mapping; cpersona v3 integration; MGP streaming for phoneme delivery |
| **v0.8.x** | Layer 3 Expansion | Animation clip support (.vrma); Swappable VRM model support; Live2D MCP server (avatar.live2d) |
| **v1.x+** | Layer 2 Evolution | Learning-based emotion→body mapping; Community avatar MCP servers |

---

## 8. Technical Dependencies

| Component | Library / Technology | Role |
|-----------|---------------------|------|
| VRM loading | `@pixiv/three-vrm` | VRM parsing, BlendShape, SpringBone, LookAt |
| 3D rendering | `three.js` | WebGL scene, camera, lighting |
| Noise generation | Custom or `simplex-noise` | Perlin/Simplex noise for procedural motion |
| MGP server | Rust (`cloto_shared`, `serde`, `tokio`) | avatar.vrm MGP server |
| TTS integration | `voice.tts` MCP (existing) | Phoneme timeline generation |
| Emotion engine | `persona.emotion` / cpersona v3 | Emotion vector source |
| Gaze tracking | `vision.gaze_webcam` MCP (existing) | User face coordinates |

---

*Document created: 2026-03-09*
*Last updated: 2026-03-09*
