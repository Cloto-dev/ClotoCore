import { lazy, Suspense, useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Route, BrowserRouter as Router, Routes } from 'react-router-dom';
import { AppLayout } from './components/AppLayout';
import { SetupWizard } from './components/SetupWizard';
import { AgentProvider } from './contexts/AgentContext';
import { useConnection } from './contexts/ConnectionContext';
import { checkForUpdates, isTauri } from './lib/tauri';
import { api } from './services/api';

const UPDATE_CHECK_DELAY_MS = 3000;

const MemoryCore = lazy(() => import('./components/MemoryCore').then((m) => ({ default: m.MemoryCore })));
const McpServersPage = lazy(() => import('./pages/McpServersPage').then((m) => ({ default: m.McpServersPage })));
const CronJobs = lazy(() => import('./components/CronJobs').then((m) => ({ default: m.CronJobs })));
const ModulePage = lazy(() => import('./pages/ModulePage').then((m) => ({ default: m.ModulePage })));
const VrmViewerPage = lazy(() => import('./vrm/VrmViewerPage').then((m) => ({ default: m.VrmViewerPage })));

export function App() {
  const [setupDone, setSetupDone] = useState(() => localStorage.getItem('cloto-setup-completed') === '1');

  const { connected } = useConnection();
  const { t } = useTranslation();

  // Re-trigger the setup wizard ONLY when the session began with a stored
  // completion flag and the backend disagrees — i.e. the version-upgrade
  // scenario where the stored `cloto-setup-completed=1` predates the new
  // kernel. We capture the *mount-time* value of `setupDone` once; if the
  // session started with no flag (fresh install), this effect never runs at
  // all, so a Finish click never bounces.
  //
  // bug-384: bug-383's `useRef(true)` still consumed the gate on the first
  // false→true transition of `setupDone` (i.e. right after onComplete on a
  // fresh install), which could bounce the user back to step 0 if the backend
  // momentarily reported `setup_complete=false`. Capturing `setupDone` itself
  // is the correct gate — `useRef` only reads its initial argument on the
  // first render, so a later `setSetupDone(true)` cannot retroactively open
  // this guard.
  const checkOnUpgradeRef = useRef<boolean>(setupDone);
  useEffect(() => {
    if (!connected || !setupDone) return;
    if (!checkOnUpgradeRef.current) return;
    checkOnUpgradeRef.current = false;
    api
      .getSetupStatus()
      .then((status) => {
        if (!status.setup_complete && !status.setup_in_progress) {
          localStorage.removeItem('cloto-setup-completed');
          setSetupDone(false);
        }
      })
      .catch(() => {
        /* ignore — backend not ready yet */
      });
  }, [connected, setupDone]);

  // Auto-update check on startup (Tauri only, user-configurable)
  useEffect(() => {
    if (!connected || !isTauri) return;
    if (localStorage.getItem('cloto-auto-update') === 'off') return;

    const timer = setTimeout(async () => {
      try {
        const info = await checkForUpdates();
        if (info.available) {
          window.dispatchEvent(new CustomEvent('cloto-update-available', { detail: { version: info.latestVersion } }));
        }
      } catch {
        // Silent fail — network unavailable, rate limited, etc.
      }
    }, UPDATE_CHECK_DELAY_MS);
    return () => clearTimeout(timer);
  }, [connected]);

  // VRM viewer window bypasses connection gate (it loads VRM directly from API)
  const isVrmRoute = window.location.pathname.startsWith('/vrm-viewer/');

  // Rotating flavour text for loading screen
  const BOOT_LINES = t('boot.lines', { returnObjects: true }) as string[];

  const pickRandom = useCallback(
    (prev: number) => {
      let next: number;
      do {
        next = Math.floor(Math.random() * BOOT_LINES.length);
      } while (next === prev);
      return next;
    },
    [BOOT_LINES],
  );

  const [lineIdx, setLineIdx] = useState(() => Math.floor(Math.random() * BOOT_LINES.length));

  useEffect(() => {
    if (connected) return;
    const id = setInterval(() => setLineIdx((prev) => pickRandom(prev)), 6000);
    return () => clearInterval(id);
  }, [connected, pickRandom]);

  // Full-screen loading screen while backend is unreachable
  if (!connected && !isVrmRoute) {
    return (
      <div className="min-h-screen bg-surface-base flex flex-col items-center justify-center select-none">
        <h1 className="text-4xl font-black tracking-[0.2em] text-content-primary">{t('boot.title')}</h1>
        <div className="mt-6 h-6 flex items-center justify-center">
          <p
            key={lineIdx}
            className="text-[11px] font-mono text-brand/70 uppercase tracking-[0.15em] animate-boot-line"
          >
            {BOOT_LINES[lineIdx]}
          </p>
        </div>
      </div>
    );
  }

  // First-run setup wizard — the desktop shell only (skip for VRM viewer window).
  //
  // The wizard is a *machine's* first-run affordance, and `localStorage` is an
  // honest store for that: one desktop shell, one machine, one flag. A browser
  // pointed at the same kernel is a different origin with an empty flag, which
  // is why every new browser got all 8 steps against a kernel that had been
  // configured for days.
  //
  // The kernel cannot be asked instead. `/api/setup/status` reports
  // `setup_complete: true` from the very first boot: the migration suite seeds
  // an `agents` row and `status_handler` accepts that row as proof of a usable
  // setup (the bug-384 fallback in `handlers/setup.rs`). Measured 2026-09-10 on
  // the headless deployment — `setup_complete: true` while `mcp_servers_present`,
  // `venv_exists` and `uv_available` were all false. Gating on it would not tell
  // a fresh install from a configured one; it would delete the first-run wizard
  // everywhere.
  //
  // Hiding it from the browser is structural rather than cosmetic, because the
  // wizard writes: `applyPreset()` *replaces* the default agent's MCP grant set
  // (`putAgentMcpAccess`). A browser never reaches that today only because it
  // cannot obtain the admin key and the call returns early — the safety is an
  // accident of a key path that does not work, and it would arm itself the day
  // a route handed the key out. Nothing here installs anything the marketplace
  // page cannot install afterwards.
  if (isTauri && !setupDone && !isVrmRoute) {
    return (
      <SetupWizard
        onComplete={() => {
          localStorage.setItem('cloto-setup-completed', '1');
          setSetupDone(true);
        }}
      />
    );
  }

  return (
    <AgentProvider>
      <Router>
        <Routes>
          <Route
            path="/vrm-viewer/:agentId"
            element={
              <Suspense fallback={null}>
                <VrmViewerPage />
              </Suspense>
            }
          />
          <Route element={<AppLayout />}>
            <Route path="/" element={null} />
            <Route path="/dashboard" element={<MemoryCore />} />
            <Route path="/mcp-servers" element={<McpServersPage />} />
            <Route path="/cron" element={<CronJobs />} />
            <Route path="/modules/:id" element={<ModulePage />} />
          </Route>
        </Routes>
      </Router>
    </AgentProvider>
  );
}
