import { useTranslation } from 'react-i18next';
import { MarketplaceCatalogEntry } from '../../types';
import {
  Terminal, Wrench, Clock, Layers, Search, BookOpen, Image, Cpu,
  Zap, Sparkles, Box, Brain, Monitor, Eye, Mic, Package,
  type LucideIcon,
} from 'lucide-react';

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
};

interface MarketplaceCardProps {
  entry: MarketplaceCatalogEntry;
  onInstall: (entry: MarketplaceCatalogEntry) => void;
}

export function MarketplaceCard({ entry, onInstall }: MarketplaceCardProps) {
  const { t } = useTranslation('mcp');
  const Icon = (entry.icon && ICON_MAP[entry.icon]) || Package;

  const isInstalled = entry.installed && !entry.update_available;
  const isUpdate = entry.installed && entry.update_available;

  return (
    <div className="bg-glass border border-edge rounded-lg p-3 flex flex-col gap-2">
      {/* Header */}
      <div className="flex items-center gap-2">
        <Icon size={14} className="text-brand shrink-0" />
        <span className="text-[11px] font-mono font-bold text-content-primary truncate">{entry.name}</span>
        <span className="ml-auto text-[9px] font-mono px-1.5 rounded bg-surface-secondary text-content-tertiary uppercase shrink-0">
          {entry.category}
        </span>
      </div>

      {/* Description */}
      <p className="text-[10px] font-mono text-content-tertiary line-clamp-2 leading-relaxed">
        {entry.description}
      </p>

      {/* Tags */}
      {entry.tags.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {entry.tags.map(tag => (
            <span key={tag} className="bg-surface-secondary text-content-tertiary text-[9px] px-1.5 rounded">
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
            <span className="flex items-center gap-1 text-[10px] font-mono text-emerald-500">
              <span className="w-1.5 h-1.5 rounded-full bg-emerald-500" />
              {t('marketplace.installed')}
            </span>
          )}
          {isUpdate && (
            <span className="flex items-center gap-1 text-[10px] font-mono text-amber-500">
              <span className="w-1.5 h-1.5 rounded-full bg-amber-500" />
              {t('marketplace.update_available')}
            </span>
          )}
          {!entry.installed && (
            <span className="flex items-center gap-1 text-[10px] font-mono text-content-tertiary">
              <span className="w-1.5 h-1.5 rounded-full bg-content-tertiary/40" />
              {t('marketplace.not_installed')}
            </span>
          )}
          <span className="text-[9px] font-mono text-content-tertiary">v{entry.version}</span>
        </div>

        {/* Action button */}
        {isInstalled && (
          <button
            disabled
            className="px-2 py-1 text-[10px] font-mono rounded bg-glass text-content-tertiary border border-edge cursor-default"
          >
            Installed
          </button>
        )}
        {isUpdate && (
          <button
            onClick={() => onInstall(entry)}
            className="px-2 py-1 text-[10px] font-mono rounded bg-amber-500/10 hover:bg-amber-500/20 text-amber-500 border border-amber-500/30 transition-colors"
          >
            {t('marketplace.update_available')}
          </button>
        )}
        {!entry.installed && (
          <button
            onClick={() => onInstall(entry)}
            className="px-2 py-1 text-[10px] font-mono rounded bg-brand/10 hover:bg-brand/20 text-brand border border-brand/30 transition-colors"
          >
            {t('marketplace.install')}
          </button>
        )}
      </div>
    </div>
  );
}
