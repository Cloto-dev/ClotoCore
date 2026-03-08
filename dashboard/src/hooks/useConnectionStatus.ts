import { useState, useEffect, useCallback } from 'react';
import { api } from '../services/api';

export interface ConnectionStatus {
  connected: boolean;
  checking: boolean;
}

const POLL_INTERVAL = 5_000; // 5 seconds

export function useConnectionStatusProvider(): ConnectionStatus {
  const [connected, setConnected] = useState(false);
  const [checking, setChecking] = useState(true);

  const check = useCallback(async () => {
    try {
      await api.getHealth();
      setConnected(true);
    } catch {
      setConnected(false);
    } finally {
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    check();
    const id = setInterval(check, POLL_INTERVAL);
    return () => clearInterval(id);
  }, [check]);

  return { connected, checking };
}
