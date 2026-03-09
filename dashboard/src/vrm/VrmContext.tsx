import { createContext, useContext, useState, useCallback, useEffect } from 'react';
import { AvatarAgentState, IdleBehaviorParams, DEFAULT_IDLE_PARAMS } from './engine/types';

interface VrmContextValue {
  agentState: AvatarAgentState;
  setAgentState: (state: AvatarAgentState) => void;
  idleParams: IdleBehaviorParams;
  setIdleParams: (params: IdleBehaviorParams) => void;
  vrmEnabled: boolean;
  vrmVisible: boolean;
  setVrmVisible: (v: boolean) => void;
}

const VrmContext = createContext<VrmContextValue | null>(null);

const VRM_VISIBLE_KEY = 'cloto-vrm-visible';

export function VrmProvider({
  vrmEnabled,
  children,
}: {
  vrmEnabled: boolean;
  children: React.ReactNode;
}) {
  const [agentState, setAgentState] = useState<AvatarAgentState>('idle');
  const [idleParams, setIdleParams] = useState<IdleBehaviorParams>(DEFAULT_IDLE_PARAMS);
  const [vrmVisible, setVrmVisibleState] = useState(() => {
    try {
      return localStorage.getItem(VRM_VISIBLE_KEY) !== 'false';
    } catch {
      return true;
    }
  });

  const setVrmVisible = useCallback((v: boolean) => {
    setVrmVisibleState(v);
    try {
      localStorage.setItem(VRM_VISIBLE_KEY, String(v));
    } catch { /* ignore */ }
  }, []);

  // Reset state when vrmEnabled changes (agent switch)
  useEffect(() => {
    setAgentState('idle');
  }, [vrmEnabled]);

  return (
    <VrmContext.Provider value={{
      agentState,
      setAgentState,
      idleParams,
      setIdleParams,
      vrmEnabled,
      vrmVisible: vrmEnabled && vrmVisible,
      setVrmVisible,
    }}>
      {children}
    </VrmContext.Provider>
  );
}

export function useVrmContext(): VrmContextValue {
  const ctx = useContext(VrmContext);
  if (!ctx) throw new Error('useVrmContext must be used within VrmProvider');
  return ctx;
}
