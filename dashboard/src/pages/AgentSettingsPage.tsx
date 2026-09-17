import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { DeleteAgentModal } from '../components/agents/DeleteAgentModal';
import { PowerToggleModal } from '../components/PowerToggleModal';
import {
  applyRecallMetadata,
  normalizePrecision,
  normalizeRecallPolicy,
  normalizeSessionScope,
  PRECISION_DEFAULT,
  type PrecisionValue,
  type RecallPolicyValue,
  type SessionScopeValue,
} from '../components/RecallSection';
import { Select } from '../components/ui/Select';
import { VrmThumbnailDialog } from '../components/VrmThumbnailDialog';
import '../components/Workshop.css';
import { useAgentContext } from '../contexts/AgentContext';
import type { RoutingRuleEntry } from '../hooks/useAgentCreation';
import { useApi } from '../hooks/useApi';
import { useMcpServers } from '../hooks/useMcpServers';
import {
  type AgentGrants,
  changedServers,
  deniedToolCount,
  grantsFor,
  grantsFromEntries,
  mergeAgentEntries,
  withServerGrant,
  withToolGrant,
} from '../lib/agentAccess';
import { exportAgent } from '../lib/agentExport';
import { AgentIcon, agentAccentTriplet, parseAccentTriplet } from '../lib/agentIdentity';
import { displayServerId } from '../lib/format';
import { isEngineServer, isMemoryServer } from '../lib/serverCategory';
import { extractVrmThumbnail } from '../lib/vrmThumbnail';
import type { AgentInstructionsReport, AgentMetadata, McpToolInfo } from '../types';

const DEFAULT_AGENT_ID = 'agent.cloto_default';
/** The largest avatar the kernel accepts, in bytes. */
const AVATAR_MAX_BYTES = 5 * 1024 * 1024;
/** How many servers the tool section shows before "show the rest". */
const SERVERS_SHOWN = 7;

type Section = 'basics' | 'engine' | 'memory' | 'appearance' | 'tools' | 'danger';
const SECTIONS: Section[] = ['basics', 'engine', 'memory', 'appearance', 'tools', 'danger'];

function isSection(v: string | null): v is Section {
  return v !== null && (SECTIONS as string[]).includes(v);
}

/** The accent as the person edits it: a CSS colour, not the stored triplet. */
function accentFieldValue(agent: AgentMetadata): string {
  const stored = parseAccentTriplet(agent.metadata?.accent);
  return stored ? `hsl(${agent.metadata?.accent})` : '';
}

/** `hsl(190 70% 58%)` → `190 70% 58%`. Anything else is left alone so the
 *  validator, not this, decides whether it can be stored. */
function toTriplet(field: string): string {
  const m = /^\s*hsl\(([^)]*)\)\s*$/i.exec(field);
  return (m ? m[1] : field).trim();
}

/**
 * One agent's settings (docs/gui/samples/08-agent-settings.html): six sections
 * on the left, rows of "item + explanation + control" on the right, and a save
 * bar below.
 *
 * Nothing on this page reaches the kernel until Save is pressed — the rule in
 * CLAUDE.md, "Agent Config Rules". The two exceptions both sit behind their own
 * confirm modal: deleting the agent and toggling its power.
 */
