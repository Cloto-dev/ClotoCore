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
  ShieldAlert,
  ShieldCheck,
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

// Static class map (Tailwind is pre-compiled, so tone keys must resolve to
// fully-literal class strings — no `bg-${tone}-500/15` interpolation).
const BADGE_TONE = {
  orange: 'bg-orange-500/15 text-orange-400 border-orange-500/25',
  emerald: 'bg-emerald-500/15 text-emerald-400 border-emerald-500/25',
  amber: 'bg-amber-500/15 text-amber-400 border-amber-500/25',
} as const;

type BadgeTone = keyof typeof BADGE_TONE;

function Badge({
  icon: BadgeIcon,
  label,
  tone,
  title,
}: {
  icon?: LucideIcon;
  label: string;
  tone: BadgeTone;
  title?: string;
}) {
  return (
    <span
      title={title}
      className={`inline-flex items-center gap-1 text-[10px] font-mono px-1.5 py-0.5 rounded border ${BADGE_TONE[tone]}`}
    >
      {BadgeIcon && <BadgeIcon size={11} aria-hidden="true" />}
      {label}
    </span>
  );
}

interface MarketplaceCardProps {
  entry: MarketplaceCatalogEntry;
  onInstall: (entry: MarketplaceCatalogEntry) => void;
  onUninstall: (entry: MarketplaceCatalogEntry) => void;
  actionsDisabled?: boolean;
}

export function MarketplaceCard({ entry, onInstall, onUninstall, actionsDisabled }: MarketplaceCardProps) {
  const { t } = useTranslation('mcp');
  const Icon = (entry.icon && ICON_MAP[entry.icon]) || Package;

  const alreadyPresent = entry.installed || entry.running;
  const isInstalled = alreadyPresent && !entry.update_available;
  const isUpdate = entry.installed && entry.update_available;

  return (
    <div className="card-solid border border-edge rounded-xl p-4 flex flex-col gap-2 hover:border-brand group">
      {/* Header */}
      <div className="flex items-center gap-2">
        <Icon size={14} className="text-brand shrink-0" />
        <span className="text-[13px] font-sans font-bold text-content-primary truncate">{entry.name}</span>
        <span className="ml-auto text-[11px] font-sans px-1.5 rounded bg-surface-secondary text-content-tertiary uppercase shrink-0">
          {entry.category}
        </span>
      </div>

      {/* Badge row (runtime + Magic Seal verification) */}
      <div className="flex items-center gap-1.5 flex-wrap">
        {entry.runtime === 'rust' && <Badge label="Rust" tone="orange" />}
        {entry.seal ? (
          <Badge
            icon={ShieldCheck}
            label={t('marketplace.seal_verified')}
            tone="emerald"
            title={t('marketplace.seal_verified_tooltip')}
          />
        ) : (
          <Badge
            icon={ShieldAlert}
            label={t('marketplace.seal_unverified')}
            tone="amber"
            title={t('marketplace.seal_unverified_tooltip')}
          />
        )}
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
          {isUpdate && entry.changelog && (
            <p className="text-[10px] font-sans text-amber-500/70 line-clamp-2 leading-relaxed w-full">
              {entry.changelog}
            </p>
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
            disabled={actionsDisabled}
            className={`px-2 py-1 text-[11px] font-sans rounded border transition-colors ${
              actionsDisabled
                ? 'bg-glass text-content-tertiary border-edge cursor-not-allowed opacity-50'
                : 'bg-red-500/10 hover:bg-red-500/20 text-red-500 border-red-500/30'
            }`}
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
            disabled={actionsDisabled}
            className={`px-2 py-1 text-[11px] font-sans rounded border transition-colors ${
              actionsDisabled
                ? 'bg-glass text-content-tertiary border-edge cursor-not-allowed opacity-50'
                : 'bg-amber-500/10 hover:bg-amber-500/20 text-amber-500 border-amber-500/30'
            }`}
          >
            {t('marketplace.update_available')}
          </button>
        )}
        {!alreadyPresent && (
          <button
            onClick={() => onInstall(entry)}
            disabled={actionsDisabled}
            className={`px-2 py-1 text-[11px] font-sans rounded border transition-colors ${
              actionsDisabled
                ? 'bg-glass text-content-tertiary border-edge cursor-not-allowed opacity-50'
                : 'bg-brand/10 hover:bg-brand/20 text-brand border-brand/30'
            }`}
          >
            {t('marketplace.install')}
          </button>
        )}
      </div>
    </div>
  );
}
