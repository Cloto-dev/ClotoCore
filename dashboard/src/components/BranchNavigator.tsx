import { ChevronLeft, ChevronRight } from 'lucide-react';

interface BranchNavigatorProps {
  count: number;
  activeIndex: number;
  indices: number[];
  onNavigate: (branchIndex: number) => void;
}

export function BranchNavigator({ count, activeIndex, indices, onNavigate }: BranchNavigatorProps) {
  const currentPos = indices.indexOf(activeIndex);
  const displayPos = currentPos >= 0 ? currentPos + 1 : 1;

  return (
    <div className="flex items-center gap-1 py-0.5">
      <button
        onClick={() => {
          if (currentPos > 0) onNavigate(indices[currentPos - 1]);
        }}
        disabled={currentPos <= 0}
        className="p-0.5 rounded hover:bg-glass text-content-muted hover:text-content-secondary transition-colors disabled:opacity-30 disabled:cursor-default"
      >
        <ChevronLeft size={12} />
      </button>
      <span className="text-[10px] font-mono text-content-muted tabular-nums select-none">
        {displayPos}/{count}
      </span>
      <button
        onClick={() => {
          if (currentPos < indices.length - 1) onNavigate(indices[currentPos + 1]);
        }}
        disabled={currentPos >= indices.length - 1}
        className="p-0.5 rounded hover:bg-glass text-content-muted hover:text-content-secondary transition-colors disabled:opacity-30 disabled:cursor-default"
      >
        <ChevronRight size={12} />
      </button>
    </div>
  );
}
