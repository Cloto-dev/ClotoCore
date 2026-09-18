/** How a panel's declared writes are put to the operator.
 *
 * A consent is only as good as the operator's reading of what they consent to,
 * and `POST /api/chat/agent.manager/messages` is not something to read. The
 * routes a panel is likely to ask for get a sentence; anything else is shown as
 * written, because a sentence guessed for a route this does not know would be
 * a sentence that could be wrong. */

export type WriteDescription =
  | { kind: 'send_messages'; agent: string }
  | { kind: 'start_conversations'; agent: string }
  | { kind: 'raw'; entry: string };

// HARDCODED(crates/core/src/lib.rs "/chat/{agent_id}/messages" and
// "/chat/{agent_id}/conversations"): the two chat routes the first panel
// declares; an unrecognised route falls back to the raw declaration.
const SEND = /^POST \/api\/chat\/([^/]+)\/messages$/;
const START = /^POST \/api\/chat\/([^/]+)\/conversations$/;

export function describeWrite(entry: string): WriteDescription {
  const send = SEND.exec(entry);
  if (send) return { kind: 'send_messages', agent: decodeSegment(send[1]) };
  const start = START.exec(entry);
  if (start) return { kind: 'start_conversations', agent: decodeSegment(start[1]) };
  return { kind: 'raw', entry };
}

function decodeSegment(segment: string): string {
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
}
