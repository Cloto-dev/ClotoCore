import { VRM } from '@pixiv/three-vrm';

/**
 * Procedural blinking animation using VRM Expression blendshapes.
 * Random blink intervals with natural close/hold/open timing.
 */

type BlinkPhase = 'idle' | 'closing' | 'holding' | 'opening';

export class ProceduralBlinking {
  private phase: BlinkPhase = 'idle';
  private phaseTimer = 0;
  private nextBlinkIn: number;
  private blinkValue = 0;
  private doubleBlink = false;
  private doubleBlinkDone = false;

  // Timing constants (seconds)
  private closeTime = 0.1;
  private holdTime = 0.05;
  private openTime = 0.1;
  private doubleBinkProbability = 0.15;

  constructor() {
    this.nextBlinkIn = this.randomInterval(1.0);
  }

  private randomInterval(frequencyMult: number): number {
    const base = 2 + Math.random() * 4; // 2-6 seconds
    return base / Math.max(frequencyMult, 0.1);
  }

  update(vrm: VRM, deltaTime: number, blinkFrequency: number) {
    const expressionManager = vrm.expressionManager;
    if (!expressionManager) return;

    this.phaseTimer += deltaTime;

    switch (this.phase) {
      case 'idle':
        this.nextBlinkIn -= deltaTime;
        if (this.nextBlinkIn <= 0) {
          this.phase = 'closing';
          this.phaseTimer = 0;
          this.doubleBlink = Math.random() < this.doubleBinkProbability;
          this.doubleBlinkDone = false;
        }
        break;

      case 'closing': {
        const t = Math.min(this.phaseTimer / this.closeTime, 1);
        this.blinkValue = t;
        if (t >= 1) {
          this.phase = 'holding';
          this.phaseTimer = 0;
        }
        break;
      }

      case 'holding':
        this.blinkValue = 1;
        if (this.phaseTimer >= this.holdTime) {
          this.phase = 'opening';
          this.phaseTimer = 0;
        }
        break;

      case 'opening': {
        const t = Math.min(this.phaseTimer / this.openTime, 1);
        this.blinkValue = 1 - t;
        if (t >= 1) {
          if (this.doubleBlink && !this.doubleBlinkDone) {
            // Start second blink immediately
            this.doubleBlinkDone = true;
            this.phase = 'closing';
            this.phaseTimer = 0;
          } else {
            this.phase = 'idle';
            this.blinkValue = 0;
            this.nextBlinkIn = this.randomInterval(blinkFrequency);
          }
        }
        break;
      }
    }

    // Apply to VRM expression blendshapes (support both VRM 0.x and 1.0 naming)
    if (expressionManager.getExpression('blink')) {
      expressionManager.setValue('blink', this.blinkValue);
    } else {
      expressionManager.setValue('blinkLeft', this.blinkValue);
      expressionManager.setValue('blinkRight', this.blinkValue);
    }
  }
}
