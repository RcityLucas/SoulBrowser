import { create } from 'zustand';
import { persist } from 'zustand/middleware';
import { soulbrowserAPI } from '@/api/soulbrowser';
import { apiClient } from '@/api/client';

export type BackendStatus = 'unknown' | 'checking' | 'online' | 'offline';

interface BackendConfigState {
  baseUrl: string;
  status: BackendStatus;
  lastChecked?: string;
  setBaseUrl: (url: string) => void;
  setStatus: (status: BackendStatus, checkedAt?: string) => void;
  checkBackend: () => Promise<void>;
}

const defaultBaseUrl = soulbrowserAPI.getBaseUrl();

const normalizeUrl = (url: string) => url.replace(/\/+$/, '').toLowerCase();
const legacyBaseUrls = new Set([
  '',
  'http://127.0.0.1:8788',
  'http://localhost:8788',
  'http://0.0.0.0:8788',
  'http://127.0.0.1:8800',
  'http://localhost:8800',
  'http://0.0.0.0:8800',
  'http://127.0.0.1:8801',
  'http://localhost:8801',
  'http://0.0.0.0:8801',
]);

export const useBackendConfigStore = create<BackendConfigState>()(
  persist(
    (set, get) => ({
      baseUrl: defaultBaseUrl,
      status: 'unknown',
      lastChecked: undefined,
      setBaseUrl: (url) => {
        const trimmed = url.trim();
        soulbrowserAPI.setBaseUrl(trimmed);
        apiClient.setBaseUrl(trimmed);
        set({ baseUrl: trimmed, status: 'checking' });
        void get().checkBackend();
      },
      setStatus: (status, checkedAt) =>
        set((state) => ({
          status,
          lastChecked:
            status === 'online' || status === 'offline'
              ? checkedAt ?? new Date().toISOString()
              : state.lastChecked,
        })),
      checkBackend: async () => {
        set({ status: 'checking' });
        try {
          const ok = await apiClient.healthCheck();
          set({
            status: ok ? 'online' : 'offline',
            lastChecked: new Date().toISOString(),
          });
        } catch {
          set({ status: 'offline', lastChecked: new Date().toISOString() });
        }
      },
    }),
    {
      name: 'soulbrowser-backend-config',
      version: 3,
      migrate: (persistedState: unknown, _version) => {
        const state = persistedState as BackendConfigState | undefined;
        if (!state) return state;
        const normalized = normalizeUrl(state.baseUrl ?? '');
        if (legacyBaseUrls.has(normalized)) {
          return { ...state, baseUrl: defaultBaseUrl };
        }
        return state;
      },
      partialize: (state) => ({ baseUrl: state.baseUrl }),
      onRehydrateStorage: () => (state) => {
        if (!state) return;
        const normalized = normalizeUrl(state.baseUrl ?? '');
        if (legacyBaseUrls.has(normalized)) {
          state.setBaseUrl(defaultBaseUrl);
          return;
        }
        if (state.baseUrl) {
          soulbrowserAPI.setBaseUrl(state.baseUrl);
          apiClient.setBaseUrl(state.baseUrl);
        }
      },
    }
  )
);
