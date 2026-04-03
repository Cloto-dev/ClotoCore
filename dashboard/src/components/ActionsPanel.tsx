import { ChevronLeft, PanelRightClose } from 'lucide-react';
import type { ActionCategory, Artifact, DialogueTab, ExternalActionTab } from '../hooks/useActions';
import { CodeBlock } from './CodeBlock';
import { DialogueCard } from './DialogueCard';
import { ExternalActionCard } from './ExternalActionCard';

interface ActionsPanelProps {
  isOpen: boolean;
  onClose: () => void;
  onOpen: () => void;
  activeCategory: ActionCategory;
  onCategoryChange: (cat: ActionCategory) => void;
  hasDialogues: boolean;
  hasExternalActions: boolean;
  artifacts: Artifact[];
  activeArtifactIndex: number;
  onArtifactTabChange: (index: number) => void;
  dialogues: DialogueTab[];
  externalActions: ExternalActionTab[];
  unreadDialogueCount: number;
  unreadExternalCount: number;
  totalCount: number;
}

function getLabel(code: string): string {
  const lines = code.split('\n');
  const firstNonEmpty = lines.find((l) => l.trim() && !l.trim().startsWith('//') && !l.trim().startsWith('#'));
  if (!firstNonEmpty) return 'snippet';
  const trimmed = firstNonEmpty.trim();
  return trimmed.length > 37 ? `${trimmed.slice(0, 34)}...` : trimmed;
}

