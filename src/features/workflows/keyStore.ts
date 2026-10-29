/**
 * Workflow API Keys belong to the desktop client, not to the server.
 *
 * The service only ever sees them as `Authorization: Bearer …`. Nothing is
 * written to the server's keyring or a device cookie, so a fresh client can be
 * used without registering a device and no CSRF token is involved.
 */
export interface StoredKey {
  id: string;
  name: string;
  key: string;
  createdAt: number;
  /** A key used once. It is never written to storage. */
  temporary?: boolean;
}

export interface StoredKeyState {
  keys: StoredKey[];
  activeId: string | null;
}

const STORAGE = 'cyapi.workflows.keys.v1';
const MAX_KEYS = 10;

export const maskKey = (key: string) => `•••• ${key.slice(-4)}`;

function readStorage(): StoredKeyState {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(STORAGE) || 'null');
    if (!value || typeof value !== 'object') return { keys: [], activeId: null };
    const item = value as { keys?: unknown; activeId?: unknown };
    const keys = Array.isArray(item.keys)
      ? item.keys
          .filter(
            (entry): entry is StoredKey =>
              Boolean(entry) &&
              typeof entry === 'object' &&
              typeof (entry as StoredKey).id === 'string' &&
              typeof (entry as StoredKey).name === 'string' &&
              typeof (entry as StoredKey).key === 'string',
          )
          .slice(0, MAX_KEYS)
      : [];
    const activeId =
      typeof item.activeId === 'string' && keys.some((entry) => entry.id === item.activeId)
        ? item.activeId
        : null;
    return { keys, activeId };
  } catch {
    return { keys: [], activeId: null };
  }
}

export function loadKeys(): StoredKeyState {
  return readStorage();
}

export function saveKeys(keys: StoredKey[], activeId: string | null): void {
  try {
    localStorage.setItem(
      STORAGE,
      JSON.stringify({ keys: keys.filter((entry) => !entry.temporary), activeId }),
    );
  } catch {
    // A full or unavailable storage must not break the current session.
  }
}

export function addKey(keys: StoredKey[], name: string, value: string): StoredKey {
  const key = value.trim();
  if (!key) throw new Error('请输入 API Key');
  if (key.length > 4096) throw new Error('API Key 过长');
  const existing = keys.find((entry) => entry.key === key);
  if (existing) return existing;
  if (keys.length >= MAX_KEYS) throw new Error(`最多保存 ${MAX_KEYS} 个 Key`);
  return {
    id: crypto.randomUUID(),
    name: name.trim().slice(0, 40) || `Key ${keys.length + 1}`,
    key,
    createdAt: Date.now(),
  };
}
