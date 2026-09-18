import type { ReactNode } from 'react';
import './Ask.css';

export interface AskAction {
  label: string;
  onClick: () => void;
}

interface Props {
  /** Who is asking, when the surroundings do not already say. */
  who?: ReactNode;
  /** The question, in the agent's words. */
  question: ReactNode;
  /** What would be run, one line each, in the mono box. */
  command?: string[];
  /** The explanation, as a sentence. */
  why?: ReactNode;
  whyTone?: 'plain' | 'bad';
  /** The answers: one to go ahead, the rest as alternatives. */
  ok?: AskAction;
  alts?: AskAction[];
  actionsDisabled?: boolean;
  /** "影響: 小" — drawn at the row's right. */
  risk?: ReactNode;
  /** The deck's position indicator, beside the risk. */
  pager?: ReactNode;
  /** An act that is not an answer, shown on hover. */
  dismiss?: AskAction;
  first?: boolean;
  children?: ReactNode;
}

/**
 * A question from the agent (docs/gui/samples/02-chat-conversation.html): the
 * agent's colour on the left edge, the question in their words, the command in
 * a mono box, the consequence as a sentence, and the answers below.
 */
export function Ask({
  who,
  question,
  command,
  why,
  whyTone = 'plain',
  ok,
  alts = [],
  actionsDisabled = false,
  risk,
  pager,
  dismiss,
  first = false,
  children,
}: Props) {
  const hasActs = ok || alts.length > 0 || risk || pager;
  return (
    <div className={`ask${first ? ' first' : ''}`}>
      {dismiss && (
        <button type="button" className="dismiss" onClick={dismiss.onClick} aria-label={dismiss.label}>
          {dismiss.label}
        </button>
      )}
      {who && <div className="who">{who}</div>}
      <div className="q">{question}</div>
      {command && command.length > 0 && <div className="cmd">{command.join('\n')}</div>}
      {why && <div className={`why${whyTone === 'bad' ? ' bad' : ''}`}>{why}</div>}
      {children}
      {hasActs && (
        <div className="acts">
          {ok && (
            <button type="button" className="ok" onClick={ok.onClick} disabled={actionsDisabled}>
              {ok.label}
            </button>
          )}
          {alts.map((a) => (
            <button type="button" className="alt" key={a.label} onClick={a.onClick} disabled={actionsDisabled}>
              {a.label}
            </button>
          ))}
          {risk && <span className="risk">{risk}</span>}
          {pager && <span className="pager">{pager}</span>}
        </div>
      )}
    </div>
  );
}