export function AgentSettingsPage() {
  const api = useApi();
  const { t } = useTranslation('agents');
  const { t: tc } = useTranslation('common');
  const navigate = useNavigate();
  const { id } = useParams<{ id: string }>();
  const [searchParams] = useSearchParams();
  const { agents, refetchAgents } = useAgentContext();
  const { servers } = useMcpServers();

  const agent = agents.find((a) => a.id === id) ?? null;
  const isDefault = agent?.id === DEFAULT_AGENT_ID;

  const [section, setSection] = useState<Section>('basics');
  const sectionRefs = useRef<Record<Section, HTMLHeadingElement | null>>({
    basics: null,
    engine: null,
    memory: null,
    appearance: null,
    tools: null,
    danger: null,
  });

  // ---- pending state (everything the person can edit) ----
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [currentPassword, setCurrentPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [removePassword, setRemovePassword] = useState(false);
  const [engineId, setEngineId] = useState('');
  const [routing, setRouting] = useState<RoutingRuleEntry[]>([]);
  const [memoryId, setMemoryId] = useState('');
  const [recallPolicy, setRecallPolicy] = useState<RecallPolicyValue>('always');
  const [sessionScope, setSessionScope] = useState<SessionScopeValue>('per_user');
  const [precision, setPrecision] = useState<PrecisionValue>(PRECISION_DEFAULT);
  const [precisionDirty, setPrecisionDirty] = useState(false);
  const [precisionSupported, setPrecisionSupported] = useState(false);
  const [accent, setAccent] = useState('');
  const [grants, setGrants] = useState<AgentGrants>({});
  const [loadedGrants, setLoadedGrants] = useState<AgentGrants>({});

  // Avatar / VRM, deferred exactly as the old workspace deferred them.
  const [hasAvatar, setHasAvatar] = useState(false);
  const [pendingAvatarFile, setPendingAvatarFile] = useState<File | null>(null);
  const [pendingAvatarDelete, setPendingAvatarDelete] = useState(false);
  const [avatarPreviewUrl, setAvatarPreviewUrl] = useState<string | null>(null);
  const [hasVrm, setHasVrm] = useState(false);
  const [pendingVrmFile, setPendingVrmFile] = useState<File | null>(null);
  const [pendingVrmDelete, setPendingVrmDelete] = useState(false);
  const [vrmThumbnailFile, setVrmThumbnailFile] = useState<File | null>(null);
  const [vrmThumbnailUrl, setVrmThumbnailUrl] = useState<string | null>(null);
  const [showVrmThumbnailDialog, setShowVrmThumbnailDialog] = useState(false);

  // ---- loaded, read-only ----
  const [instructions, setInstructions] = useState<AgentInstructionsReport | null>(null);
  const [memoryCount, setMemoryCount] = useState<number | null>(null);
  const [toolsByServer, setToolsByServer] = useState<Record<string, McpToolInfo[]>>({});
  const [openServers, setOpenServers] = useState<Set<string>>(new Set());
  const [allServers, setAllServers] = useState(false);

  const [saving, setSaving] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [powerOpen, setPowerOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);

  const engineServers = useMemo(() => servers.filter(isEngineServer), [servers]);
  const memoryServers = useMemo(() => servers.filter(isMemoryServer), [servers]);

  /** Put every field back to what the kernel last said. Makes no calls. */
  const reset = useCallback(
    (from: AgentMetadata, entries: AgentGrants) => {
      setName(from.name);
      setDescription(from.description);
      setCurrentPassword('');
      setNewPassword('');
      setRemovePassword(false);
      setEngineId(from.default_engine_id ?? '');
      setRouting(parseRouting(from.metadata?.engine_routing));
      setMemoryId(from.metadata?.preferred_memory ?? '');
      setRecallPolicy(normalizeRecallPolicy(from.metadata?.recall_policy));
      setSessionScope(normalizeSessionScope(from.metadata?.session_scope));
      setPrecisionDirty(false);
      setAccent(accentFieldValue(from));
      setGrants(entries);
      setHasAvatar(from.metadata?.has_avatar === 'true');
      setHasVrm(from.metadata?.has_vrm === 'true');
      setPendingAvatarFile(null);
      setPendingAvatarDelete(false);
      setPendingVrmFile(null);
      setPendingVrmDelete(false);
      if (avatarPreviewUrl) URL.revokeObjectURL(avatarPreviewUrl);
      setAvatarPreviewUrl(null);
    },
    [avatarPreviewUrl],
  );

  // Load: the agent's own row is already in context; its grants, its
  // always-loaded files and its memory count are not.
  const agentId = agent?.id;
  useEffect(() => {
    if (!agentId) return;
    let cancelled = false;
    void (async () => {
      const [access, files, memories] = await Promise.all([
        api
          .getAgentAccess(agentId)
          .then((r) => grantsFromEntries(r.entries, agentId))
          .catch(() => ({}) as AgentGrants),
        api.getAgentInstructionFiles(agentId).catch(() => null),
        api
          .getMemories(agentId)
          .then((r) => r.memories.length)
          .catch(() => null),
      ]);
      if (cancelled) return;
      setLoadedGrants(access);
      setGrants(access);
      setInstructions(files);
      setMemoryCount(memories);
    })();
    return () => {
      cancelled = true;
    };
  }, [api, agentId]);

  // The agent's own fields. Keyed on the id so switching agents refills the
  // form, and not on the object, which a refetch replaces on every poll.
  const loadedAgent = useRef<string | null>(null);
  useEffect(() => {
    if (!agent) return;
    if (loadedAgent.current === agent.id) return;
    loadedAgent.current = agent.id;
    reset(agent, {});
  }, [agent, reset]);

  // Whether the memory server can take a precision at all, and what this agent's
  // is when it can be read back (read-edit-save rather than write-only).
  useEffect(() => {
    if (!agentId) return;
    let cancelled = false;
    void api
      .getMemories()
      .then((res) => {
        if (cancelled) return;
        setPrecisionSupported(res.capabilities?.set_recall_precision ?? false);
        if (res.capabilities?.get_recall_precision) {
          void api
            .getRecallPrecision(agentId)
            .then((info) => {
              if (!cancelled) setPrecision(normalizePrecision(info.precision));
            })
            .catch(() => {
              /* leave it at the default; the control is still write-capable */
            });
        }
      })
      .catch(() => {
        /* no memory server answered: the control stays disabled */
      });
    return () => {
      cancelled = true;
    };
  }, [api, agentId]);

  // `?section=` opens the page at one section, which is how the roster's
  // "Tool permissions" button arrives.
  const wanted = searchParams.get('section');
  useEffect(() => {
    if (!isSection(wanted)) return;
    setSection(wanted);
    sectionRefs.current[wanted]?.scrollIntoView({ block: 'start' });
  }, [wanted]);

  if (!agent) {
    return <div className="ws empty">{tc('loading')}</div>;
  }

  const passwordSet = agent.metadata?.has_power_password === 'true';
  const passwordTouched = removePassword || newPassword.length > 0;

  const accentTriplet = toTriplet(accent);
  const accentValid = accent.trim() === '' || parseAccentTriplet(accentTriplet) !== null;
  const previewAccent = accentValid
    ? agentAccentTriplet({ id: agent.id, metadata: { ...agent.metadata, accent: accentTriplet } })
    : agentAccentTriplet(agent);

  const dirty =
    name !== agent.name ||
    description !== agent.description ||
    engineId !== (agent.default_engine_id ?? '') ||
    JSON.stringify(routing) !== JSON.stringify(parseRouting(agent.metadata?.engine_routing)) ||
    memoryId !== (agent.metadata?.preferred_memory ?? '') ||
    recallPolicy !== normalizeRecallPolicy(agent.metadata?.recall_policy) ||
    sessionScope !== normalizeSessionScope(agent.metadata?.session_scope) ||
    precisionDirty ||
    accent !== accentFieldValue(agent) ||
    passwordTouched ||
    pendingAvatarFile !== null ||
    pendingAvatarDelete ||
    pendingVrmFile !== null ||
    pendingVrmDelete ||
    changedServers(loadedGrants, grants).length > 0;

  const jumpTo = (s: Section) => {
    setSection(s);
    sectionRefs.current[s]?.scrollIntoView({ block: 'start', behavior: 'smooth' });
  };

  async function handleSave() {
    if (!agent || !dirty || saving) return;
    if (!accentValid) {
      setProblem(t('settings.accent_invalid'));
      return;
    }
    setSaving(true);
    setProblem(null);
    try {
      // The engine and the memory server are only usable if the agent may call
      // them, so choosing one grants it. Done before the diff, so the grant is
      // part of what gets written. Only where the answer is still "default":
      // a Deny written by hand on this same page is an instruction, and
      // silently flipping it to Allow would be the screen overruling it.
      let nextGrants = grants;
      if (engineId && grantsFor(nextGrants, engineId).server === 'default') {
        nextGrants = withServerGrant(nextGrants, engineId, 'allow');
      }
      if (memoryId && grantsFor(nextGrants, memoryId).server === 'default') {
        nextGrants = withServerGrant(nextGrants, memoryId, 'allow');
      }

      // The password goes first because it is the step most likely to be
      // refused (a mistyped current password). Refused first, nothing else has
      // been written; refused last, the page would report a failure over a
      // save that had mostly happened.
      if (passwordTouched) {
        await api.setAgentPowerPassword(
          agent.id,
          removePassword ? '' : newPassword,
          passwordSet ? currentPassword : undefined,
        );
      }

      const metadata: Record<string, string> = { ...agent.metadata };
      // Owned by dedicated APIs. `updateAgent` replaces the whole map, so
      // sending these back would fight the API that owns them.
      delete metadata.has_avatar;
      delete metadata.avatar_path;
      delete metadata.avatar_description;
      delete metadata.avatar_updated_at;
      delete metadata.has_power_password;
      delete metadata.has_vrm;
      delete metadata.vrm_path;
      if (memoryId) metadata.preferred_memory = memoryId;
      else delete metadata.preferred_memory;
      if (routing.length > 0) metadata.engine_routing = JSON.stringify(routing);
      else delete metadata.engine_routing;
      if (accentTriplet) metadata.accent = accentTriplet;
      else delete metadata.accent;
      applyRecallMetadata(metadata, recallPolicy, sessionScope);

      await api.updateAgent(agent.id, {
        name: name !== agent.name ? name : undefined,
        description: description !== agent.description ? description : undefined,
        default_engine_id: engineId || undefined,
        metadata,
      });

      if (precisionDirty && precisionSupported) {
        await api.setRecallPrecision(agent.id, precision);
      }

      // Avatar and VRM after updateAgent: those APIs write into the same
      // metadata map with json_set, so they have to land last.
      if (pendingAvatarDelete && !pendingAvatarFile) await api.deleteAvatar(agent.id);
      if (pendingAvatarFile) await api.uploadAvatar(agent.id, pendingAvatarFile);
      if (pendingVrmDelete && !pendingVrmFile) await api.deleteVrm(agent.id);
      if (pendingVrmFile) await api.uploadVrm(agent.id, pendingVrmFile);

      // One server at a time, each read immediately before it is written: the
      // endpoint replaces the server's whole entry set, so a list built from
      // anything older would delete the other agents' grants on it.
      for (const serverId of changedServers(loadedGrants, nextGrants)) {
        // Not caught: a read that failed is not an empty list. Writing this
        // agent's rows over "nothing" would be the deletion described above.
        const fresh = await api.getMcpServerAccess(serverId);
        const merged = mergeAgentEntries(fresh.entries, agent.id, serverId, grantsFor(nextGrants, serverId));
        await api.putMcpServerAccess(serverId, merged);
      }

      if (avatarPreviewUrl) URL.revokeObjectURL(avatarPreviewUrl);
      await refetchAgents();
      navigate('/');
    } catch (err) {
      setProblem(err instanceof Error ? err.message : t('settings.save_failed'));
    } finally {
      setSaving(false);
    }
  }

  function discard() {
    if (!agent) return;
    reset(agent, loadedGrants);
    setProblem(null);
  }

  const handleAvatarUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = '';
    if (!file) return;
    if (file.size > AVATAR_MAX_BYTES) {
      setProblem(t('plugin_workspace.avatar_too_large'));
      return;
    }
    if (avatarPreviewUrl) URL.revokeObjectURL(avatarPreviewUrl);
    setPendingAvatarFile(file);
    setPendingAvatarDelete(false);
    setAvatarPreviewUrl(URL.createObjectURL(file));
    setHasAvatar(true);
  };

  const handleAvatarDelete = () => {
    if (avatarPreviewUrl) URL.revokeObjectURL(avatarPreviewUrl);
    setPendingAvatarFile(null);
    setPendingAvatarDelete(true);
    setAvatarPreviewUrl(null);
    setHasAvatar(false);
  };

  const handleVrmUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = '';
    if (!file) return;
    setPendingVrmFile(file);
    setPendingVrmDelete(false);
    setHasVrm(true);
    if (sessionStorage.getItem('cloto-vrm-thumbnail-skip') === '1') return;
    try {
      const thumbnail = await extractVrmThumbnail(file);
      if (thumbnail) {
        setVrmThumbnailFile(thumbnail);
        setVrmThumbnailUrl(URL.createObjectURL(thumbnail));
        setShowVrmThumbnailDialog(true);
      }
    } catch {
      // A model without a usable thumbnail is not an error.
    }
  };

  const useVrmThumbnail = () => {
    if (avatarPreviewUrl) URL.revokeObjectURL(avatarPreviewUrl);
    setPendingAvatarFile(vrmThumbnailFile);
    setPendingAvatarDelete(false);
    setAvatarPreviewUrl(vrmThumbnailUrl);
    setHasAvatar(true);
    setShowVrmThumbnailDialog(false);
  };

  const skipVrmThumbnail = () => {
    if (vrmThumbnailUrl) URL.revokeObjectURL(vrmThumbnailUrl);
    setVrmThumbnailFile(null);
    setVrmThumbnailUrl(null);
    setShowVrmThumbnailDialog(false);
  };

  const toggleServerOpen = (serverId: string) => {
    setOpenServers((prev) => {
      const next = new Set(prev);
      if (next.has(serverId)) next.delete(serverId);
      else next.add(serverId);
      return next;
    });
    if (!toolsByServer[serverId]) {
      void api
        .getMcpServerTools(serverId)
        .then((tools) => setToolsByServer((prev) => ({ ...prev, [serverId]: tools })))
        .catch(() => setToolsByServer((prev) => ({ ...prev, [serverId]: [] })));
    }
  };

  const grantSeg = (
    value: 'default' | 'allow' | 'deny',
    onChange: (g: 'default' | 'allow' | 'deny') => void,
    label: string,
  ) => (
    <fieldset className="grant" aria-label={label}>
      <button type="button" className={value === 'default' ? 'on' : ''} onClick={() => onChange('default')}>
        {t('settings.grant_default')}
      </button>
      <button type="button" className={`allow${value === 'allow' ? ' on' : ''}`} onClick={() => onChange('allow')}>
        {t('settings.grant_allow')}
      </button>
      <button type="button" className={`deny${value === 'deny' ? ' on' : ''}`} onClick={() => onChange('deny')}>
        {t('settings.grant_deny')}
      </button>
    </fieldset>
  );

  const seg = <T extends string>(
    value: T,
    options: { value: T; label: string }[],
    onChange: (v: T) => void,
    label: string,
  ) => (
    <fieldset className="seg" aria-label={label}>
      {options.map((o) => (
        <button
          type="button"
          key={o.value}
          className={value === o.value ? 'on' : ''}
          aria-pressed={value === o.value}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </fieldset>
  );

  // Servers worth showing first: the ones this agent has an answer about, and
  // the ones that are running. The rest are one click away.
  const interesting = servers.filter(
    (s) =>
      grantsFor(grants, s.id).server !== 'default' ||
      Object.keys(grantsFor(grants, s.id).tools).length > 0 ||
      s.status === 'Connected',
  );
  const rest = servers.filter((s) => !interesting.includes(s));
  const orderedServers = [...interesting, ...rest];
  const shownServers = allServers ? orderedServers : orderedServers.slice(0, SERVERS_SHOWN);

  const leftOut = instructions?.files.filter((f) => f.present && !f.loaded) ?? [];

  // Every engine the picker can name: the connected ones, plus the one this
  // agent already has when it is not among them. Without that row a stopped
  // engine would draw as "Select…", which reads as "none chosen".
  const engineOptions = engineServers.map((s) => ({ value: s.id, label: displayServerId(s.id) }));
  if (engineId && !engineServers.some((s) => s.id === engineId)) {
    engineOptions.unshift({
      value: engineId,
      label: t('settings.engine_not_connected', { engine: displayServerId(engineId) }),
    });
  }

  return (
    // The page is about one agent, so it wears that agent's colour — the one
    // being tried in the colour row, before it is saved.
    <div className="ws" style={{ '--agent': previewAccent } as React.CSSProperties}>
      <div className="ws-head">
        <button type="button" className="btn back" onClick={() => navigate('/')}>
          ‹ {t('title')}
        </button>
        <span className="face-lg head">
          <AgentIcon agent={agent} size={agent.metadata?.has_avatar === 'true' ? 36 : 20} />
        </span>
        <h1>{agent.name}</h1>
        <span className="count mono">{agent.id}</span>
        <span className="spacer" />
        <button type="button" className="btn" onClick={() => navigate(`/?agent=${encodeURIComponent(agent.id)}`)}>
          {t('chat')}
        </button>
        {!isDefault && (
          <button type="button" className="btn" onClick={() => void exportAgent(api, agent)}>
            {t('export_config')}
          </button>
        )}
        <button type="button" className="btn" onClick={() => setPowerOpen(true)}>
          {agent.enabled ? t('roster.stop') : t('roster.start')}
        </button>
      </div>

      <div className="detail">
        <nav className="rail" aria-label={t('settings.sections')}>
          {SECTIONS.map((s) => (
            <button type="button" key={s} className={section === s ? 'on' : ''} onClick={() => jumpTo(s)}>
              {t(`settings.section_${s}`)}
            </button>
          ))}
        </nav>

        <div className="pane">
          {/* ---------------- Basics ---------------- */}
          <h2
            ref={(el) => {
              sectionRefs.current.basics = el;
            }}
          >
            {t('settings.section_basics')}
          </h2>
          <div className="frow">
            <div className="k">{t('form.name')}</div>
            <div className="v">
              <input
                className="in"
                aria-label={t('form.name')}
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </div>
          </div>
          <div className="frow">
            <div className="k">
              {t('form.description')}
              <small>{t('settings.description_sub')}</small>
            </div>
            <div className="v">
              <textarea
                className="in"
                aria-label={t('form.description')}
                value={description}
                onChange={(e) => setDescription(e.target.value)}
              />
            </div>
          </div>
          <div className="frow">
            <div className="k">
              {t('settings.password')}
              <small>{t('settings.password_sub')}</small>
            </div>
            <div className="v">
              {passwordSet && (
                <input
                  className="in"
                  type="password"
                  aria-label={t('settings.password_current')}
                  placeholder={t('settings.password_current')}
                  value={currentPassword}
                  onChange={(e) => setCurrentPassword(e.target.value)}
                />
              )}
              <input
                className="in"
                type="password"
                aria-label={t('settings.password_new')}
                placeholder={passwordSet ? t('settings.password_unchanged') : t('settings.password_none')}
                value={newPassword}
                disabled={removePassword}
                onChange={(e) => setNewPassword(e.target.value)}
              />
              {passwordSet && (
                <label className="hint">
                  <input
                    type="checkbox"
                    checked={removePassword}
                    onChange={(e) => {
                      setRemovePassword(e.target.checked);
                      if (e.target.checked) setNewPassword('');
                    }}
                  />{' '}
                  {t('settings.password_remove')}
                </label>
              )}
              <div className="hint">{t('settings.password_hint')}</div>
            </div>
          </div>
          <div className="frow">
            <div className="k">
              {t('settings.always_loaded')}
              <small>{t('settings.always_loaded_sub')}</small>
            </div>
            <div className="v">
              {leftOut.length > 0 && (
                <div className="dev warn">
                  {t('settings.left_out', { files: leftOut.map((f) => f.name).join(', ') })}
                </div>
              )}
              {instructions === null ? (
                <div className="hint">{t('settings.always_loaded_unknown')}</div>
              ) : (
                instructions.files.map((file) => (
                  <div className="tool" key={file.name}>
                    <span className="nm">{file.name}</span>
                    <span className="d">
                      {file.present
                        ? t('settings.file_present', {
                            chars: file.chars,
                            share: Math.round((file.chars / instructions.budget_chars) * 100),
                          })
                        : t('settings.file_absent')}
                    </span>
                    <span />
                  </div>
                ))
              )}
              <div className="hint">{t('settings.always_loaded_hint')}</div>
            </div>
          </div>

          {/* ---------------- Engine ---------------- */}
          <h2
            ref={(el) => {
              sectionRefs.current.engine = el;
            }}
          >
            {t('settings.section_engine')}
          </h2>
          <div className="frow">
            <div className="k">{t('form.llm_engine')}</div>
            <div className="v">
              <Select
                label={t('form.llm_engine')}
                placeholder={t('form.select')}
                value={engineId}
                options={engineOptions}
                onChange={setEngineId}
              />
              <div className="hint">{t('settings.engine_hint')}</div>
            </div>
          </div>
          <div className="frow">
            <div className="k">
              {t('routing.title')}
              <small>{t('settings.routing_sub')}</small>
            </div>
            <div className="v">
              {routing.map((rule, i) => (
                // Keyed by position: rules have no id, and two of them may hold
                // the same text while being edited.
                <div className="tool rule" key={i}>
                  <input
                    className="in mono"
                    aria-label={t('routing.match_label', { index: i + 1 })}
                    placeholder="contains:keyword"
                    value={rule.match ?? ''}
                    onChange={(e) => setRouting(routing.map((r, j) => (j === i ? { ...r, match: e.target.value } : r)))}
                  />
                  <Select
                    label={t('routing.engine_label', { index: i + 1 })}
                    placeholder={t('routing.select_engine')}
                    value={rule.engine ?? ''}
                    options={engineServers.map((s) => ({ value: s.id, label: displayServerId(s.id) }))}
                    onChange={(v) => setRouting(routing.map((r, j) => (j === i ? { ...r, engine: v } : r)))}
                  />
                  <button
                    type="button"
                    className="btn"
                    aria-label={t('routing.remove_rule')}
                    onClick={() => setRouting(routing.filter((_, j) => j !== i))}
                  >
                    ×
                  </button>
                </div>
              ))}
              <button
                type="button"
                className="btn"
                onClick={() => setRouting([...routing, { match: 'default', engine: '' }])}
              >
                {t('routing.add_rule')}
              </button>
              <div className="hint">{t('settings.routing_help')}</div>
            </div>
          </div>
          <div className="frow">
            <div className="k">
              {t('settings.fallback')}
              <small>{t('settings.fallback_sub')}</small>
            </div>
            <div className="v">
              <Select
                label={t('settings.fallback')}
                placeholder={t('routing.fallback_none')}
                value={routing.length > 0 ? (routing[routing.length - 1].fallback ?? '') : ''}
                options={[
                  { value: '', label: t('routing.fallback_none') },
                  ...engineServers.map((s) => ({ value: s.id, label: displayServerId(s.id) })),
                ]}
                disabled={routing.length === 0}
                onChange={(v) =>
                  setRouting(routing.map((r, j) => (j === routing.length - 1 ? { ...r, fallback: v || undefined } : r)))
                }
              />
              <div className="hint">{t('settings.fallback_hint')}</div>
            </div>
          </div>

          {/* ---------------- Memory ---------------- */}
          <h2
            ref={(el) => {
              sectionRefs.current.memory = el;
            }}
          >
            {t('settings.section_memory')}
          </h2>
          <div className="frow">
            <div className="k">{t('settings.memory_server')}</div>
            <div className="v">
              <Select
                label={t('settings.memory_server')}
                placeholder={t('form.memory_none')}
                value={memoryId}
                options={[
                  { value: '', label: t('form.memory_none') },
                  ...memoryServers.map((s) => ({ value: s.id, label: displayServerId(s.id) })),
                ]}
                onChange={setMemoryId}
              />
              <div className="hint">
                {memoryCount === null
                  ? t('settings.memory_count_unknown')
                  : t('settings.memory_count', { count: memoryCount })}
              </div>
            </div>
          </div>
          <div className="frow">
            <div className="k">
              {t('recall.timing_label')}
              <small>{t('recall.timing_hint')}</small>
            </div>
            <div className="v">
              {seg<RecallPolicyValue>(
                recallPolicy,
                [
                  { value: 'always', label: t('recall.timing_always') },
                  { value: 'session_start+active', label: t('recall.timing_session_active') },
                  { value: 'session_start', label: t('recall.timing_session') },
                  { value: 'manual_only', label: t('recall.timing_manual') },
                ],
                setRecallPolicy,
                t('recall.timing_label'),
              )}
            </div>
          </div>
          <div className="frow">
            <div className="k">
              {t('recall.scope_label')}
              <small>{t('recall.scope_hint')}</small>
            </div>
            <div className="v">
              {seg<SessionScopeValue>(
                sessionScope,
                [
                  { value: 'per_user', label: t('recall.scope_per_user') },
                  { value: 'channel', label: t('recall.scope_channel') },
                  { value: 'thread', label: t('recall.scope_thread') },
                ],
                setSessionScope,
                t('recall.scope_label'),
              )}
            </div>
          </div>
          <div className="frow">
            <div className="k">
              {t('recall.precision_label')}
              <small>{precisionSupported ? t('recall.precision_hint') : t('recall.precision_unsupported')}</small>
            </div>
            <div className="v">
              <fieldset className="seg" aria-label={t('recall.precision_label')} disabled={!precisionSupported}>
                {(['strict', 'balanced', 'lenient'] as PrecisionValue[]).map((p) => (
                  <button
                    type="button"
                    key={p}
                    className={precision === p ? 'on' : ''}
                    aria-pressed={precision === p}
                    onClick={() => {
                      setPrecision(p);
                      setPrecisionDirty(true);
                    }}
                  >
                    {t(`recall.precision_${p}`)}
                  </button>
                ))}
              </fieldset>
            </div>
          </div>

          {/* ---------------- Appearance ---------------- */}
          <h2
            ref={(el) => {
              sectionRefs.current.appearance = el;
            }}
          >
            {t('settings.section_appearance')}
          </h2>
          <div className="frow">
            <div className="k">
              {t('plugin_workspace.avatar')}
              <small>{t('settings.avatar_sub')}</small>
            </div>
            <div className="v">
              <div className="vrow">
                <span className="face-lg">
                  {avatarPreviewUrl ? (
                    <img src={avatarPreviewUrl} alt="" width={64} height={64} style={{ objectFit: 'cover' }} />
                  ) : (
                    <AgentIcon agent={hasAvatar ? agent : { ...agent, metadata: {} }} size={hasAvatar ? 64 : 32} />
                  )}
                </span>
                <span>
                  <label className="btn">
                    {t('settings.avatar_choose')}
                    <input type="file" accept="image/*" style={{ display: 'none' }} onChange={handleAvatarUpload} />
                  </label>
                  {vrmThumbnailFile && (
                    <button type="button" className="btn" onClick={() => setShowVrmThumbnailDialog(true)}>
                      {t('settings.avatar_from_vrm')}
                    </button>
                  )}
                  {hasAvatar && (
                    <button type="button" className="btn" onClick={handleAvatarDelete}>
                      {t('plugin_workspace.avatar_remove')}
                    </button>
                  )}
                  <div className="hint">{t('settings.avatar_hint')}</div>
                </span>
              </div>
            </div>
          </div>
          <div className="frow">
            <div className="k">
              {t('settings.vrm')}
              <small>{t('settings.vrm_sub')}</small>
            </div>
            <div className="v">
              <div className="vrow">
                <label className="btn">
                  {t('settings.vrm_choose')}
                  <input type="file" accept=".vrm" style={{ display: 'none' }} onChange={handleVrmUpload} />
                </label>
                {hasVrm && (
                  <button
                    type="button"
                    className="btn"
                    onClick={() => {
                      setPendingVrmFile(null);
                      setPendingVrmDelete(true);
                      setHasVrm(false);
                    }}
                  >
                    {t('settings.vrm_remove')}
                  </button>
                )}
                <span className="hint" style={{ margin: 0 }}>
                  {hasVrm ? t('settings.vrm_present') : t('settings.vrm_absent')}
                </span>
              </div>
            </div>
          </div>
          <div className="frow">
            <div className="k">
              {t('settings.colour')}
              <small>{t('settings.colour_sub')}</small>
            </div>
            <div className="v">
              <div className="crow">
                <span className="dot-lg" style={{ background: `hsl(${previewAccent})` }} />
                <input
                  className="in mono narrow"
                  aria-label={t('settings.colour')}
                  placeholder={`hsl(${agentAccentTriplet({ id: agent.id })})`}
                  value={accent}
                  onChange={(e) => setAccent(e.target.value)}
                />
                <span className="hint">{t('settings.colour_hint')}</span>
              </div>
              {!accentValid && <div className="dev">{t('settings.accent_invalid')}</div>}
            </div>
          </div>

          {/* ---------------- Tool permissions ---------------- */}
          <h2
            ref={(el) => {
              sectionRefs.current.tools = el;
            }}
          >
            {t('settings.section_tools')}
          </h2>
          <div className="hint" style={{ margin: '0 0 8px' }}>
            {t('settings.tools_hint')}
          </div>
          {shownServers.map((server) => {
            const open = openServers.has(server.id);
            const denied = deniedToolCount(grants, server.id);
            return (
              <div key={server.id}>
                <div className="tool">
                  <button type="button" className="nm" aria-expanded={open} onClick={() => toggleServerOpen(server.id)}>
                    {server.id}
                  </button>
                  <span className="d">
                    {denied > 0
                      ? t('settings.tools_with_denied', { count: server.tools.length, denied })
                      : t('settings.tools_count', { count: server.tools.length })}
                  </span>
                  {grantSeg(
                    grantsFor(grants, server.id).server,
                    (g) => setGrants(withServerGrant(grants, server.id, g)),
                    t('settings.grant_for', { server: server.id }),
                  )}
                </div>
                {open &&
                  (toolsByServer[server.id] ?? server.tools.map((name) => ({ name, description: null }))).map(
                    (tool) => (
                      <div className="tool sub" key={tool.name}>
                        <span className="nm">{tool.name}</span>
                        <span className="d">{tool.description ?? ''}</span>
                        {grantSeg(
                          grantsFor(grants, server.id).tools[tool.name] ?? 'default',
                          (g) => setGrants(withToolGrant(grants, server.id, tool.name, g)),
                          t('settings.grant_for_tool', { server: server.id, tool: tool.name }),
                        )}
                      </div>
                    ),
                  )}
              </div>
            );
          })}
          {!allServers && orderedServers.length > SERVERS_SHOWN && (
            <button type="button" className="btn" onClick={() => setAllServers(true)}>
              {t('settings.show_rest', { count: orderedServers.length - SERVERS_SHOWN })}
            </button>
          )}

          {/* ---------------- Danger zone ---------------- */}
          <h2
            className="danger"
            ref={(el) => {
              sectionRefs.current.danger = el;
            }}
          >
            {t('settings.section_danger')}
          </h2>
          {!isDefault && (
            <div className="frow">
              <div className="k">
                <span className="danger">{t('settings.delete_title')}</span>
                <small>{t('settings.delete_sub')}</small>
              </div>
              <div className="v">
                <button type="button" className="btn danger" onClick={() => setDeleteOpen(true)}>
                  {t('settings.delete_agent', { name: agent.name })}
                </button>
              </div>
            </div>
          )}
          {isDefault && <div className="hint">{t('settings.default_protected')}</div>}
        </div>
      </div>

      <div className="savebar">
        <span>{problem ? <span className="danger">{problem}</span> : t('settings.unsaved_note')}</span>
        <span className="spacer" />
        <button type="button" className="btn" onClick={discard} disabled={!dirty || saving}>
          {t('settings.discard')}
        </button>
        <button type="button" className="btn pri" onClick={handleSave} disabled={!dirty || saving}>
          {saving ? t('settings.saving') : tc('save')}
        </button>
      </div>

      <VrmThumbnailDialog
        open={showVrmThumbnailDialog}
        thumbnailUrl={vrmThumbnailUrl ?? ''}
        onApply={useVrmThumbnail}
        onSkip={skipVrmThumbnail}
      />
      {powerOpen && (
        <PowerToggleModal
          agent={agent}
          onClose={() => setPowerOpen(false)}
          onSuccess={() => {
            void refetchAgents();
          }}
        />
      )}
      {deleteOpen && (
        <DeleteAgentModal
          agent={agent}
          onClose={() => setDeleteOpen(false)}
          onDeleted={() => {
            setDeleteOpen(false);
            void refetchAgents();
            navigate('/');
          }}
        />
      )}
    </div>
  );
}

/** The agent's routing rules, or none when the metadata holds nothing readable. */
function parseRouting(raw: string | undefined): RoutingRuleEntry[] {
  if (!raw) return [];
  try {
    const rules = JSON.parse(raw);
    return Array.isArray(rules) ? rules : [];
  } catch {
    return [];
  }
}
