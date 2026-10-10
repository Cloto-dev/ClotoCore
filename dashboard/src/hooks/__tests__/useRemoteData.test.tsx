import { renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { useRemoteData } from '../useRemoteData';

describe('useRemoteData', () => {
  it('keeps the HTTP status of a failed read beside its message', async () => {
    const refused = Object.assign(new Error('Permission denied'), { status: 403 });
    const { result } = renderHook(() => useRemoteData(() => Promise.reject(refused), { key: 'status-403' }));
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.error).toBe('Permission denied');
    expect(result.current.errorStatus).toBe(403);
  });

  it('has no status for a failure that was not an HTTP answer', async () => {
    const { result } = renderHook(() =>
      useRemoteData(() => Promise.reject(new TypeError('Failed to fetch')), { key: 'status-none' }),
    );
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.error).toBe('Failed to fetch');
    expect(result.current.errorStatus).toBeNull();
  });
});
