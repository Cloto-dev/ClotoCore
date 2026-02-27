import { useState, useEffect, useCallback, useRef } from 'react';
import { api } from '../services/api';

export interface ConnectionStatus {
  connected: boolean;
  checking: boolean;
}

const POLL_INTERVAL = 5_000; // 5 seconds
const FAILURE_THRESHOLD = 3; // consecutive failures before disconnected

export function useConnectionStatusProvider(): ConnectionStatus {
  const [connected, setConnected] = useState(false);
  const [checking, setChecking] = useState(true);
  const failCountRef = useRef(0);

  const check = useCallback(async () => {
    try {
      await api.getHealth();
      failCountRef.current = 0;
      setConnected(true);
    } catch {
      failCountRef.current += 1;
      if (failCountRef.current >= FAILURE_THRESHOLD) {
        setConnected(false);
      }
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
