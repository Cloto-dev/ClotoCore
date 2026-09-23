import { describe, expect, it } from 'vitest';
import type { ModuleInfo } from '../../types';
import { pagesOf, sidebarModules } from '../panelPages';

const cil = (panel: string, position: number, extra: Partial<ModuleInfo> = {}): ModuleInfo => ({
  id: `cil-${panel}`,
  name: panel,
  connector: { id: 'cil', name: 'CIL Console', position },
  ...extra,
});

// The listing arrives sorted by id; the declared order here is the reverse, so
// anything that read the order off the list would put the pages backwards.
const ZETA_FIRST = [cil('alpha', 1), cil('zeta', 0)];

describe('pagesOf', () => {
  it('orders a connector’s panels by where they stand in its declaration, not by id', () => {
    const paged = pagesOf(ZETA_FIRST, 'cil-alpha');
    expect(paged?.pages.map((p) => p.id)).toEqual(['cil-zeta', 'cil-alpha']);
    expect(paged?.index).toBe(1);
    expect(paged?.group.name).toBe('CIL Console');
  });

  it('is no view of pages for a single panel, a placed module, or an unknown id', () => {
    expect(pagesOf([cil('only', 0)], 'cil-only')).toBeNull();
    expect(pagesOf([{ id: 'notes', name: 'Notes' }], 'notes')).toBeNull();
    expect(pagesOf(ZETA_FIRST, 'missing')).toBeNull();
  });

  it('does not count a rejected panel as a page', () => {
    const modules = [cil('good', 0), cil('bad', 1, { error: 'invalid' })];
    expect(pagesOf(modules, 'cil-good')).toBeNull();
    expect(pagesOf(modules, 'cil-bad')).toBeNull();
  });

  it('keeps two connectors’ pages apart', () => {
    const modules = [...ZETA_FIRST, { ...cil('x', 0), connector: { id: 'other', name: 'Other', position: 0 } }];
    expect(pagesOf(modules, 'cil-zeta')?.pages).toHaveLength(2);
  });
});

describe('sidebarModules', () => {
  it('lists a connector of several panels once, leading to its first page', () => {
    const entries = sidebarModules([{ id: 'a-notes', name: 'Notes' }, ...ZETA_FIRST]);
    expect(entries.map((e) => [e.label, e.id])).toEqual([
      ['Notes', 'a-notes'],
      ['CIL Console', 'cil-zeta'],
    ]);
    expect(entries[1].ids).toEqual(['cil-zeta', 'cil-alpha']);
  });

  it('lists a single-panel connector under the panel’s own name, and leaves rejected rows out', () => {
    const entries = sidebarModules([cil('only', 0, { name: 'Only' }), { id: 'broken', error: 'x' }]);
    expect(entries).toEqual([{ key: 'cil-only', label: 'Only', id: 'cil-only', ids: ['cil-only'] }]);
  });
});
