import { Brain, Clock, Target, Users } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { type PillOption, PillSelect } from './ui/PillSelect';
import { SectionHeader } from './ui/SectionHeader';

// Metadata values mirror the kernel's `RecallPolicy::from_metadata` /
// `SessionScope::from_metadata` (crates/core/src/handlers/system.rs). Keep these
// strings in lockstep with the kernel — they are the on-wire contract.
export type RecallPolicyValue = 'always' | 'session_start+active' | 'session_start' | 'manual_only';
export type SessionScopeValue = 'per_user' | 'channel' | 'thread';
// knob 3 is owned by the memory server (set_recall_precision), not agent metadata.
export type PrecisionValue = 'strict' | 'balanced' | 'lenient';

export const RECALL_POLICY_DEFAULT: RecallPolicyValue = 'always';
export const SESSION_SCOPE_DEFAULT: SessionScopeValue = 'per_user';
export const PRECISION_DEFAULT: PrecisionValue = 'balanced';

const RECALL_POLICY_VALUES: RecallPolicyValue[] = ['always', 'session_start+active', 'session_start', 'manual_only'];
const SESSION_SCOPE_VALUES: SessionScopeValue[] = ['per_user', 'channel', 'thread'];
const PRECISION_VALUES: PrecisionValue[] = ['strict', 'balanced', 'lenient'];

/** Normalize an arbitrary metadata string to a known policy (unknown → default). */
export function normalizeRecallPolicy(raw: string | undefined): RecallPolicyValue {
  return RECALL_POLICY_VALUES.includes(raw as RecallPolicyValue) ? (raw as RecallPolicyValue) : RECALL_POLICY_DEFAULT;
}

/** Normalize an arbitrary metadata string to a known scope (unknown → default). */
export function normalizeSessionScope(raw: string | undefined): SessionScopeValue {
  return SESSION_SCOPE_VALUES.includes(raw as SessionScopeValue) ? (raw as SessionScopeValue) : SESSION_SCOPE_DEFAULT;
}

/**
 * Normalize a read-back precision level to one of the three pills (unknown → default).
 * The memory server may report 'custom' for a raw beta override set outside this UI; the
 * pill has no 'custom' option, so it falls back to the default rather than misrepresenting.
 */
export function normalizePrecision(raw: string | undefined): PrecisionValue {
  return PRECISION_VALUES.includes(raw as PrecisionValue) ? (raw as PrecisionValue) : PRECISION_DEFAULT;
}

/**
 * Apply the two recall knobs to an agent's metadata map for persistence
 * (mutates and returns it). A non-default selection writes the key; a default
 * selection deletes it, so absent == kernel default and the metadata stays
 * minimal. Mirrors the `preferred_memory` set/delete convention in
 * `AgentPluginWorkspace.handleSave`.
 */
export function applyRecallMetadata(
  metadata: Record<string, string>,
  recallPolicy: RecallPolicyValue,
  sessionScope: SessionScopeValue,
): Record<string, string> {
  if (recallPolicy !== RECALL_POLICY_DEFAULT) {
    metadata.recall_policy = recallPolicy;
  } else {
    delete metadata.recall_policy;
  }
  if (sessionScope !== SESSION_SCOPE_DEFAULT) {
    metadata.session_scope = sessionScope;
  } else {
    delete metadata.session_scope;
  }
  return metadata;
}

interface Props {
  recallPolicy: RecallPolicyValue;
  sessionScope: SessionScopeValue;
  precision: PrecisionValue;
  /** Whether the agent's memory server advertises set_recall_precision (feature-detected). */
  precisionSupported: boolean;
  onRecallPolicyChange: (v: RecallPolicyValue) => void;
  onSessionScopeChange: (v: SessionScopeValue) => void;
  onPrecisionChange: (v: PrecisionValue) => void;
}

export function RecallSection({
  recallPolicy,
  sessionScope,
  precision,
  precisionSupported,
  onRecallPolicyChange,
  onSessionScopeChange,
  onPrecisionChange,
}: Props) {
  const { t } = useTranslation('agents');

  const policyOptions: PillOption<RecallPolicyValue>[] = [
    { value: 'always', label: t('recall.timing_always'), hint: t('recall.timing_always_hint') },
    {
      value: 'session_start+active',
      label: t('recall.timing_session_active'),
      hint: t('recall.timing_session_active_hint'),
    },
    { value: 'session_start', label: t('recall.timing_session'), hint: t('recall.timing_session_hint') },
    { value: 'manual_only', label: t('recall.timing_manual'), hint: t('recall.timing_manual_hint') },
  ];

  const scopeOptions: PillOption<SessionScopeValue>[] = [
    { value: 'per_user', label: t('recall.scope_per_user'), hint: t('recall.scope_per_user_hint') },
    { value: 'channel', label: t('recall.scope_channel'), hint: t('recall.scope_channel_hint') },
    { value: 'thread', label: t('recall.scope_thread'), hint: t('recall.scope_thread_hint') },
  ];

  // knob 3 (precision) is owned by the memory server. The pill is enabled only
  // when the active memory server advertises set_recall_precision (feature-detected).
  const precisionOptions: PillOption<PrecisionValue>[] = [
    { value: 'strict', label: t('recall.precision_strict'), hint: t('recall.precision_strict_hint') },
    { value: 'balanced', label: t('recall.precision_balanced'), hint: t('recall.precision_balanced_hint') },
    { value: 'lenient', label: t('recall.precision_lenient'), hint: t('recall.precision_lenient_hint') },
  ];

  return (
    <section>
      <SectionHeader icon={Brain} title={t('recall.title')} />
      <div className="space-y-4">
        <Row label={t('recall.timing_label')} hint={t('recall.timing_hint')}>
          <PillSelect
            value={recallPolicy}
            options={policyOptions}
            onSelect={onRecallPolicyChange}
            icon={Clock}
            accented={recallPolicy !== RECALL_POLICY_DEFAULT}
          />
        </Row>

        <Row label={t('recall.scope_label')} hint={t('recall.scope_hint')}>
          <PillSelect
            value={sessionScope}
            options={scopeOptions}
            onSelect={onSessionScopeChange}
            icon={Users}
            accented={sessionScope !== SESSION_SCOPE_DEFAULT}
          />
        </Row>

        <Row
          label={t('recall.precision_label')}
          hint={precisionSupported ? t('recall.precision_hint') : t('recall.precision_unsupported')}
        >
          <PillSelect
            value={precision}
            options={precisionOptions}
            onSelect={onPrecisionChange}
            icon={Target}
            accented={precisionSupported && precision !== PRECISION_DEFAULT}
            disabled={!precisionSupported}
          />
        </Row>
      </div>
    </section>
  );
}

function Row({ label, hint, children }: { label: string; hint: string; children: React.ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="min-w-0">
        <div className="text-[10px] font-bold text-content-tertiary uppercase tracking-wider">{label}</div>
        <div className="text-[10px] text-content-tertiary/70 mt-0.5">{hint}</div>
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}
