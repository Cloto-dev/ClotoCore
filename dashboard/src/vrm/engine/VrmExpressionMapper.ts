import type { VRM } from '@pixiv/three-vrm';

/**
 * Maps ClotoCore standard expression/viseme names to expressions actually
 * available on the loaded VRM model.
 *
 * three-vrm v3 handles VRM 0.x → 1.0 name normalization (Joy→happy, A→aa, etc.)
 * automatically during load. This mapper handles the remaining compatibility gap:
 * custom models that use non-standard names or are missing standard expressions.
 */
export class VrmExpressionMapper {
  /** ClotoCore name → VRM model expression name (null = unavailable) */
  private expressionMap = new Map<string, string | null>();
  private visemeMap = new Map<string, string | null>();
  private blinkNames: string[] = [];

  /** Fallback chains: first match wins. */
  private static readonly EXPRESSION_FALLBACKS: Record<string, string[]> = {
    happy: ['happy', 'joy', 'smile'],
    angry: ['angry', 'anger'],
    sad: ['sad', 'sorrow', 'crying'],
    relaxed: ['relaxed', 'fun', 'calm'],
    surprised: ['surprised', 'surprise'],
    neutral: ['neutral'],
  };

  private static readonly VISEME_FALLBACKS: Record<string, string[]> = {
    aa: ['aa', 'a'],
    ih: ['ih', 'i'],
    ou: ['ou', 'u'],
    ee: ['ee', 'e'],
    oh: ['oh', 'o'],
  };

  /**
   * Probe the VRM's expressionManager and build mapping tables.
   * Call once after VRM load.
   */
  initialize(vrm: VRM): void {
    const mgr = vrm.expressionManager;
    if (!mgr) return;

    this.expressionMap.clear();
    this.visemeMap.clear();
    this.blinkNames = [];

    // Expressions
    for (const [standard, fallbacks] of Object.entries(VrmExpressionMapper.EXPRESSION_FALLBACKS)) {
      this.expressionMap.set(standard, this.firstAvailable(mgr, fallbacks));
    }

    // Visemes
    for (const [standard, fallbacks] of Object.entries(VrmExpressionMapper.VISEME_FALLBACKS)) {
      this.visemeMap.set(standard, this.firstAvailable(mgr, fallbacks));
    }

    // Blink
    if (mgr.getExpression('blink')) {
      this.blinkNames = ['blink'];
    } else if (mgr.getExpression('blinkLeft') && mgr.getExpression('blinkRight')) {
      this.blinkNames = ['blinkLeft', 'blinkRight'];
    }

    // Debug log
    const missing = [...this.expressionMap.entries()].filter(([, v]) => v === null).map(([k]) => k);
    if (missing.length > 0) {
      if (import.meta.env.DEV) console.warn('[VRM Mapper] Missing expressions:', missing.join(', '));
    }
    const missingV = [...this.visemeMap.entries()].filter(([, v]) => v === null).map(([k]) => k);
    if (missingV.length > 0) {
      if (import.meta.env.DEV) console.warn('[VRM Mapper] Missing visemes:', missingV.join(', '));
    }
  }

  /** Resolve a ClotoCore expression name to the model's actual name. */
  resolveExpression(name: string): string | null {
    return this.expressionMap.get(name) ?? null;
  }

  /** Resolve a ClotoCore viseme name to the model's actual name. */
  resolveViseme(name: string): string | null {
    return this.visemeMap.get(name) ?? null;
  }

  /** Blink expression name(s) available on this model. */
  getBlinkNames(): string[] {
    return this.blinkNames;
  }

  /** All resolved viseme names (for iteration/reset). */
  getResolvedVisemes(): string[] {
    return [...this.visemeMap.values()].filter((v): v is string => v !== null);
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private firstAvailable(mgr: any, candidates: string[]): string | null {
    for (const name of candidates) {
      if (mgr.getExpression(name)) return name;
    }
    return null;
  }
}
