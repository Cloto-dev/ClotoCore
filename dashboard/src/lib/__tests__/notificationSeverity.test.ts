import { afterEach, describe, expect, it, vi } from 'vitest';
import type { NotificationSeverity } from '../../services/api';
import { type DisplayLevel, displayLevel, interrupts, loadThreshold, saveThreshold } from '../notificationSeverity';

/**
 * Every identifier the kernel can store. Listed rather than derived so that a
 * severity added to the union without a decision about which band it falls in
 * shows up here as a missing case rather than as a silent `undefined` rank.
 */
const ALL: NotificationSeverity[] = ['emergency', 'alert', 'critical', 'error', 'warning', 'notice', 'info', 'debug'];

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

describe('displayLevel', () => {
  it('gives every stored identifier a band', () => {
    for (const severity of ALL) {
      expect(['high', 'medium', 'low']).toContain(displayLevel(severity));
    }
  });

  it('puts the kernel-derived approval levels where a reader expects them', () => {
    // `error` is what the gate derives for a destructive or unclassified tool
    // call, and `warning` for a moderate one. If those two ever landed in the
    // same band the threshold would stop separating anything.
    expect(displayLevel('error')).toBe('high');
    expect(displayLevel('warning')).toBe('medium');
    expect(displayLevel('critical')).toBe('high');
    expect(displayLevel('info')).toBe('low');
  });
});

describe('interrupts', () => {
  it('lets the loudest through at every threshold', () => {
    for (const threshold of ['high', 'medium', 'low'] as DisplayLevel[]) {
      expect(interrupts('critical', threshold)).toBe(true);
    }
  });

  it('narrows as the threshold rises', () => {
    expect(interrupts('warning', 'low')).toBe(true);
    expect(interrupts('warning', 'medium')).toBe(true);
    expect(interrupts('warning', 'high')).toBe(false);

    expect(interrupts('info', 'low')).toBe(true);
    expect(interrupts('info', 'medium')).toBe(false);
  });
});

describe('loadThreshold', () => {
  it('defaults to medium', () => {
    expect(loadThreshold()).toBe('medium');
  });

  it('round-trips a choice', () => {
    saveThreshold('high');
    expect(loadThreshold()).toBe('high');
  });

  it('treats a value it does not recognise as absent', () => {
    // Browser storage can hold whatever a previous version or another tab left.
    // An unrecognised threshold compared against `undefined` is false for every
    // item, which would stop everything interrupting and say nothing about it.
    localStorage.setItem('cloto-notification-threshold', 'catastrophic');
    expect(loadThreshold()).toBe('medium');
  });

  it('still answers when storage throws', () => {
    vi.stubGlobal('localStorage', {
      getItem: () => {
        throw new Error('storage disabled');
      },
      setItem: () => {
        throw new Error('storage disabled');
      },
    });
    expect(loadThreshold()).toBe('medium');
    expect(() => saveThreshold('high')).not.toThrow();
  });
});
