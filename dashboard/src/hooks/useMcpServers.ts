import { api } from '../services/api';
import { useRemoteData } from './useRemoteData';

export function useMcpServers(apiKey: string) {
  const { data: servers, ...rest } = useRemoteData(
    () => api.listMcpServers(apiKey).then(d => d.servers),
    { key: `mcp-servers:${apiKey}`, errorMessage: 'Failed to connect', minRefetchMs: 400 },
  );
  return { servers, ...rest };
}
