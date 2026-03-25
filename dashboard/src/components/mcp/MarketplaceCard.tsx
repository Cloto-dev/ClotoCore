import {
  BookOpen,
  Box,
  Brain,
  Clock,
  Cpu,
  Eye,
  Image,
  Layers,
  type LucideIcon,
  MessageCircle,
  Mic,
  Monitor,
  Package,
  Search,
  Sparkles,
  Terminal,
  Wrench,
  Zap,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { MarketplaceCatalogEntry } from '../../types';

const ICON_MAP: Record<string, LucideIcon> = {
  terminal: Terminal,
  wrench: Wrench,
  clock: Clock,
  layers: Layers,
  search: Search,
  'book-open': BookOpen,
  image: Image,
  cpu: Cpu,
  zap: Zap,
  sparkles: Sparkles,
  box: Box,
  brain: Brain,
  monitor: Monitor,
  eye: Eye,
  mic: Mic,
  'message-circle': MessageCircle,
};

interface MarketplaceCardProps {
  entry: MarketplaceCatalogEntry;
  onInstall: (entry: MarketplaceCatalogEntry) => void;
  onUninstall: (entry: MarketplaceCatalogEntry) => void;
}

export function MarketplaceCard({ entry, onInstall, onUninstall }: MarketplaceCardProps) {
  const { t } = useTranslation('mcp');
  const Icon = (entry.icon && ICON_MAP[entry.icon]) || Package;

  const alreadyPresent = entry.installed || entry.running;
  const isInstalled = alreadyPresent && !entry.update_available;
  const isUpdate = entry.installed && entry.update_available;

  return (
    <div className="bg-surface-primary/50 border border-edge rounded-xl p-4 flex flex-col gap-2 transition-all duration-200 hover:border-brand group">
      {/* Header */}
      <div className="flex items-center gap-2">
        <Icon size={14} className="text-brand shrink-0" />
        <span className="text-[13px] font-sans font-bold text-content-primary truncate">{entry.name}</span>
        {entry.runtime === 'rust' && (
          <span className="text-[9px] font-mono px-1 rounded bg-orange-500/15 text-orange-400 border border-orange-500/25 shrink-0">
            Rust
          </span>
        )}
        <span className="ml-auto text-[11px] font-sans px-1.5 rounded bg-surface-secondary text-content-tertiary uppercase shrink-0">
          {entry.category}
        </span>
      </div>

      {/* Description */}
      <p className="text-[11px] font-sans text-content-tertiary line-clamp-2 leading-relaxed">{entry.description}</p>

      {/* Tags */}
      {entry.tags.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {entry.tags.map((tag) => (
            <span
              key={tag}
              className="bg-surface-secondary text-content-tertiary text-[11px] px-1.5 rounded border border-edge"
            >
              {tag}
            </span>
          ))}
        </div>
      )}

      {/* Footer */}
      <div className="flex items-center justify-between mt-auto pt-1">
        <div className="flex items-center gap-2">
          {/* Status indicator */}
          {isInstalled && (
            <span className="flex items-center gap-1 text-[10px] font-sans text-emerald-500">
              <span className="w-1.5 h-1.5 rounded-full bg-emerald-500" />
              {t('marketplace.installed')}
            </span>
          )}
          {isUpdate && (
            <span className="flex items-center gap-1 text-[10px] font-sans text-amber-500">
              <span className="w-1.5 h-1.5 rounded-full bg-amber-500" />
              {t('marketplace.update_available')}
            </span>
          )}
          {!alreadyPresent && (
            <span className="flex items-center gap-1 text-[10px] font-sans text-content-tertiary">
              <span className="w-1.5 h-1.5 rounded-full bg-content-tertiary/40" />
              {t('marketplace.not_installed')}
            </span>
          )}
          <span className="text-[11px] font-sans text-content-tertiary">v{entry.version}</span>
        </div>

        {/* Action button */}
        {isInstalled && entry.installed && (
          <button
            onClick={() => onUninstall(entry)}
            className="px-2 py-1 text-[11px] font-sans rounded bg-red-500/10 hover:bg-red-500/20 text-red-500 border border-red-500/30 transition-colors"
          >
            {t('marketplace.uninstall')}
          </button>
        )}
        {isInstalled && !entry.installed && (
          <button
            disabled
            className="px-2 py-1 text-[11px] font-sans rounded bg-glass text-content-tertiary border border-edge cursor-default"
          >
            {t('marketplace.installed')}
          </button>
        )}
        {isUpdate && (
          <button
            onClick={() => onInstall(entry)}
            className="px-2 py-1 text-[11px] font-sans rounded bg-amber-500/10 hover:bg-amber-500/20 text-amber-500 border border-amber-500/30 transition-colors"
          >
            {t('marketplace.update_available')}
          </button>
        )}
        {!alreadyPresent && (
          <button
            onClick={() => onInstall(entry)}
            className="px-2 py-1 text-[11px] font-sans rounded bg-brand/10 hover:bg-brand/20 text-brand border border-brand/30 transition-colors"
          >
            {t('marketplace.install')}
          </button>
        )}
      </div>
    </div>
  );
}
