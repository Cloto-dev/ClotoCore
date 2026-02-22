import { memo } from 'react';

interface HistoryItem {
  v: string;
  t: string;
  d: string;
  active: boolean;
}

const HISTORY_DATA: HistoryItem[] = [
  { v: 'KS2.1', t: '最適化', d: '高速演算エンジンの導入と基盤の堅牢化。より安定した知能へ。', active: true },
  { v: 'KS2.0', t: '進化', d: 'ニューラル・エピソード記憶とRAGの統合。長期記憶の獲得。', active: false },
  { v: 'KS1.0', t: '始動', d: '初期対話エンジンの構築。[redacted]としての第一歩。', active: false },
];

export const SystemHistory = memo(function SystemHistory() {
  return (
    <footer className="mt-20 pt-8 border-t border-edge flex flex-col items-end">
      <div className="max-w-xs w-full space-y-6">
        <div className="flex items-center justify-end gap-2 text-content-tertiary mb-4">
          <span className="text-[10px] font-black uppercase tracking-[0.3em]">System Evolution History</span>
          <div className="h-[1px] w-8 bg-surface-secondary" />
        </div>
        
        <div className="relative space-y-8 pr-6">
          {/* Connection Line */}
          <div className="absolute right-[5px] top-2 bottom-2 w-[1px] bg-surface-secondary" />

          {HISTORY_DATA.map((item) => (
            <div key={item.v} className="relative text-right group">
              <div className="flex flex-col items-end">
                <div className="flex items-center gap-3 mb-1">
                  <span className={`text-[10px] font-mono font-bold ${item.active ? 'text-brand' : 'text-content-tertiary'}`}>
                    {item.v}
                  </span>
                  <span className={`text-xs font-black ${item.active ? 'text-content-primary' : 'text-content-secondary'}`}>
                    {item.t}
                  </span>
                </div>
                <p className="text-[10px] leading-relaxed text-content-tertiary font-medium max-w-[200px]">
                  {item.d}
                </p>
              </div>
              {/* Timeline Dot */}
              <div 
                className={`absolute -right-[27px] top-1.5 w-3 h-3 rounded-full border-2 bg-surface-primary transition-all duration-500 ${
                  item.active
                    ? 'border-brand shadow-[0_0_10px_rgba(46,77,230,0.4)] scale-110'
                    : 'border-content-muted group-hover:border-content-tertiary'
                }`}
              >
                {item.active && (
                  <span className="absolute inset-0 rounded-full bg-brand animate-ping opacity-20" />
                )}
              </div>
            </div>
          ))}
        </div>
      </div>
      
      <div className="mt-12 text-[9px] font-mono text-content-muted uppercase tracking-widest">
        &copy; 2026 Exiv Project
      </div>
    </footer>
  );
});
