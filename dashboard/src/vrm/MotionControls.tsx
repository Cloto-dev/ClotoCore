import { type RefObject, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../hooks/useApi';
import { validateModelFile } from '../mate/assetFiles';
import { partnerSettingsChanged } from '../mate/usePartnerMedia';
import type { AgentMetadata } from '../types';
import { COMPANION_GESTURES, type MotionStyle } from './engine/companionMotionLibrary';
import type { IdleBehaviorParams } from './engine/types';
import type { VrmAnimationController } from './engine/VrmAnimationController';
import { MotionComparison } from './MotionComparison';
import { motionKeys, motionPoses, savedIdleBehavior, savedMotionPose, savedMotionStyle } from './motionSettings';
import { SLIDER_GROUPS } from './poseControls';

export function MotionControls({
  agent,
  controller,
  ready,
  onSaved,
}: {
  agent: AgentMetadata;
  controller: RefObject<VrmAnimationController | null>;
  ready: boolean;
  onSaved: () => Promise<void>;
}) {
  const { t } = useTranslation('agents');
  const api = useApi();
  const [style, setStyle] = useState(() => savedMotionStyle(agent));
  const [idle, setIdle] = useState(() => savedIdleBehavior(agent));
  const [preset, setPreset] = useState(() => savedMotionPose(agent));
  const drafts = useRef<Partial<Record<MotionStyle, { idle: IdleBehaviorParams; preset: string }>>>({});
  const [mode, setMode] = useState<'animation' | 'pose'>('animation');
  const [motion, setMotion] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    setStyle(savedMotionStyle(agent));
    setIdle(savedIdleBehavior(agent));
    setPreset(savedMotionPose(agent));
    drafts.current = {};
  }, [
    agent.id,
    agent.metadata?.mate_idle,
    agent.metadata?.mate_pose,
    agent.metadata?.mate_motion_style,
    agent.metadata?.mate_companion_idle,
    agent.metadata?.mate_companion_pose,
  ]);
  useEffect(() => setSaved(false), [agent.id]);
  function compare(nextStyle: MotionStyle) {
    drafts.current[style] = { idle, preset };
    const next = drafts.current[nextStyle] ?? {
      idle: savedIdleBehavior(agent, nextStyle),
      preset: savedMotionPose(agent, nextStyle),
    };
    setStyle(nextStyle);
    if (motion) controller.current?.stopVrma();
    setIdle(next.idle);
    setPreset(next.preset);
    setMotion(null);
    setProblem(null);
    setSaved(false);
    controller.current?.setMotionStyle(nextStyle);
    controller.current?.setIdleParams(next.idle);
    if (next.preset) void controller.current?.setPose(next.preset);
  }
  function update(next: IdleBehaviorParams) {
    setIdle(next);
    setSaved(false);
    controller.current?.setIdleParams(next);
  }
  async function loadMotion(file: File) {
    if (!ready || !controller.current || busy) return;
    setBusy(true);
    setProblem(null);
    const target = controller.current;
    try {
      await validateModelFile(file, 'motion');
      if (mode === 'animation') await target.loadVrmaAnimationFile(file);
      else await target.loadVrmaPoseFile(file);
      setMotion(file.name);
      setPreset('');
    } catch {
      setProblem(t('mate.motion_failed'));
    } finally {
      setBusy(false);
    }
  }
  async function save() {
    if (busy) return;
    setBusy(true);
    setProblem(null);
    setSaved(false);
    try {
      // Read the latest metadata before replacing the opaque map, preserving other settings.
      const fresh = (await api.getAgents()).find((a) => a.id === agent.id);
      if (!fresh) throw new Error('Partner unavailable');
      const metadata = { ...fresh.metadata };
      // Derived flags and media fields belong to their dedicated API, not this save.
      for (const key of [
        'has_avatar',
        'avatar_path',
        'avatar_description',
        'avatar_updated_at',
        'has_vrm',
        'vrm_path',
        'has_power_password',
        'has_password',
      ])
        delete metadata[key];
      await api.updateAgent(agent.id, {
        metadata: {
          ...metadata,
          mate_motion_style: style,
          [motionKeys(style).idle]: JSON.stringify(idle),
          [motionKeys(style).pose]: preset,
        },
      });
      partnerSettingsChanged(agent.id);
      await onSaved();
      setSaved(true);
    } catch {
      setProblem(t('mate.save_failed'));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section
      className="mate-settings mate-motion"
      aria-label={t('mate.motion')}
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => {
        e.preventDefault();
        const file = e.dataTransfer.files[0];
        if (file) void loadMotion(file);
      }}
    >
      <h2>{t('mate.motion')}</h2>
      <p>{t('mate.motion_hint')}</p>
      {!ready && <p className="mate-notice">{t('mate.motion_needs_vrm')}</p>}
      <MotionComparison
        agent={agent}
        controller={controller}
        ready={ready}
        motion={motion}
        onStarted={(name) => {
          setMotion(name);
          setPreset('');
        }}
      />
      <fieldset disabled={!ready || busy}>
        <legend className="sr-only">{t('mate.motion')}</legend>
        <label className="mate-select-label">
          {t('mate.motion_style')}
          <select value={style} onChange={(e) => compare(e.target.value as MotionStyle)}>
            <option value="companion">{t('mate.companion_motion')}</option>
            <option value="legacy">{t('mate.legacy_motion')}</option>
          </select>
        </label>
        <p>{t('mate.motion_compare_hint')}</p>
        <h3>{t('mate.posture')}</h3>
        <div className="mate-preset-list">
          {Object.keys(motionPoses(style)).map((name) => (
            <button
              type="button"
              key={name}
              aria-pressed={preset === name}
              onClick={() => {
                setProblem(null);
                setPreset(name);
                setMotion(null);
                update({ ...idle, pose: { ...motionPoses(style)[name] } });
                void controller.current?.setPose(name);
              }}
            >
              {t(`mate.poses.${name}`)}
            </button>
          ))}
        </div>
        {style === 'companion' && (
          <div className="mate-field">
            <h3>{t('mate.try_gesture')}</h3>
            <div className="mate-preset-list">
              {COMPANION_GESTURES.map((name) => (
                <button
                  type="button"
                  key={name}
                  disabled={Boolean(motion)}
                  onClick={() => controller.current?.previewGesture(name)}
                >
                  {t(`mate.gestures.${name}`)}
                </button>
              ))}
            </div>
            <p>{t('mate.gesture_preview_hint')}</p>
          </div>
        )}
        <div className="mate-field">
          <h3>{t('mate.idle')}</h3>
          {(['breathing_rate', 'sway_amplitude', 'blink_frequency'] as const).map((key) => (
            <label className="mate-slider" key={key}>
              <span>{t(`mate.${key}`)}</span>
              <input
                type="range"
                min={0}
                max={2}
                step={0.1}
                value={idle[key]}
                onChange={(e) => update({ ...idle, [key]: Number(e.target.value) })}
              />
              <output>{idle[key].toFixed(1)}</output>
            </label>
          ))}
        </div>
        <div className="mate-field">
          <h3>{t('mate.motion_file')}</h3>
          <label className="mate-select-label">
            {t('mate.play_as')}
            <select value={mode} onChange={(e) => setMode(e.target.value as typeof mode)}>
              <option value="animation">{t('mate.animation')}</option>
              <option value="pose">{t('mate.static_pose')}</option>
            </select>
          </label>
          <label className="mate-upload">
            {t('mate.choose_motion')}
            <input
              type="file"
              accept=".vrma,.glb"
              onChange={(e) => {
                const file = e.target.files?.[0];
                e.target.value = '';
                if (file) void loadMotion(file);
              }}
            />
          </label>
          {motion && (
            <div className="mate-motion-current">
              <span>{motion}</span>
              <button
                type="button"
                onClick={() => {
                  controller.current?.stopVrma();
                  setMotion(null);
                  setPreset('');
                }}
              >
                {t('mate.stop_motion')}
              </button>
            </div>
          )}
          <p>{t('mate.motion_local')}</p>
        </div>
        <details>
          <summary>{t('mate.pose_detail')}</summary>
          {SLIDER_GROUPS.map(({ group, sliders }) => (
            <div className="mate-field" key={group}>
              <h3>{t(`mate.groups.${group.replaceAll(' ', '_').toLowerCase()}`)}</h3>
              {sliders.map(({ key, min, max, step }) => (
                <label className="mate-slider" key={key}>
                  <span>{t(`mate.bones.${key}`)}</span>
                  <input
                    type="range"
                    min={min}
                    max={max}
                    step={step}
                    value={idle.pose[key]}
                    onChange={(e) => {
                      controller.current?.stopVrma();
                      setMotion(null);
                      setPreset('');
                      update({ ...idle, pose: { ...idle.pose, [key]: Number(e.target.value) } });
                    }}
                  />
                  <output>{idle.pose[key].toFixed(2)}</output>
                </label>
              ))}
            </div>
          ))}
        </details>
        <button type="button" onClick={() => void save()}>
          {t('mate.save_motion')}
        </button>
      </fieldset>
      {problem && (
        <p role="alert" className="mate-problem">
          {problem}
        </p>
      )}
      <p role="status">{busy ? t('mate.saving') : saved ? t('mate.saved') : ''}</p>
    </section>
  );
}