export function ActionsPanel({
  isOpen,
  onClose,
  onOpen,
  activeCategory,
  onCategoryChange,
  hasDialogues,
  hasExternalActions,
  artifacts,
  activeArtifactIndex,
  onArtifactTabChange,
  dialogues,
  externalActions,
  unreadDialogueCount,
  unreadExternalCount,
  totalCount,
}: ActionsPanelProps) {
  if (totalCount === 0) return null;

  const active = artifacts[activeArtifactIndex] || artifacts[0];
  const showCategoryBar = hasDialogues || hasExternalActions;
  const totalUnread = unreadDialogueCount + unreadExternalCount;

  // Collapsed state
  if (!isOpen) {
    return (
      <button
        onClick={onOpen}
        className="h-full w-8 shrink-0 border-l border-edge bg-surface-primary/50 backdrop-blur-sm hover:bg-glass-strong flex flex-col items-center justify-center gap-2 transition-colors group"
        title="Open Actions"
      >
        <ChevronLeft size={12} className="text-content-tertiary group-hover:text-brand transition-colors" />
        <span className="text-[9px] font-black uppercase tracking-widest text-content-tertiary group-hover:text-content-secondary [writing-mode:vertical-rl] rotate-180">
          Actions
        </span>
        <div className="flex flex-col items-center gap-1">
          <span className="text-[9px] font-mono text-brand/70">{totalCount}</span>
          {totalUnread > 0 && <span className="w-1.5 h-1.5 rounded-full bg-brand animate-pulse" />}
        </div>
      </button>
    );
  }

  return (
    <div
      className="h-full bg-surface-primary/50 backdrop-blur-sm border-l border-edge flex flex-col"
      style={{ width: '480px', maxWidth: '50vw', minWidth: '320px' }}
    >
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-edge/50 shrink-0">
        <div className="flex items-center gap-2">
          <span className="text-xs font-black uppercase tracking-widest text-content-primary">Actions</span>
          <span className="text-[9px] font-mono text-content-tertiary">{totalCount}</span>
        </div>
        <button
          onClick={onClose}
          className="p-1.5 rounded-md hover:bg-surface-secondary text-content-tertiary hover:text-content-primary transition-all"
          title="Collapse"
        >
          <PanelRightClose size={14} />
        </button>
      </div>

      {/* Category bar — show when dialogues or external actions exist */}
      {showCategoryBar && (
        <div className="flex border-b border-edge/50 shrink-0">
          <button
            onClick={() => onCategoryChange('code')}
            className={`flex-1 px-3 py-2 text-[10px] font-bold uppercase tracking-wider transition-all border-b-2 ${
              activeCategory === 'code'
                ? 'border-brand text-content-primary'
                : 'border-transparent text-content-tertiary hover:text-content-secondary'
            }`}
          >
            Code
            {artifacts.length > 0 && <span className="ml-1.5 text-[9px] font-mono opacity-60">{artifacts.length}</span>}
          </button>
          {hasDialogues && (
            <button
              onClick={() => onCategoryChange('dialogues')}
              className={`flex-1 px-3 py-2 text-[10px] font-bold uppercase tracking-wider transition-all border-b-2 relative ${
                activeCategory === 'dialogues'
                  ? 'border-brand text-content-primary'
                  : 'border-transparent text-content-tertiary hover:text-content-secondary'
              }`}
            >
              Dialogues
              <span className="ml-1.5 text-[9px] font-mono opacity-60">{dialogues.length}</span>
              {unreadDialogueCount > 0 && activeCategory !== 'dialogues' && (
                <span className="absolute top-1.5 right-2 w-1.5 h-1.5 rounded-full bg-brand animate-pulse" />
              )}
            </button>
          )}
          {hasExternalActions && (
            <button
              onClick={() => onCategoryChange('external')}
              className={`flex-1 px-3 py-2 text-[10px] font-bold uppercase tracking-wider transition-all border-b-2 relative ${
                activeCategory === 'external'
                  ? 'border-brand text-content-primary'
                  : 'border-transparent text-content-tertiary hover:text-content-secondary'
              }`}
            >
              External
              <span className="ml-1.5 text-[9px] font-mono opacity-60">{externalActions.length}</span>
              {unreadExternalCount > 0 && activeCategory !== 'external' && (
                <span className="absolute top-1.5 right-2 w-1.5 h-1.5 rounded-full bg-brand animate-pulse" />
              )}
            </button>
          )}
        </div>
      )}

      {/* Content: Code */}
      {activeCategory === 'code' && artifacts.length > 0 && (
        <>
          {/* Artifact tab bar */}
          {artifacts.length > 1 && (
            <div className="flex border-b border-edge overflow-x-auto no-scrollbar shrink-0">
              {artifacts.map((artifact, i) => (
                <button
                  key={artifact.id}
                  onClick={() => onArtifactTabChange(i)}
                  className={`px-3 py-2 text-[10px] font-mono whitespace-nowrap transition-all border-b-2 ${
                    i === activeArtifactIndex
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
          {active && (
            <div className="flex-1 overflow-y-auto no-scrollbar p-2">
              <CodeBlock code={active.code} language={active.language} showHeader={true} className="h-full" />
            </div>
          )}
        </>
      )}

      {/* Content: Code empty state (when on code tab but no artifacts) */}
      {activeCategory === 'code' && artifacts.length === 0 && (
        <div className="flex-1 flex items-center justify-center">
          <span className="text-[10px] text-content-tertiary">No code artifacts yet</span>
        </div>
      )}

      {/* Content: Dialogues — vertical scroll list, newest first */}
      {activeCategory === 'dialogues' && dialogues.length > 0 && (
        <div className="flex-1 overflow-y-auto no-scrollbar p-3 space-y-3">
          {[...dialogues].reverse().map((tab) => (
            <DialogueCard key={tab.dialogue.dialogue_id} dialogue={tab.dialogue} />
          ))}
        </div>
      )}

      {/* Content: External Actions — vertical scroll list, newest first */}
      {activeCategory === 'external' && externalActions.length > 0 && (
        <div className="flex-1 overflow-y-auto no-scrollbar p-3 space-y-3">
          {[...externalActions].reverse().map((tab) => (
            <ExternalActionCard key={tab.action.action_id} action={tab.action} />
          ))}
        </div>
      )}
    </div>
  );
}
