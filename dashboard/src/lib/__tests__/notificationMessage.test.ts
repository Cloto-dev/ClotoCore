import { describe, expect, it } from 'vitest';
import { keyedMessage, notificationText } from '../notificationMessage';

/**
 * A stand-in for i18next's `t` that knows exactly one pack.
 *
 * Written rather than reused so the fallback can be tested for what it is: a
 * key the pack does not have has to come back as the English the kernel sent,
 * and only a translator that really misses keys can show that.
 */
function translator(pack: Record<string, string>) {
  return (key: string, options: Record<string, unknown>) => {
    const line = pack[key];
    if (line === undefined) return String(options.defaultValue ?? key);
    return line.replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(options[name] ?? ''));
  };
}

const JA = {
  'notifications.messages.command.awaiting_approval.title': '{{n}} 件のコマンドが承認待ちです',
  'notifications.messages.hub_access.expires_soon.title': 'hub アクセストークンの期限が近づいています',
  'notifications.messages.hub_access.expires_soon.body': '{{date}} に期限切れになります。対象 {{n}} 件。',
};

function item(over: Partial<Parameters<typeof notificationText>[0]> = {}) {
  return {
    title: 'English title',
    body: 'English body',
    metadata: null as Record<string, unknown> | null,
    ...over,
  };
}

describe('notificationText', () => {
  it('says a keyed notice in the reader’s language, filling its values', () => {
    const said = notificationText(
      item({
        title: '2 command(s) awaiting approval',
        body: null,
        metadata: { message: { key: 'command.awaiting_approval', params: { n: 2 } } },
      }),
      translator(JA),
      'ja',
    );
    expect(said.title).toBe('2 件のコマンドが承認待ちです');
  });

  it('writes the date the reader’s way, from the kernel’s RFC 3339', () => {
    const said = notificationText(
      item({
        metadata: {
          message: {
            key: 'hub_access.expires_soon',
            params: { expires_at: '2026-12-19T00:00:00+00:00', n: 1 },
          },
        },
      }),
      translator(JA),
      'ja',
    );
    // The point is that the kernel's `%Y-%m-%d` is not what is shown; the exact
    // spelling belongs to Intl and differs by platform data, so this asserts
    // that it was reformatted rather than pinning one rendering.
    expect(said.body).toContain('件');
    expect(said.body).not.toContain('2026-12-19T00:00:00+00:00');
    expect(said.body).toMatch(/2026/);
  });

  // The fallback is the whole reason the English text is still sent. Delete
  // `defaultValue` from either call in notificationText and these go red.
  it('falls back to the kernel’s English when the pack lacks the key', () => {
    const said = notificationText(
      item({ metadata: { message: { key: 'kernel.shutdown', params: {} } } }),
      translator({}),
      'ja',
    );
    expect(said).toEqual({ title: 'English title', body: 'English body' });
  });

  it('falls back for the body alone when only the title is translated', () => {
    const said = notificationText(
      item({
        title: "'rm' was rejected",
        body: 'the sandbox refused it',
        metadata: { message: { key: 'tool.rejected', params: { tool: 'rm' } } },
      }),
      translator({ 'notifications.messages.tool.rejected.title': "'{{tool}}' は拒否されました" }),
      'ja',
    );
    expect(said.title).toBe("'rm' は拒否されました");
    expect(said.body).toBe('the sandbox refused it');
  });

  it('shows an unkeyed notice as the kernel wrote it', () => {
    const said = notificationText(item(), translator(JA), 'ja');
    expect(said).toEqual({ title: 'English title', body: 'English body' });
  });

  it('keeps a null body null rather than inventing one', () => {
    const said = notificationText(
      item({ body: null, metadata: { message: { key: 'kernel.shutdown', params: {} } } }),
      translator(JA),
      'ja',
    );
    expect(said.body).toBeNull();
  });
});

describe('keyedMessage', () => {
  // Metadata can come from an agent, so a shape that is not a key is not one.
  it.each([
    ['no metadata', null],
    ['no message', { link: '/settings' }],
    ['message is not an object', { message: 'kernel.shutdown' }],
    ['key is not a string', { message: { key: 7, params: {} } }],
    ['key is blank', { message: { key: '   ', params: {} } }],
  ])('treats %s as unkeyed', (_name, metadata) => {
    expect(keyedMessage(metadata as Record<string, unknown> | null)).toBeNull();
  });

  it('defaults params to empty when they are missing or malformed', () => {
    expect(keyedMessage({ message: { key: 'a.b' } })?.params).toEqual({});
    expect(keyedMessage({ message: { key: 'a.b', params: 'nope' } })?.params).toEqual({});
  });
});
