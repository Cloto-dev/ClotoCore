import type { MarketplaceCatalogEntry } from '../types';
import { useApi } from './useApi';
import { useRemoteData } from './useRemoteData';

export function useMarketplace() {
  const api = useApi();
  const { data: servers, ...rest } = useRemoteData<MarketplaceCatalogEntry>(
    () => api.getMarketplaceCatalog().then((d) => d.servers),
    {
      key: `marketplace:${api.apiKey}`,
      errorMessage: 'Failed to load catalog',
      minRefetchMs: 400,
      refetcher: () => api.getMarketplaceCatalog(true).then((d) => d.servers),
    },
  );
  return { servers, ...rest };
}
