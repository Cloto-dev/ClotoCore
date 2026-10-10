import { type RefObject, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { AgentMetadata } from '../types';
import { importMotion, MotionImportError } from './engine/MotionImport';
import type { VrmAnimationController } from './engine/VrmAnimationController';
import { type MotionPlayback, VrmaLoader } from './engine/VrmaLoader';
import {
  listMotions,
  MOTION_SOURCES,
  type MotionEntry,
  motionReport,
  removeMotion,
  saveMotion,
  saveMotionReview,
} from './motionLibrary';

const EMPTY: MotionPlayback = { active: false, paused: false, duration: 0, time: 0, speed: 1 };
export function MotionComparison({
  agent,
  controller,
  ready,
  motion,
  onStarted,
}: {
  agent: AgentMetadata;
  controller: RefObject<VrmAnimationController | null>;
  ready: boolean;
  motion: string | null;
  onStarted: (name: string | null) => void;
}) {
  const { t } = useTranslation('agents');
  const [entries, setEntries] = useState<MotionEntry[]>([]);
  const [selected, setSelected] = useState('');
  const [source, setSource] = useState<string>(MOTION_SOURCES[0]);
  const [conditions, setConditions] = useState('');
  const [notes, setNotes] = useState('');
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState('');
  const [notice, setNotice] = useState('');
  const [playback, setPlayback] = useState(EMPTY);
  const [fps, setFps] = useState(30);
  const playing = useRef<VrmAnimationController | null>(null);
  const live = useRef(true);
  const entry = entries.find((row) => row.id === selected);
  useEffect(() => {
    live.current = true;
    void listMotions()
      .then((rows) => {
        if (live.current) setEntries(rows);
      })
      .catch(() => {
        if (live.current) setProblem(t('mate.lab.storage_failed'));
      });
    return () => {
      live.current = false;
    };
  }, []);
  useEffect(() => {
    setNotes(entry?.reviews[agent.id]?.notes ?? '');
    setFps(entry?.fps ?? 30);
    setNotice('');
  }, [selected, agent.id]);
  useEffect(() => {
    const timer = window.setInterval(() => {
      const target = controller.current;
      setPlayback(
        ready && target?.isVrmaActive && (target.comparisonMotionName || (motion && target === playing.current))
          ? target.motionPlayback
          : EMPTY,
      );
    }, 100);
    return () => clearInterval(timer);
  }, [controller, ready, motion]);

  async function register(file: File) {
    if (busy) return;
    setBusy(true);
    setProblem('');
    setNotice('');
    const loader = new VrmaLoader();
    try {
      const animation = await importMotion(file, (f) => loader.loadFile(f));
      const hash = await crypto.subtle.digest('SHA-256', await file.arrayBuffer());
      const row: MotionEntry = {
        id: crypto.randomUUID(),
        name: file.name.replace(/\.[^.]+$/, ''),
        source: source.trim() || 'Other',
        conditions,
        filename: file.name,
        file,
        sha256: Array.from(new Uint8Array(hash), (byte) => byte.toString(16).padStart(2, '0')).join(''),
        duration: animation.duration,
        bones: animation.humanoidTracks.rotation.size,
        fps: 30,
        added: new Date().toISOString(),
        reviews: {},
      };
      await saveMotion(row);
      if (live.current) {
        setEntries(await listMotions());
        setSelected(row.id);
        setNotice(t('mate.lab.registered'));
      }
    } catch (error) {
      if (live.current)
        setProblem(t(error instanceof MotionImportError ? `mate.lab.error_${error.code}` : 'mate.lab.import_failed'));
    } finally {
      loader.dispose();
      if (live.current) setBusy(false);
    }
  }

  async function play() {
    const target = controller.current;
    if (!entry || !target || !ready || busy) return;
    setBusy(true);
    setProblem('');
    setNotice('');
    try {
      await target.loadComparisonMotion(new File([entry.file], entry.filename));
      if (live.current && controller.current === target) {
        playing.current = target;
        onStarted(entry.name);
        setPlayback(target.motionPlayback);
      }
    } catch {
      if (live.current) setProblem(t('mate.lab.play_failed'));
    } finally {
      if (live.current) setBusy(false);
    }
  }

  function download() {
    const url = URL.createObjectURL(new Blob([motionReport(entries)], { type: 'application/json' }));
    const a = document.createElement('a');
    a.href = url;
    a.download = 'mizmate-motion-comparison.json';
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  return (
    <div
      className="mate-field mate-lab"
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => {
        e.preventDefault();
        e.stopPropagation();
        const file = e.dataTransfer.files[0];
        if (file) void register(file);
      }}
    >
      <h3>{t('mate.lab.title')}</h3>
      <p>{t('mate.lab.hint')}</p>
      <details open={entries.length === 0}>
        <summary>{t('mate.lab.register')}</summary>
        <label className="mate-select-label">
          {t('mate.lab.source')}
          <input
            type="text"
            list="mate-motion-sources"
            value={source}
            maxLength={80}
            onChange={(e) => setSource(e.target.value)}
          />
          <datalist id="mate-motion-sources">
            {MOTION_SOURCES.map((name) => (
              <option key={name} value={name} />
            ))}
          </datalist>
        </label>
        <label className="mate-lab-text">
          {t('mate.lab.conditions')}
          <textarea value={conditions} maxLength={4000} onChange={(e) => setConditions(e.target.value)} />
        </label>
        <label className="mate-upload">
          {t('mate.lab.register')}
          <input
            type="file"
            disabled={busy}
            accept=".vrma,.glb,.fbx,.bvh"
            onChange={(e) => {
              const file = e.target.files?.[0];
              e.target.value = '';
              if (file) void register(file);
            }}
          />
        </label>
        <details>
          <summary>{t('mate.lab.formats')}</summary>
          <p>{t('mate.lab.format_hint')}</p>
          <p>{t('mate.lab.conversion_hint')}</p>
        </details>
        <p>{t('mate.lab.local')}</p>
      </details>
      {playback.active && (
        <div className="mate-lab-player">
          <p>{t('mate.lab.playing', { name: motion || controller.current?.comparisonMotionName })}</p>
          <label className="mate-slider">
            <span>{t('mate.lab.position')}</span>
            <input
              type="range"
              min={0}
              max={playback.duration}
              step={0.001}
              value={playback.time}
              onChange={(e) => {
                controller.current?.pauseMotion(true);
                controller.current?.seekMotion(Number(e.target.value));
                setPlayback(controller.current?.motionPlayback ?? EMPTY);
              }}
            />
            <output>{playback.time.toFixed(2)} s</output>
          </label>
          <div className="mate-preset-list">
            <button
              type="button"
              onClick={() => {
                controller.current?.pauseMotion(!playback.paused);
                setPlayback(controller.current?.motionPlayback ?? EMPTY);
              }}
            >
              {t(playback.paused ? 'mate.lab.resume' : 'mate.lab.pause')}
            </button>
            {[-1, 1].map((direction) => (
              <button
                type="button"
                key={direction}
                onClick={() => {
                  const target = controller.current;
                  target?.pauseMotion(true);
                  target?.seekMotion(playback.time + direction / fps);
                  setPlayback(target?.motionPlayback ?? EMPTY);
                }}
              >
                {t(direction === -1 ? 'mate.lab.previous_frame' : 'mate.lab.next_frame')}
              </button>
            ))}
            <button
              type="button"
              onClick={() => {
                controller.current?.stopVrma();
                playing.current = null;
                onStarted(null);
                setPlayback(EMPTY);
              }}
            >
              {t('mate.stop_motion')}
            </button>
          </div>
          <label className="mate-select-label">
            {t('mate.lab.speed')}
            <select
              value={playback.speed}
              onChange={(e) => {
                controller.current?.setMotionSpeed(Number(e.target.value));
                setPlayback(controller.current?.motionPlayback ?? EMPTY);
              }}
            >
              {[0.25, 0.5, 1, 1.5, 2].map((value) => (
                <option key={value} value={value}>
                  {value}×
                </option>
              ))}
            </select>
          </label>
          <label className="mate-select-label">
            {t('mate.lab.fps')}
            <select value={fps} onChange={(e) => setFps(Number(e.target.value))}>
              {[24, 30, 60].map((value) => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </select>
          </label>
        </div>
      )}
      {entries.length > 0 ? (
        <>
          <label className="mate-select-label">
            {t('mate.lab.selection')}
            <select value={selected} onChange={(e) => setSelected(e.target.value)}>
              <option value="">{t('mate.lab.select')}</option>
              {entries.map((row) => (
                <option key={row.id} value={row.id}>
                  {row.source} · {row.name}
                </option>
              ))}
            </select>
          </label>
          {entry && (
            <>
              <p className="mate-lab-meta">
                {entry.source} · {entry.filename} · {entry.duration.toFixed(2)} s ·{' '}
                {t('mate.lab.bones', { count: entry.bones })}
              </p>
              {entry.conditions && <p className="mate-lab-conditions">{entry.conditions}</p>}
              <div className="mate-preset-list">
                <button type="button" disabled={!ready || busy} onClick={() => void play()}>
                  {t('mate.lab.play')}
                </button>
                <button
                  type="button"
                  disabled={busy}
                  onClick={async () => {
                    setBusy(true);
                    setProblem('');
                    try {
                      await removeMotion(entry.id);
                      setEntries(await listMotions());
                      setSelected('');
                      setNotice(t('mate.lab.removed'));
                    } catch {
                      setProblem(t('mate.lab.storage_failed'));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  {t('mate.lab.remove')}
                </button>
              </div>
              <label className="mate-lab-text">
                {t('mate.lab.review', { avatar: agent.name })}
                <textarea
                  value={notes}
                  maxLength={4000}
                  onChange={(e) => {
                    setNotes(e.target.value);
                    setNotice('');
                  }}
                />
              </label>
              <button
                type="button"
                disabled={busy}
                onClick={async () => {
                  setBusy(true);
                  setProblem('');
                  try {
                    await saveMotionReview(entry.id, agent.id, agent.name, notes, fps);
                    setEntries(await listMotions());
                    setNotice(t('mate.lab.saved'));
                  } catch {
                    setProblem(t('mate.lab.storage_failed'));
                  } finally {
                    setBusy(false);
                  }
                }}
              >
                {t('mate.lab.save_review')}
              </button>
            </>
          )}
          <button type="button" onClick={download}>
            {t('mate.lab.export')}
          </button>
        </>
      ) : (
        <p>{t('mate.lab.empty')}</p>
      )}
      {problem && (
        <p role="alert" className="mate-problem">
          {problem}
        </p>
      )}
      <p role="status">{busy ? t('mate.lab.busy') : notice}</p>
    </div>
  );
}
