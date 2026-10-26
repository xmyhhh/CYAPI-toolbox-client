import { useEffect, useSyncExternalStore } from 'react';
import { cyapiRoutesSnapshot, readCyapiRoutes, subscribeCyapiRoutes } from '../lib/api/cyapi';

export function useCyapiRoutes() {
  const snapshot = useSyncExternalStore(subscribeCyapiRoutes, cyapiRoutesSnapshot);
  useEffect(() => {
    void readCyapiRoutes();
  }, []);
  return { ...snapshot, refresh: () => readCyapiRoutes({ force: true }) };
}
