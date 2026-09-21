/**
 * The two registers do not share a bare class name.
 *
 * `ChatRoom.css` and `Workshop.css` each draw one of the two faces, and both
 * are loaded together on every screen. A rule written as a bare `.x` in one of
 * them therefore reaches the other face as well, and it does so silently: at
 * equal specificity neither rule is overridden, so each element ends up with
 * the union of the two, and whichever properties only one side names apply
 * unopposed.
 *
 * That has twice drawn something visibly wrong. `tool` names both a workshop
 * list row and the chat header's icon button, and the row's 200px first column
 * and padding reached the 32px button, pushing each icon about 100px out of its
 * own box. `hint` names both the note under the composer and the note under a
 * workshop field, and each face was drawn with the other's box metrics.
 *
 * Neither was caught by a rendering test, because nothing renders both faces at
 * once. This checks the cause instead of the symptom: a class defined bare in
 * one file must not be the thing the other file's rules select.
 */

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

/** Selector lists, with comments and at-rule preludes removed. */
function selectors(css: string): string[] {
  const out: string[] = [];
  const text = css.replace(/\/\*[\s\S]*?\*\//g, '');
  for (const m of text.matchAll(/([^{}]+)\{/g)) {
    const prelude = m[1].trim();
    if (!prelude || prelude.startsWith('@')) continue;
    for (const one of prelude.split(',')) {
      const s = one.trim();
      if (s) out.push(s);
    }
  }
  return out;
}

/** Classes this file styles with nothing but the class itself: `.x { … }`. */
function bareClasses(css: string): Set<string> {
  const found = new Set<string>();
  for (const s of selectors(css)) {
    const m = /^\.([a-zA-Z][\w-]*)$/.exec(s);
    if (m) found.add(m[1]);
  }
  return found;
}

/**
 * Classes this file's rules actually select — the rightmost compound of each
 * selector, which is what decides whether a rule reaches a given element.
 * `.face .tool` keys on `tool`; `.hint:not(.lead)` keys on `hint`, not `lead`.
 */
function keyClasses(css: string): Set<string> {
  const found = new Set<string>();
  for (const s of selectors(css)) {
    const last = s
      .split(/[\s>+~]+/)
      .filter(Boolean)
      .pop();
    if (!last) continue;
    const bare = last.replace(/:[a-zA-Z-]+\([^)]*\)/g, '');
    const m = /^\.([a-zA-Z][\w-]*)/.exec(bare);
    if (m) found.add(m[1]);
  }
  return found;
}

/** Names defined bare in `a` that rules in `b` also select. */
function reach(a: string, b: string): string[] {
  const keys = keyClasses(b);
  return [...bareClasses(a)].filter((n) => keys.has(n)).sort();
}

const CHAT = readFileSync('src/components/ChatRoom.css', 'utf8');
const SHOP = readFileSync('src/components/Workshop.css', 'utf8');

describe('the collision detector', () => {
  // A detector that has never returned a name is not evidence of anything, so
  // these are the two shapes it has to catch: the same name bare on both
  // sides, and bare on one side against a scoped rule on the other. Both are
  // taken from the defects described above, reduced to the rules that mattered.
  it('sees a name left bare on both sides', () => {
    expect(reach('.hint { font-size: 12.5px }', '.hint { font-size: 12px }')).toEqual(['hint']);
  });

  it('sees a bare name against a scoped rule on the other side', () => {
    expect(reach('.tool { padding: 7px 10px }', '.face .tool { width: 32px }')).toEqual(['tool']);
  });

  it('stays quiet once each side is scoped to its own face', () => {
    expect(reach('.ws .hint { font-size: 12.5px }', '.room .hint { font-size: 12px }')).toEqual([]);
  });

  it('does not mistake a name inside :not() for the one being selected', () => {
    expect(reach('.lead { color: red }', '.cli-list .hint:not(.lead) { color: blue }')).toEqual([]);
  });
});

describe('the chat and the workshop', () => {
  it('does not let a workshop rule reach the chat', () => {
    expect(reach(SHOP, CHAT)).toEqual([]);
  });

  it('does not let a chat rule reach the workshop', () => {
    expect(reach(CHAT, SHOP)).toEqual([]);
  });
});
