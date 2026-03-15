import type { VRM } from '@pixiv/three-vrm';
import type { VrmExpressionMapper } from './VrmExpressionMapper';

export interface VisemeEntry {
  viseme: string; // "aa" | "ih" | "ou" | "ee" | "oh" | "neutral"
  start_ms: number;
  duration_ms: number;
}

export interface VisemeTimeline {
  entries: VisemeEntry[];
  total_duration_ms: number;
}

/** All VRM viseme expression names used for lip sync. */
const VISEME_EXPRESSIONS = ['aa', 'ih', 'ou', 'ee', 'oh'] as const;

/** Smoothstep interpolation. */
function smoothstep(edge0: number, edge1: number, x: number): number {
  const t = Math.max(0, Math.min(1, (x - edge0) / (edge1 - edge0)));
  return t * t * (3 - 2 * t);
}

/**
 * Plays a viseme timeline on a VRM model's expression manager.
 * Interpolates between visemes with smoothstep transitions.
 */
export class VisemePlayer {
  private timeline: VisemeEntry[] = [];
  private startTime = 0;
  private playing = false;
  /** When true, elapsed time is provided externally (audio-synced mode). */
  private externalClock = false;
  private externalTimeMs = 0;
  /** Transition duration in seconds for blending between visemes. */
  private transitionSec = 0.03; // 30ms
  private mapper: VrmExpressionMapper | null = null;

  /** Set the expression mapper for cross-model compatibility. */
  setMapper(mapper: VrmExpressionMapper) {
    this.mapper = mapper;
  }

  play(timeline: VisemeEntry[]) {
    this.timeline = timeline;
    this.startTime = performance.now();
    this.playing = true;
    this.externalClock = false;
  }

  /** Start playback in audio-synced mode (time provided via setExternalTime). */
  playSync(timeline: VisemeEntry[]) {
    this.timeline = timeline;
    this.playing = true;
    this.externalClock = true;
    this.externalTimeMs = 0;
  }

  /** Set the current time from an external source (e.g. AudioContext). */
  setExternalTime(ms: number) {
    this.externalTimeMs = ms;
  }

  stop() {
    this.playing = false;
    this.timeline = [];
    this.externalClock = false;
  }

  /** Replace the timeline mid-playback (Layer B correction). */
  updateTimeline(timeline: VisemeEntry[]) {
    this.timeline = timeline;
  }

  isPlaying(): boolean {
    return this.playing;
  }

  /** Returns true if playback is driven by an external audio clock. */
  isSynced(): boolean {
    return this.externalClock;
  }

  /** Get the active viseme names (mapped or raw fallback). */
  private get activeVisemes(): readonly string[] {
    if (this.mapper) return this.mapper.getResolvedVisemes();
    return VISEME_EXPRESSIONS;
  }

  /** Resolve a viseme name through the mapper. */
  private resolveViseme(name: string): string | null {
    if (this.mapper) return this.mapper.resolveViseme(name);
    return name;
  }

  /** Call every frame to apply current viseme weights to VRM expressions. */
  update(vrm: VRM, _deltaTime: number) {
    const expr = vrm.expressionManager;
    if (!expr || !this.playing || this.timeline.length === 0) {
      // Reset all viseme expressions when not playing
      if (expr && !this.playing) {
        for (const name of this.activeVisemes) {
          expr.setValue(name, 0);
        }
      }
      return;
    }

    const elapsedMs = this.externalClock ? this.externalTimeMs : performance.now() - this.startTime;

    // Find the current viseme entry
    let currentEntry: VisemeEntry | null = null;
    let nextEntry: VisemeEntry | null = null;

    for (let i = 0; i < this.timeline.length; i++) {
      const entry = this.timeline[i];
      const entryEnd = entry.start_ms + entry.duration_ms;

      if (elapsedMs >= entry.start_ms && elapsedMs < entryEnd) {
        currentEntry = entry;
        nextEntry = this.timeline[i + 1] ?? null;
        break;
      }
    }

    // Past the end of the timeline: stop
    if (!currentEntry) {
      const lastEntry = this.timeline[this.timeline.length - 1];
      if (elapsedMs >= lastEntry.start_ms + lastEntry.duration_ms) {
        this.playing = false;
        for (const name of this.activeVisemes) {
          expr.setValue(name, 0);
        }
        return;
      }
      // Before the first entry or in a gap: all zero
      for (const name of this.activeVisemes) {
        expr.setValue(name, 0);
      }
      return;
    }

    // Calculate weight for current viseme
    const entryEnd = currentEntry.start_ms + currentEntry.duration_ms;
    const transitionMs = this.transitionSec * 1000;

    // Fade in at start, fade out at end
    let weight = 1.0;
    if (elapsedMs < currentEntry.start_ms + transitionMs) {
      weight = smoothstep(currentEntry.start_ms, currentEntry.start_ms + transitionMs, elapsedMs);
    }
    if (nextEntry && elapsedMs > entryEnd - transitionMs) {
      weight = smoothstep(entryEnd, entryEnd - transitionMs, elapsedMs);
    }

    // Apply: set current viseme (mapped), zero others
    const resolvedCurrent = this.resolveViseme(currentEntry.viseme);
    for (const name of this.activeVisemes) {
      if (name === resolvedCurrent) {
        expr.setValue(name, weight);
      } else {
        expr.setValue(name, 0);
      }
    }
  }

  /** Reset all viseme expressions to zero. */
  reset(vrm: VRM) {
    const expr = vrm.expressionManager;
    if (!expr) return;
    for (const name of this.activeVisemes) {
      expr.setValue(name, 0);
    }
  }
}
