import { useState, useEffect, useRef } from 'react';
import { ArrowLeft, Lock } from 'lucide-react';

interface LockScreenProps {
  onUnlock: () => void;
  onBack: () => void;
}

export function LockScreen({ onUnlock, onBack }: LockScreenProps) {
  const [val, setVal] = useState('');
  const [error, setError] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    // Auto-focus input
    inputRef.current?.focus();
    
    // Keep focus
    const handleClick = () => inputRef.current?.focus();
    window.addEventListener('click', handleClick);
    return () => window.removeEventListener('click', handleClick);
  }, []);

  useEffect(() => {
    if (val.length === 4) {
      const PASSCODE = import.meta.env.VITE_LOCK_PASSCODE || '1518';
      if (val === PASSCODE) {
        onUnlock();
      } else {
        setError(true);
        const timer = setTimeout(() => {
          setVal('');
          setError(false);
        }, 500);
        return () => clearTimeout(timer);
      }
    }
  }, [val, onUnlock]);

  return (
    <div className="absolute inset-0 z-50 flex flex-col items-center justify-center bg-transparent select-none">
       <button 
         onClick={onBack} 
         className="absolute top-6 left-6 p-2 rounded-full bg-white border border-slate-200 text-slate-400 hover:text-[#2e4de6] hover:border-[#2e4de6] transition-colors z-50"
       >
         <ArrowLeft size={20} />
       </button>

       <div className="flex flex-col items-center gap-6">
          <div className={`p-6 rounded-full bg-white border-2 transition-colors duration-300 ${error ? 'border-red-500 bg-red-50' : 'border-slate-200'}`}>
             <Lock size={40} className={`transition-colors duration-300 ${error ? 'text-red-500' : 'text-slate-400'}`} />
          </div>
          
          <div className="text-center">
            <div className="text-xs font-black tracking-[0.3em] text-slate-800 uppercase mb-1">Restricted Access</div>
            <div className="text-[10px] font-mono text-slate-400 tracking-widest uppercase">Encryption Level 4</div>
          </div>
          
          <div className="flex gap-4 my-4">
            {[0, 1, 2, 3].map(i => (
              <div 
                key={i} 
                className={`w-4 h-4 rounded-full border-2 transition-all duration-200 ${
                  val.length > i 
                    ? (error ? 'bg-red-500 border-red-500 scale-110' : 'bg-[#2e4de6] border-[#2e4de6] scale-110') 
                    : 'bg-transparent border-slate-300'
                }`} 
              />
            ))}
          </div>

          <input 
             ref={inputRef}
             type="text" 
             inputMode="numeric"
             pattern="[0-9]*"
             maxLength={4}
             value={val} 
             onInput={(e) => {
               const v = (e.target as HTMLInputElement).value.replace(/[^0-9]/g, '');
               if (v.length <= 4) setVal(v);
             }}
             className="opacity-0 absolute w-0 h-0"
             autoFocus
          />
          
          {error && <div className="text-xs text-red-500 font-mono font-bold animate-pulse tracking-widest">ACCESS DENIED</div>}
          <div className="text-[10px] text-slate-300 font-mono mt-8 uppercase tracking-widest">Enter Passcode</div>
       </div>
    </div>
  );
}
