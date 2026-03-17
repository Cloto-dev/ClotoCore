import { useCallback, useState } from 'react';

function createStorageHook(getStorage: () => Storage) {
  return function useStorage(key: string, fallback: string): [string, (v: string) => void, () => void] {
    const [value, setInner] = useState<string>(() => {
      try {
        return getStorage().getItem(key) ?? fallback;
      } catch {
        return fallback;
      }
    });

    const set = useCallback(
      (v: string) => {
        setInner(v);
        try {
          getStorage().setItem(key, v);
        } catch (e) {
          if (import.meta.env.DEV) console.warn(`useStorage: failed to write key "${key}"`, e);
        }
      },
      [key],
    );

    const remove = useCallback(() => {
      setInner(fallback);
      try {
        getStorage().removeItem(key);
      } catch (e) {
        if (import.meta.env.DEV) console.warn(`useStorage: failed to remove key "${key}"`, e);
      }
    }, [key, fallback]);

    return [value, set, remove];
  };
}

/** Read/write a string value in localStorage with error handling. */
export const useLocalStorage = createStorageHook(() => localStorage);

/** Read/write a string value in sessionStorage with error handling. */
export const useSessionStorage = createStorageHook(() => sessionStorage);
