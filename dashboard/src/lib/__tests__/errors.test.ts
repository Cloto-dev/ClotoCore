import { describe, expect, it } from 'vitest';
import { extractError } from '../errors';

describe('extractError', () => {
  it('extracts message from Error instances', () => {
    expect(extractError(new Error('something broke'), 'fallback')).toBe('something broke');
  });

  it('returns fallback for non-Error values', () => {
    expect(extractError('string error', 'fallback')).toBe('fallback');
    expect(extractError(42, 'fallback')).toBe('fallback');
    expect(extractError(null, 'fallback')).toBe('fallback');
    expect(extractError(undefined, 'fallback')).toBe('fallback');
  });

  it('returns fallback for plain objects', () => {
    expect(extractError({ message: 'not an Error' }, 'fallback')).toBe('fallback');
  });
});
