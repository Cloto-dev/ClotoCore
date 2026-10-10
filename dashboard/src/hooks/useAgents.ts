import { useCallback, useEffect } from 'react';
import { PARTNER_SETTINGS_CHANGED } from '../mate/usePartnerMedia';
import { browserSessionReady } from '../services/session';
import { useApi } from './useApi';
import { useRemoteData } from './useRemoteData';

export function useAgents() {
  const api = useApi();
  const fetcher = useCallback(async () => {
    await browserSessionReady();
    return api.getAgents();
  }, [api]);
  const { data: agents, ...rest } = useRemoteData(fetcher, {
    key: `agents:${api.apiKey}`,
    errorMessage: 'Failed to fetch agents',
  });
  const refetch = rest.refetch;
  useEffect(() => {
    const refresh = () => void refetch();
    window.addEventListener(PARTNER_SETTINGS_CHANGED, refresh);
    window.addEventListener('focus', refresh);
    return () => {
      window.removeEventListener(PARTNER_SETTINGS_CHANGED, refresh);
      window.removeEventListener('focus', refresh);
    };
  }, [refetch]);
  return { agents, ...rest };
}
