import { AlertTriangle, Search } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { useMarketplace } from '../../hooks/useMarketplace';
import { extractError } from '../../lib/errors';
import type { MarketplaceCatalogEntry } from '../../types';
import { ConfirmDialog } from '../ui/ConfirmDialog';
import { InstallDialog } from './InstallDialog';
import { MarketplaceCard } from './MarketplaceCard';

const CATEGORIES: Array<{ key: string; label: string }> = [
  { key: 'all', label: 'filter_all' },
  { key: 'mind', label: 'mind' },
  { key: 'memory', label: 'memory' },
  { key: 'tool', label: 'tool' },
  { key: 'vision', label: 'vision' },
  { key: 'voice', label: 'voice' },
  { key: 'output', label: 'output' },
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

  const [searchQuery, setSearchQuery] = useState('');
  const [categoryFilter, setCategoryFilter] = useState('all');
  const [installingServer, setInstallingServer] = useState<MarketplaceCatalogEntry | null>(null);
  const [uninstallTarget, setUninstallTarget] = useState<MarketplaceCatalogEntry | null>(null);
  const [uninstalling, setUninstalling] = useState(false);
  const [uninstallError, setUninstallError] = useState<string | null>(null);
  const api = useApi();

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

  function handleInstalled() {
    setInstallingServer(null);
    refetch();
  }

  async function handleUninstall() {
    if (!uninstallTarget) return;
    setUninstalling(true);
    setUninstallError(null);
    try {
      await api.uninstallMarketplaceServer(uninstallTarget.id);
      setUninstallTarget(null);
      refetch();
    } catch (err) {
      setUninstallTarget(null);
      setUninstallError(extractError(err, t('marketplace.uninstall_error')));
      setTimeout(() => setUninstallError(null), 5000);
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
            />
          ))}
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
          setUninstallError(null);
        }}
      />

      {/* Uninstall error */}
      {uninstallError && (
        <div className="fixed bottom-4 right-4 z-[60] bg-red-500/10 border border-red-500/30 text-red-500 text-[11px] font-sans px-3 py-2 rounded-lg backdrop-blur-sm">
          {uninstallError}
        </div>
      )}
    </div>
  );
}
