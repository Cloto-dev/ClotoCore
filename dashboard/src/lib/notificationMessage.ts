/**
 * Saying a kernel-raised notice in the reader's language.
 *
 * The kernel writes `title` and `body` in English and cannot do otherwise — it
 * does not know who is reading. Alongside them it may carry
 * `metadata.message`: a language-pack key under the `notifications` namespace
 * and the values that fill it.
 *
 * The English text is never discarded. It is the fallback for a notice with no
 * key at all (raised by an older kernel, or by `mgp.operator.ask`, where the
 * words are the agent's own and are not ours to translate) and for a key the
 * reader's pack has not translated. So a notice can always be read, and the
 * worst case is the one we already had.
 */

/**
 * Where these strings live in the pack.
 *
 * The bell's own strings are already under `notifications.` in the `common`
 * namespace, so the sentences go beneath them rather than into a second thing
 * also called "notifications" — a namespace of that name beside this key would
 * differ only by `:` against `.` at every call site.
 */
const KEY_PREFIX = 'notifications.messages.';

/** What the kernel puts in `metadata.message`. */
interface KeyedMessage {
  key: string;
  params: Record<string, unknown>;
}

/**
 * The key and params, or `null` when this notice carries none.
 *
 * Metadata can come from an agent, so nothing here trusts its shape: a
 * `message` that is not an object with a non-empty string `key` is treated as
 * absent rather than rendered.
 */
export function keyedMessage(metadata: Record<string, unknown> | null): KeyedMessage | null {
  const message = metadata?.message;
  if (typeof message !== 'object' || message === null) return null;
  const { key, params } = message as { key?: unknown; params?: unknown };
  if (typeof key !== 'string' || key.trim() === '') return null;
  return {
    key,
    params: typeof params === 'object' && params !== null ? (params as Record<string, unknown>) : {},
  };
}

/**
 * `params`, with anything the reader's locale decides spelled for that locale.
 *
 * The kernel hands dates over in RFC 3339 precisely so it does not pick a
 * format for everyone; `date` is that value written the reader's way. An
 * unparseable value is passed through untouched rather than shown as
 * "Invalid Date".
 */
function localized(params: Record<string, unknown>, locale: string): Record<string, unknown> {
  const { expires_at: expiresAt, ...rest } = params;
  if (typeof expiresAt !== 'string') return params;
  const at = new Date(expiresAt);
  if (Number.isNaN(at.getTime())) return { ...rest, date: expiresAt };
  return { ...rest, date: new Intl.DateTimeFormat(locale, { dateStyle: 'long' }).format(at) };
}

/** How a notice reads: translated where we can, English where we cannot. */
export interface NotificationText {
  title: string;
  body: string | null;
}

/**
 * Resolve one notice's wording.
 *
 * `translate` is i18next's `t` bound to the `notifications` namespace; it is
 * passed in rather than imported so this stays a pure function the tests can
 * drive without standing up i18next.
 */
export function notificationText(
  item: { title: string; body: string | null; metadata: Record<string, unknown> | null },
  translate: (key: string, options: Record<string, unknown>) => string,
  locale: string,
): NotificationText {
  const message = keyedMessage(item.metadata);
  if (!message) return { title: item.title, body: item.body };
  const params = localized(message.params, locale);
  return {
    title: translate(`${KEY_PREFIX}${message.key}.title`, { ...params, defaultValue: item.title }),
    // A key may translate the title and leave the body alone — `tool.rejected`
    // does, because its body is the rejection's own reason and has no template.
    // `defaultValue` is what keeps that body readable instead of blank.
    body:
      item.body === null ? null : translate(`${KEY_PREFIX}${message.key}.body`, { ...params, defaultValue: item.body }),
  };
}
