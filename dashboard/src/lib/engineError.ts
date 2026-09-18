// HARDCODED(crates/core/src/handlers/system.rs::handle_message_impl): the kernel stores a
// turn the engine failed to produce as an ordinary reply, `format!("[Error] {}", e)`, with no
// other mark; this prefix is the only thing that tells it apart from something the agent said.
const ENGINE_ERROR_PREFIX = '[Error]';

/**
 * The engine's own words when this reply is a failed turn, or `null` when the
 * agent actually said it. The prefix is dropped: it is the kernel's marker, not
 * part of what went wrong.
 */
export function engineErrorOf(text: string): string | null {
  return text.startsWith(ENGINE_ERROR_PREFIX) ? text.slice(ENGINE_ERROR_PREFIX.length).trimStart() : null;
}
