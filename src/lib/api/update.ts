import { nativeCall, nativeAvailable, SERVER_URL } from './native';
import { APP_VERSION, DOWNLOAD_PAGE } from '../app-info';

export interface UpdateFile {
  kind: 'installer' | 'portable';
  filename: string;
  url: string;
  size: number;
  sha256: string;
}

export interface UpdateStatus {
  schema: number;
  current: string;
  latest: string;
  hasUpdate: boolean;
  mandatory: boolean;
  notes: string[];
  releasedAt: string;
  downloadPage: string;
  installer?: UpdateFile;
  portable?: UpdateFile;
  installMode: 'installed' | 'portable';
  canSelfUpdate: boolean;
  source: 'online' | 'cached';
  checkError?: string | null;
}

export interface UpdateDownload {
  path: string;
  version: string;
  size: number;
  sha256: string;
  cached: boolean;
}

const SKIPPED_KEY = 'cyapi-toolbox.update.skipped';

export function skippedVersion(): string {
  try {
    return localStorage.getItem(SKIPPED_KEY) ?? '';
  } catch {
    return '';
  }
}

export function rememberSkipped(version: string): void {
  try {
    if (version) localStorage.setItem(SKIPPED_KEY, version);
    else localStorage.removeItem(SKIPPED_KEY);
  } catch {
    // A storage failure only means the reminder may reappear next launch.
  }
}

export async function checkUpdate(): Promise<UpdateStatus> {
  if (nativeAvailable) return nativeCall<UpdateStatus>('update-check');
  // Browser preview can check releases and download, but cannot replace a desktop binary.
  const response = await fetch(
    `${SERVER_URL}/api/client-release?currentVersion=${encodeURIComponent(APP_VERSION)}`,
    {
      signal: AbortSignal.timeout(8000),
      cache: 'no-store',
    },
  );
  if (!response.ok) throw new Error('无法获取更新信息');
  const manifest = (await response.json()) as {
    schema: number;
    version: string;
    currentVersion: string;
    hasUpdate: boolean;
    mandatory: boolean;
    notes?: string[];
    releasedAt?: string;
    downloadPage?: string;
    downloads?: UpdateFile[];
  };
  if (
    manifest.schema !== 1 ||
    !/^\d+\.\d+\.\d+$/.test(manifest.version) ||
    manifest.currentVersion !== APP_VERSION ||
    typeof manifest.hasUpdate !== 'boolean' ||
    typeof manifest.mandatory !== 'boolean'
  )
    throw new Error('更新信息格式无效');
  return {
    schema: 1,
    current: APP_VERSION,
    latest: manifest.version,
    hasUpdate: manifest.hasUpdate,
    mandatory: manifest.mandatory,
    notes: manifest.notes ?? [],
    releasedAt: manifest.releasedAt ?? '',
    downloadPage: manifest.downloadPage ?? DOWNLOAD_PAGE,
    installer: manifest.downloads?.find((file) => file.kind === 'installer'),
    portable: manifest.downloads?.find((file) => file.kind === 'portable'),
    installMode: 'portable',
    canSelfUpdate: false,
    source: 'online',
  };
}

export async function downloadUpdate(): Promise<UpdateDownload> {
  return nativeCall<UpdateDownload>('update-download');
}

export async function installUpdate(): Promise<{ started: boolean }> {
  return nativeCall<{ started: boolean }>('update-install');
}

export async function openExternal(url: string): Promise<void> {
  if (nativeAvailable) {
    await nativeCall('open-url', { url });
    return;
  }
  window.open(url, '_blank', 'noopener,noreferrer');
}

/** Prefer a direct file so the browser starts downloading immediately. */
export function updateTargetUrl(status: UpdateStatus): string {
  const file = status.installMode === 'installed' ? status.installer : status.portable;
  return file?.url ?? status.downloadPage;
}
