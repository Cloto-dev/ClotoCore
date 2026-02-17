import { useState, useRef, useEffect } from 'react';
import { memo } from 'react';
import { X } from 'lucide-react';

interface GlassWindowProps {
  id: string;
  title: string;
  initialPosition: { x: number, y: number };
  children: any;
  onClose: () => void;
  onFocus: () => void;
  zIndex: number;
}

export const GlassWindow = memo(function GlassWindow({ title, initialPosition, children, onClose, onFocus, zIndex }: GlassWindowProps) {
  const [position, setPosition] = useState(initialPosition);
  const windowRef = useRef<HTMLDivElement>(null);
  const isDragging = useRef(false);
  const dragOffset = useRef({ x: 0, y: 0 });

  const handleMouseDown = (e: React.MouseEvent) => {
    isDragging.current = true;
    dragOffset.current = {
      x: e.clientX - position.x,
      y: e.clientY - position.y
    };
    
    // Performance: Disable blur during drag via direct DOM to avoid render
    if (windowRef.current) {
      windowRef.current.classList.remove('backdrop-blur-sm');
      windowRef.current.classList.add('backdrop-blur-none');
      windowRef.current.style.transition = 'none'; // Disable transitions during drag
    }
    
    if (onFocus) onFocus();
  };

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (isDragging.current && windowRef.current) {
        const x = e.clientX - dragOffset.current.x;
        const y = e.clientY - dragOffset.current.y;
        
        windowRef.current.style.left = `${x}px`;
        windowRef.current.style.top = `${y}px`;
      }
    };
    const handleMouseUp = (e: MouseEvent) => {
      if (isDragging.current) {
        isDragging.current = false;
        
        // Restore styles
        if (windowRef.current) {
          windowRef.current.classList.add('backdrop-blur-sm');
          windowRef.current.classList.remove('backdrop-blur-none');
          windowRef.current.style.transition = ''; // Restore transitions
        }

        const x = e.clientX - dragOffset.current.x;
        const y = e.clientY - dragOffset.current.y;
        setPosition({ x, y });
      }
    };

    window.addEventListener('mousemove', handleMouseMove);
    window.addEventListener('mouseup', handleMouseUp);
    return () => {
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
    };
  }, []); // Use empty dependency array as we use refs for tracking state in listeners
  
  return (
    <div 
      ref={windowRef}
      className="absolute flex flex-col bg-white/50 backdrop-blur-sm border border-white/40 rounded-lg shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-200"
      style={{ 
        left: position.x, 
        top: position.y, 
        width: '800px', 
        height: '500px',
        zIndex 
      }}
      onMouseDown={onFocus}
    >
      {/* Title Bar */}
      <div 
        className="h-8 bg-gradient-to-r from-surface-secondary/50 to-surface-primary/50 border-b border-surface-primary/20 flex items-center justify-between px-3 select-none"
        onMouseDown={handleMouseDown}
      >
        <div className="flex items-center gap-2">
           <span className="w-2 h-2 rounded-full bg-content-muted"></span>
           <span className="text-[10px] font-black text-content-secondary uppercase tracking-widest">{title}</span>
        </div>
        <button 
          onClick={(e) => { e.stopPropagation(); onClose(); }} 
          className="p-1 hover:bg-red-500 hover:text-white rounded-full transition-colors text-content-tertiary"
        >
          <X size={12} />
        </button>
      </div>
      
      {/* Content Area */}
      <div className="flex-1 relative overflow-hidden bg-transparent">
        {children}
      </div>
    </div>
  );
});
