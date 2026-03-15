import { describe, expect, it } from 'vitest';
import { safeJsonParse, tryParseJsonObject } from '../json';

describe('safeJsonParse', () => {
  it('parses valid JSON', () => {
    expect(safeJsonParse('{"a":1}', {})).toEqual({ a: 1 });
    expect(safeJsonParse('[1,2,3]', [])).toEqual([1, 2, 3]);
    expect(safeJsonParse('"hello"', '')).toBe('hello');
  });

  it('returns fallback on invalid JSON', () => {
    expect(safeJsonParse('not json', 'fallback')).toBe('fallback');
    expect(safeJsonParse('{broken', null)).toBeNull();
  });
});

describe('tryParseJsonObject', () => {
  it('parses valid object JSON', () => {
    expect(tryParseJsonObject('{"key":"value"}')).toEqual({ key: 'value' });
  });

  it('returns null for non-object JSON', () => {
    expect(tryParseJsonObject('"string"')).toBeNull();
    expect(tryParseJsonObject('42')).toBeNull();
    expect(tryParseJsonObject('true')).toBeNull();
  });

  it('returns parsed array as object (arrays are objects in JS)', () => {
    expect(tryParseJsonObject('[1,2]')).toEqual([1, 2]);
  });

  it('returns null for invalid JSON', () => {
    expect(tryParseJsonObject('{broken')).toBeNull();
  });

  it('returns null for undefined/empty input', () => {
    expect(tryParseJsonObject(undefined)).toBeNull();
    expect(tryParseJsonObject('')).toBeNull();
  });
});
