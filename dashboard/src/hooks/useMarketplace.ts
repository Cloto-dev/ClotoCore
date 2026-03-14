import { useRemoteData } from './useRemoteData';
import { useApi } from './useApi';
import { MarketplaceCatalogEntry } from '../types';

export function useMarketplace() {
  const api = useApi();
  const { data: servers, ...rest } = useRemoteData<MarketplaceCatalogEntry>(
    () => api.getMarketplaceCatalog().then(d => d.servers),
    { key: `marketplace:${api.apiKey}`, errorMessage: 'Failed to load catalog', minRefetchMs: 400 },
  );
  return { servers, ...rest };
}
