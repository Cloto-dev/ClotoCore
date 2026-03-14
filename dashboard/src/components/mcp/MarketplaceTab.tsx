import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { RefreshCw, Search, AlertTriangle } from 'lucide-react';
import { useMarketplace } from '../../hooks/useMarketplace';
import { MarketplaceCard } from './MarketplaceCard';
import { InstallDialog } from './InstallDialog';
import { MarketplaceCatalogEntry } from '../../types';

const CATEGORIES: Array<{ key: string; label: string }> = [
  { key: 'all', label: 'filter_all' },
  { key: 'mind', label: 'mind' },
  { key: 'memory', label: 'memory' },
  { key: 'tool', label: 'tool' },
  { key: 'vision', label: 'vision' },
  { key: 'voice', label: 'voice' },
  { key: 'output', label: 'output' },
];

export function MarketplaceTab() {
  const { t } = useTranslation('mcp');
  const { servers, isLoading, error, refetch } = useMarketplace();

  const [searchQuery, setSearchQuery] = useState('');
  const [categoryFilter, setCategoryFilter] = useState('all');
  const [installingServer, setInstallingServer] = useState<MarketplaceCatalogEntry | null>(null);

  // Filter servers
  const filtered = servers.filter(s => {
    // Category filter
    if (categoryFilter !== 'all' && s.category !== categoryFilter) return false;
    // Search filter
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      return (
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        s.tags.some(tag => tag.toLowerCase().includes(q))
      );
    }
    return true;
  });

  function handleInstalled() {
    setInstallingServer(null);
    refetch();
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
            onChange={e => setSearchQuery(e.target.value)}
            placeholder={t('marketplace.search_placeholder')}
            className="w-full text-[10px] font-mono bg-glass border border-edge rounded pl-7 pr-2 py-1.5 text-content-primary placeholder:text-content-tertiary"
          />
        </div>

        {/* Category filters */}
        <div className="flex items-center gap-1">
          {CATEGORIES.map(cat => (
            <button
              key={cat.key}
              onClick={() => setCategoryFilter(cat.key)}
              className={`px-2 py-1 text-[9px] font-mono uppercase rounded transition-colors ${
                categoryFilter === cat.key
                  ? 'bg-brand/10 text-brand border border-brand/30'
                  : 'text-content-tertiary hover:text-content-secondary hover:bg-glass border border-transparent'
              }`}
            >
              {cat.key === 'all' ? t('marketplace.filter_all') : cat.label}
            </button>
          ))}
        </div>

        {/* Refresh button */}
        <button
          onClick={refetch}
          className="p-1.5 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors"
          title={t('marketplace.refresh')}
        >
          <RefreshCw size={14} className={isLoading ? 'animate-spin' : ''} />
        </button>
      </div>

      {/* Grid */}
      {filtered.length === 0 ? (
        <div className="flex items-center justify-center h-48 text-content-tertiary">
          <span className="text-[10px] font-mono">{t('marketplace.no_results')}</span>
        </div>
      ) : (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-3">
          {filtered.map(entry => (
            <MarketplaceCard
              key={entry.id}
              entry={entry}
              onInstall={setInstallingServer}
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
    </div>
  );
}
