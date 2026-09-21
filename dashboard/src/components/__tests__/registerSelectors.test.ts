/**
 * No surface reaches into another through a bare class name.
 *
 * `ChatRoom.css`, `Workshop.css` and `AppSidebar.css` each draw one surface,
 * and all three are loaded together on every screen. A rule written as a bare
 * `.x` in one of them therefore reaches the others as well, and it does so
 * silently: at equal specificity neither rule is overridden, so an element ends
 * up with the union of the two, and whichever properties only one side names
 * apply unopposed.
 *
 * That has three times drawn something visibly wrong. `tool` names both a
 * workshop list row and the chat header's icon button, and the row's 200px
 * first column and padding reached the 32px button, pushing each icon about
 * 100px out of its own box. `hint` names both the note under the composer and
 * the note under a workshop field, and each face was drawn with the other's box
 * metrics. `more` names the sidebar's button for older conversations, the icon
 * that opens a server's menu, and the count at the end of a palette group; the
 * sidebar's `6px 10px` left the workshop's 28x24 box an 8x12 content box for a
 * 14px icon.
 *
 * None of them was caught by a rendering test, because nothing renders every
 * surface at once. This checks the cause instead of the symptom: a class
 * defined bare in one file must not be the thing another file's rules select.
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
 * Classes this file's rules actually select — every class in the rightmost
 * compound of each selector, which is what decides whether a rule reaches a
 * given element. `.face .tool` keys on `tool`; `.hint:not(.lead)` keys on
 * `hint`, not `lead`, because a `:not()` argument is not what is selected.
 *
 * Every class, not the first: `button.danger` selects on `danger` even though
 * the compound opens with an element name, and `.hint.lead` selects on both.
 * Reading only a leading class missed the sidebar's `button.danger` entirely,
 * so a bare `danger` in another file went unreported.
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
    for (const m of bare.matchAll(/\.([a-zA-Z][\w-]*)/g)) found.add(m[1]);
  }
  return found;
}

/** Names defined bare in `a` that rules in `b` also select. */
function reach(a: string, b: string): string[] {
  const keys = keyClasses(b);
  return [...bareClasses(a)].filter((n) => keys.has(n)).sort();
}

/** One per stylesheet that draws a surface of its own. */
const CHAT = readFileSync('src/components/ChatRoom.css', 'utf8');
const SHOP = readFileSync('src/components/Workshop.css', 'utf8');
const SIDE = readFileSync('src/components/AppSidebar.css', 'utf8');

describe('the collision detector', () => {
  // A detector that has never returned a name is not evidence of anything, so
  // these are the shapes it has to catch, each taken from a defect above and
  // reduced to the rules that mattered.
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

  // This shape survived the first version of the detector: it read only a class
  // at the head of the last compound, so a rule written against an element plus
  // a class was invisible and the bare name opposite it went unreported.
  it('sees a bare name against a compound that opens with an element', () => {
    expect(reach('.danger { color: red }', '.conv-menu button.danger { color: red }')).toEqual(['danger']);
  });

  it('reads every class in the last compound, not just the first', () => {
    expect(reach('.lead { color: red }', '.hint.lead { color: blue }')).toEqual(['lead']);
  });
});

// Every ordered pair, because the relation is not symmetric: a name left bare
// in one file is a problem only against a file that also selects it. Written
// out rather than generated in a loop — the repository's test count is read
// statically from the sources, and a loop would report one case while the
// runner reports six.
describe('the surfaces', () => {
  it('does not let a rule in the chat reach the workshop', () => {
    expect(reach(CHAT, SHOP)).toEqual([]);
  });

  it('does not let a rule in the chat reach the sidebar', () => {
    expect(reach(CHAT, SIDE)).toEqual([]);
  });

  it('does not let a rule in the workshop reach the chat', () => {
    expect(reach(SHOP, CHAT)).toEqual([]);
  });

  it('does not let a rule in the workshop reach the sidebar', () => {
    expect(reach(SHOP, SIDE)).toEqual([]);
  });

  it('does not let a rule in the sidebar reach the chat', () => {
    expect(reach(SIDE, CHAT)).toEqual([]);
  });

  it('does not let a rule in the sidebar reach the workshop', () => {
    expect(reach(SIDE, SHOP)).toEqual([]);
  });
});
