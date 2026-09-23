import { readFileSync } from 'node:fs';
import { render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { AgentMetadata } from '../../types';

vi.mock('../../hooks/useApi', () => ({ useApi: () => ({ getAvatarUrl: () => 'avatar.png' }) }));

import { AgentIcon, faceLetter } from '../agentIdentity';

describe('faceLetter', () => {
  it('takes the first character of the name, upper-cased', () => {
    expect(faceLetter('growth manager')).toBe('G');
    expect(faceLetter('Sapphy')).toBe('S');
  });

  it('keeps a script without case as it is', () => {
    expect(faceLetter('みずき')).toBe('み');
  });

  it('skips the spaces a name was typed with', () => {
    expect(faceLetter('  ks22')).toBe('K');
  });

  it('takes a whole character, never half of a surrogate pair', () => {
    // '𝒜' is two UTF-16 units; slicing one off would put a lone surrogate on the face.
    expect(faceLetter('𝒜gent')).toBe('𝒜');
  });

  it('has something to show for a name that is empty', () => {
    expect(faceLetter('')).toBe('?');
    expect(faceLetter('   ')).toBe('?');
  });
});

const agent = (metadata: Record<string, string> = {}): AgentMetadata =>
  ({ id: 'agent.growth-manager', name: 'Growth Manager', metadata }) as unknown as AgentMetadata;

describe('AgentIcon', () => {
  it('shows the first letter of the name on a face without a picture, and no glyph', () => {
    const { container } = render(<AgentIcon agent={agent()} size={20} />);
    const letter = container.querySelector('.face-letter');
    expect(letter?.textContent).toBe('G');
    expect(container.querySelector('svg')).toBeNull();
  });

  it('passes its size to the letter as a variable the face can override', () => {
    const { container } = render(<AgentIcon agent={agent()} size={34} />);
    const letter = container.querySelector<HTMLElement>('.face-letter');
    expect(letter?.style.getPropertyValue('--face-letter')).toBe('34px');
    // Not an inline font-size: that would outrank every face's own rule.
    expect(letter?.style.fontSize).toBe('');
  });

  it('shows the picture when the agent has one', () => {
    const { container } = render(<AgentIcon agent={agent({ has_avatar: 'true' })} size={40} />);
    expect(container.querySelector('img')).not.toBeNull();
    expect(container.querySelector('.face-letter')).toBeNull();
  });
});

// The face is drawn by four rules in two stylesheets. Each has to carry the flat colour first and
// the gradient after it: a webview without relative colours drops the gradient and keeps the flat
// one, and in the other order it would keep nothing of the gradient's intent either way.
const CHAT = readFileSync('src/components/ChatRoom.css', 'utf8');
const SHOP = readFileSync('src/components/Workshop.css', 'utf8');
const TOKENS = readFileSync('src/index.css', 'utf8');

function rule(css: string, selector: string): string {
  const at = css.indexOf(`\n${selector} {`);
  expect(at, `no rule for ${selector}`).toBeGreaterThanOrEqual(0);
  return css.slice(at, css.indexOf('\n}', at));
}

describe('the face stylesheet', () => {
  function drawsTheGradient(css: string, selector: string) {
    const body = rule(css, selector);
    const flat = body.indexOf('background: hsl(var(--agent));');
    const gradient = body.indexOf('background: linear-gradient(');
    expect(flat, `${selector} lost its flat fallback`).toBeGreaterThanOrEqual(0);
    expect(gradient, `${selector} has no gradient`).toBeGreaterThan(flat);
    // The end is worked out from the --agent in force where the face is drawn, turned and lifted
    // by the theme's numbers; a fixed second colour would give every agent the same end.
    expect(body).toContain('hsl(from hsl(var(--agent)) calc(h + var(--face-turn)) s');
    expect(body).toContain('calc(l + var(--face-lift))');
  }

  it('draws the chat header face flat, then as the gradient', () => drawsTheGradient(CHAT, '.face .pic'));
  it('draws the empty chat face flat, then as the gradient', () => drawsTheGradient(CHAT, '.presence .pic'));
  it('draws the selected roster face flat, then as the gradient', () => drawsTheGradient(SHOP, '.roster .r.on .f'));
  it('draws the settings face flat, then as the gradient', () => drawsTheGradient(SHOP, '.face-lg'));

  it('turns 68 degrees and lifts 8 by default', () => {
    expect(TOKENS).toMatch(/--face-turn: 68;/);
    expect(TOKENS).toMatch(/--face-lift: 8;/);
  });

  it('keeps the face flat under a theme with one accent for every agent', () => {
    expect(TOKENS).toMatch(/:root\[data-accent='fixed'\] \{\s*--face-turn: 0;\s*--face-lift: 0;/);
  });
});
