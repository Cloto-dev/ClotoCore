import { ChevronLeft, PanelRightClose } from 'lucide-react';
import type { Artifact } from '../hooks/useArtifacts';
import { CodeBlock } from './CodeBlock';

interface ArtifactPanelProps {
  artifacts: Artifact[];
  activeIndex: number;
  onTabChange: (index: number) => void;
  isOpen: boolean;
  onClose: () => void;
  onOpen: () => void;
}

function getLabel(code: string): string {
  const lines = code.split('\n');
  const firstNonEmpty = lines.find((l) => l.trim() && !l.trim().startsWith('//') && !l.trim().startsWith('#'));
  if (!firstNonEmpty) return 'snippet';
  const trimmed = firstNonEmpty.trim();
  return trimmed.length > 37 ? trimmed.slice(0, 34) + '...' : trimmed;
}

export function ArtifactPanel({ artifacts, activeIndex, onTabChange, isOpen, onClose, onOpen }: ArtifactPanelProps) {
  if (artifacts.length === 0) return null;

  const active = artifacts[activeIndex] || artifacts[0];

  // Collapsed state: show a thin vertical tab to re-open
  if (!isOpen) {
    return (
      <button
        onClick={onOpen}
        className="h-full w-8 shrink-0 border-l border-edge bg-surface-primary hover:bg-glass-strong flex flex-col items-center justify-center gap-2 transition-colors group"
        title="Open Artifacts"
      >
        <ChevronLeft size={12} className="text-content-tertiary group-hover:text-brand transition-colors" />
        <span className="text-[9px] font-black uppercase tracking-widest text-content-tertiary group-hover:text-content-secondary [writing-mode:vertical-rl] rotate-180">
          Artifacts
        </span>
        <span className="text-[9px] font-mono text-brand/70">{artifacts.length}</span>
      </button>
    );
  }

  return (
    <div
      className="h-full bg-surface-primary border-l border-edge flex flex-col"
      style={{
        width: '480px',
        maxWidth: '50vw',
        minWidth: '320px',
      }}
    >
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-edge bg-glass-strong shrink-0">
        <div className="flex items-center gap-2">
          <span className="text-xs font-black uppercase tracking-widest text-content-primary">Artifacts</span>
          <span className="text-[9px] font-mono text-content-tertiary">{artifacts.length}</span>
        </div>
        <button
          onClick={onClose}
          className="p-1.5 rounded-md hover:bg-surface-secondary text-content-tertiary hover:text-content-primary transition-all"
          title="Collapse"
        >
          <PanelRightClose size={14} />
        </button>
      </div>

      {/* Tab bar */}
      {artifacts.length > 1 && (
        <div className="flex border-b border-edge overflow-x-auto no-scrollbar shrink-0">
          {artifacts.map((artifact, i) => (
            <button
              key={artifact.id}
              onClick={() => onTabChange(i)}
              className={`px-3 py-2 text-[10px] font-mono whitespace-nowrap transition-all border-b-2 ${
                i === activeIndex
                  ? 'border-brand text-content-primary'
                  : 'border-transparent text-content-tertiary hover:text-content-secondary'
              }`}
            >
              <span className="uppercase font-bold tracking-wider mr-1.5">{artifact.language}</span>
              <span className="opacity-60">{getLabel(artifact.code)}</span>
            </button>
          ))}
        </div>
      )}

      {/* Code content */}
      <div className="flex-1 overflow-y-auto no-scrollbar p-2">
        <CodeBlock code={active.code} language={active.language} showHeader={true} className="h-full" />
      </div>
    </div>
  );
}
