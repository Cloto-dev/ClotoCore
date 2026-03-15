import { describe, expect, it } from 'vitest';
import { displayServerId } from '../format';

describe('displayServerId', () => {
  it('strips namespace prefix', () => {
    expect(displayServerId('mind.cerebras')).toBe('cerebras');
    expect(displayServerId('memory.cpersona')).toBe('cpersona');
    expect(displayServerId('tool.terminal')).toBe('terminal');
  });

  it('returns full id when no namespace', () => {
    expect(displayServerId('standalone')).toBe('standalone');
  });

  it('handles multiple dots by stripping only first prefix', () => {
    expect(displayServerId('a.b.c')).toBe('b.c');
  });

  it('handles empty string', () => {
    expect(displayServerId('')).toBe('');
  });
});
