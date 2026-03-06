import { api } from '../services/api';
import { useRemoteData } from './useRemoteData';

export function useAgents() {
  const { data: agents, ...rest } = useRemoteData(
    () => api.getAgents(),
    { key: 'agents', errorMessage: 'Failed to fetch agents' },
  );
  return { agents, ...rest };
}
