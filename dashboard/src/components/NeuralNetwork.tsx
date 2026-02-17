import { useNeuralNetwork } from '../hooks/useNeuralNetwork';

export function NeuralNetwork({ mouseRef, events, onEventProcessed, seekTime }: { 
  mouseRef: React.MutableRefObject<{ x: number, y: number }>, 
  events: any[],
  onEventProcessed: (timestamp: number) => void,
  seekTime?: number | null
}) {
  const { canvasRef, selectedModal, setSelectedModal, nodes, longPressTimer, viewport } = useNeuralNetwork(mouseRef, events, onEventProcessed, seekTime);

  const activeNode = nodes.current.find(n => n.id === selectedModal?.nodeId);
  const coreNode = nodes.current.find(n => n.id === 'karin');

  const onModalMouseDown = (e: any) => {
    e.stopPropagation();
    if (longPressTimer.current) clearTimeout(longPressTimer.current);
    longPressTimer.current = window.setTimeout(() => {
      setSelectedModal(prev => prev ? { ...prev, isDragging: true } : null);
    }, 300);
  };

  // Transform node world coordinates to screen coordinates for the tooltip
  const screenX = activeNode ? activeNode.x * viewport.k + viewport.x : 0;
  const screenY = activeNode ? activeNode.y * viewport.k + viewport.y : 0;

  return (
    <>
      <canvas ref={canvasRef} className="absolute inset-0 z-10 cursor-crosshair" />
      
      {activeNode && selectedModal && coreNode && (
        <div 
          onMouseDown={onModalMouseDown}
          className={`absolute z-30 p-3 rounded-lg border bg-white/90 backdrop-blur-md shadow-xl w-48 pointer-events-auto transition-transform duration-200 ${selectedModal.isDragging ? 'shadow-2xl scale-105 cursor-grabbing' : 'cursor-grab'}`}
          style={{
            transform: `translate(${screenX + (activeNode.x > coreNode.x ? 40 : -232) + selectedModal.offsetX}px, ${screenY + (activeNode.y > coreNode.y ? 40 : -120) + selectedModal.offsetY}px)`,
            borderColor: activeNode.color,
            borderLeftWidth: '4px',
            left: 0, top: 0
          }}
        >
          <div className="flex justify-between items-start mb-2">
            <span className="text-[10px] font-black uppercase tracking-tighter" style={{ color: activeNode.color }}>{activeNode.label}</span>
            <span className="text-[8px] font-mono text-content-tertiary bg-surface-secondary px-1 rounded">{activeNode.data?.status}</span>
          </div>
          <div className="space-y-2">
            <div className="text-[9px] leading-tight text-content-secondary font-mono italic">"{activeNode.data?.log}"</div>
            <div className="text-[8px] text-content-tertiary flex justify-between border-t pt-1">
              <span>LATENCY: 42ms</span>
              <span>{activeNode.data?.lastActive}</span>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

