import { AlertTriangle, Lock, PackageX, Search, Unlock } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { useMarketplace } from '../../hooks/useMarketplace';
import { useUnlistedInstalls } from '../../hooks/useUnlistedInstalls';
import { extractError } from '../../lib/errors';
import type { MarketplaceCatalogEntry } from '../../types';
import { ConfirmDialog } from '../ui/ConfirmDialog';
import { InstallDialog } from './InstallDialog';
import { MarketplaceCard } from './MarketplaceCard';

const IS_DEV = import.meta.env.DEV;

const ERROR_DISPLAY_MS = 5000;

const CATEGORIES: Array<{ key: string; label: string }> = [
  { key: 'all', label: 'filter_all' },
  { key: 'mind', label: 'mind' },
  { key: 'memory', label: 'memory' },
  { key: 'tool', label: 'tool' },
  { key: 'vision', label: 'vision' },
  { key: 'voice', label: 'voice' },
  { key: 'output', label: 'output' },
  { key: 'io', label: 'io' },
];

interface MarketplaceTabProps {
  onRefetchRef?: React.MutableRefObject<(() => Promise<void>) | null>;
}

export function MarketplaceTab({ onRefetchRef }: MarketplaceTabProps) {
  const { t } = useTranslation('mcp');
  const { servers, isLoading, error, refetch } = useMarketplace();

  // Expose refetch to parent so header refresh button can trigger it
  useEffect(() => {
    if (onRefetchRef) onRefetchRef.current = refetch;
    return () => {
      if (onRefetchRef) onRefetchRef.current = null;
    };
  }, [onRefetchRef, refetch]);

  const [actionsEnabled, setActionsEnabled] = useState(!IS_DEV);
  const [searchQuery, setSearchQuery] = useState('');
  const [categoryFilter, setCategoryFilter] = useState('all');
  const [installingServer, setInstallingServer] = useState<MarketplaceCatalogEntry | null>(null);
  // Both a catalog entry and an unlisted install answer to the same uninstall:
  // the endpoint keys off the row name, so one flow serves both.
  const [uninstallTarget, setUninstallTarget] = useState<{ id: string; name: string } | null>(null);
  const [uninstalling, setUninstalling] = useState(false);
  const [toast, setToast] = useState<{ type: 'success' | 'error'; message: string } | null>(null);
  const api = useApi();
  const unlisted = useUnlistedInstalls();

  // Filter servers
  const filtered = servers.filter((s) => {
    // Category filter
    if (categoryFilter !== 'all' && s.category !== categoryFilter) return false;
    // Search filter
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      return (
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        s.tags.some((tag) => tag.toLowerCase().includes(q))
      );
    }
    return true;
  });

  function showToast(type: 'success' | 'error', message: string) {
    setToast({ type, message });
    setTimeout(() => setToast(null), ERROR_DISPLAY_MS);
  }

  function handleInstalled() {
    const name = installingServer?.name ?? '';
    setInstallingServer(null);
    refetch();
    showToast('success', t('marketplace.install_success', { name }));
  }

  async function handleUninstall() {
    if (!uninstallTarget) return;
    const name = uninstallTarget.name;
    setUninstalling(true);
    try {
      await api.uninstallMarketplaceServer(uninstallTarget.id);
      setUninstallTarget(null);
      refetch();
      showToast('success', t('marketplace.uninstall_success', { name }));
    } catch (err) {
      setUninstallTarget(null);
      showToast('error', extractError(err, t('marketplace.uninstall_error')));
    } finally {
      setUninstalling(false);
    }
  }

  // Loading state
  if (isLoading && servers.length === 0) {
    return (
      <div className="flex items-center justify-center h-64 text-content-tertiary">
        <span className="text-[10px] font-mono">{t('marketplace.loading')}</span>
      </div>
    );
  }

  // Error state
  if (error) {
    return (
      <div className="flex flex-col items-center justify-center h-64 gap-3">
        <AlertTriangle size={24} className="text-red-500 opacity-60" />
        <span className="text-[10px] font-mono text-content-tertiary">{t('marketplace.error')}</span>
        <button
          onClick={refetch}
          className="px-3 py-1.5 text-[10px] font-mono rounded bg-brand/10 hover:bg-brand/20 text-brand border border-brand/30 transition-colors"
        >
          {t('marketplace.refresh')}
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {/* Search and filters */}
      <div className="flex items-center gap-3 flex-wrap">
        {/* Search input */}
        <div className="relative flex-1 min-w-[200px]">
          <Search size={12} className="absolute left-2 top-1/2 -translate-y-1/2 text-content-tertiary" />
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={t('marketplace.search_placeholder')}
            className="w-full text-[12px] font-sans bg-glass border border-edge rounded pl-7 pr-2 py-1.5 text-content-primary placeholder:text-content-tertiary"
          />
        </div>

        {/* Category filters */}
        <div className="flex items-center gap-1">
          {CATEGORIES.map((cat) => (
            <button
              key={cat.key}
              onClick={() => setCategoryFilter(cat.key)}
              className={`px-2 py-1 text-[11px] font-sans uppercase rounded transition-colors ${
                categoryFilter === cat.key
                  ? 'bg-brand/10 text-brand border border-brand/30'
                  : 'text-content-tertiary hover:text-content-secondary hover:bg-glass border border-transparent'
              }`}
            >
              {cat.key === 'all' ? t('marketplace.filter_all') : cat.label}
            </button>
          ))}
        </div>

        {/* Dev mode: marketplace actions toggle */}
        {IS_DEV && (
          <button
            onClick={() => setActionsEnabled((v) => !v)}
            className={`flex items-center gap-1 px-2 py-1 text-[10px] font-mono rounded border transition-colors ${
              actionsEnabled
                ? 'bg-amber-500/10 text-amber-500 border-amber-500/30'
                : 'bg-glass text-content-tertiary border-edge'
            }`}
            title={actionsEnabled ? 'Marketplace actions enabled' : 'Marketplace actions locked (dev mode)'}
          >
            {actionsEnabled ? <Unlock size={10} /> : <Lock size={10} />}
            {actionsEnabled ? 'UNLOCKED' : 'DEV LOCKED'}
          </button>
        )}
      </div>

      {/* Grid */}
      {filtered.length === 0 ? (
        <div className="flex items-center justify-center h-48 text-content-tertiary">
          <span className="text-[10px] font-mono">{t('marketplace.no_results')}</span>
        </div>
      ) : (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-3">
          {filtered.map((entry) => (
            <MarketplaceCard
              key={entry.id}
              entry={entry}
              onInstall={setInstallingServer}
              onUninstall={setUninstallTarget}
              actionsDisabled={!actionsEnabled}
            />
          ))}
        </div>
      )}

      {/* Installs the catalog no longer lists. Rendered below the grid because
          it is a footnote about what is already on the machine, not part of
          what is on offer. */}
      {unlisted.length > 0 && (
        <div className="flex flex-col gap-2 rounded-lg border border-amber-500/30 bg-amber-500/5 p-3">
          <div className="flex items-center gap-2 text-amber-400">
            <PackageX size={12} />
            <span className="text-[11px] font-mono">{t('marketplace.unlisted_title')}</span>
          </div>
          <p className="text-[10px] font-sans text-content-tertiary">{t('marketplace.unlisted_hint')}</p>
          <div className="flex flex-col gap-1">
            {unlisted.map((item) => (
              <div
                key={item.name}
                className="flex items-center justify-between gap-3 rounded border border-border-subtle px-2 py-1.5"
              >
                <div className="flex items-center gap-2 min-w-0">
                  <span className="text-[11px] font-mono truncate">{item.name}</span>
                  {item.installed_version && (
                    <span className="text-[10px] font-mono text-content-tertiary">{item.installed_version}</span>
                  )}
                  {item.running && (
                    <span className="text-[9px] font-mono text-emerald-400">{t('marketplace.unlisted_running')}</span>
                  )}
                </div>
                <button
                  type="button"
                  disabled={!actionsEnabled}
                  onClick={() => setUninstallTarget({ id: item.name, name: item.name })}
                  className="text-[10px] font-mono text-red-400 hover:text-red-300 disabled:opacity-40"
                >
                  {t('marketplace.uninstall')}
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Install dialog */}
      {installingServer && (
        <InstallDialog
          entry={installingServer}
          onClose={() => setInstallingServer(null)}
          onInstalled={handleInstalled}
        />
      )}

      {/* Uninstall confirmation */}
      <ConfirmDialog
        open={!!uninstallTarget}
        title={t('marketplace.uninstall')}
        message={uninstallTarget ? t('marketplace.uninstall_confirm', { name: uninstallTarget.name }) : ''}
        confirmLabel={uninstalling ? '...' : t('marketplace.uninstall')}
        variant="danger"
        onConfirm={handleUninstall}
        onCancel={() => {
          setUninstallTarget(null);
        }}
      />

      {/* Toast notification */}
      {toast && (
        <div
          className={`fixed bottom-4 right-4 z-[60] text-[11px] font-sans px-3 py-2 rounded-lg backdrop-blur-sm border ${
            toast.type === 'success'
              ? 'bg-emerald-500/10 border-emerald-500/30 text-emerald-400'
              : 'bg-red-500/10 border-red-500/30 text-red-500'
          }`}
        >
          {toast.message}
        </div>
      )}
    </div>
  );
}
