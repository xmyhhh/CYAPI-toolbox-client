import { SERVER_URL } from '../../lib/api/native';
import type {
  WorkflowCatalog,
  WorkflowDraft,
  WorkflowQuote,
  WorkflowState,
  WorkflowTask,
  WorkflowTemplate,
} from './types';

const SERVER = SERVER_URL.replace(/\/+$/, '');
export const WORKFLOW_API = `${SERVER}/api/workflows`;
export const WORKFLOW_ORIGIN = new URL(WORKFLOW_API).origin;

export class WorkflowError extends Error {
  status: number;
  constructor(message: string, status: number) {
    super(message);
    this.name = 'WorkflowError';
    this.status = status;
  }
}

async function readError(response: Response) {
  const text = await response.text().catch(() => '');
  if (!text) return `请求失败（${response.status}）`;
  try {
    const value = JSON.parse(text) as { error?: unknown; message?: unknown };
    if (typeof value.error === 'string' && value.error) return value.error;
    if (typeof value.message === 'string' && value.message) return value.message;
    if (value.error && typeof value.error === 'object') {
      const error = value.error as { message?: unknown };
      if (typeof error.message === 'string') return error.message;
    }
  } catch {
    // Plain text errors are already useful.
  }
  return text.slice(0, 400);
}

async function request<T>(
  path: string,
  {
    method = 'GET',
    key,
    body,
    headers = {},
    signal,
  }: {
    method?: string;
    key?: string;
    body?: unknown;
    headers?: Record<string, string>;
    signal?: AbortSignal;
  } = {},
): Promise<T> {
  const response = await fetch(WORKFLOW_API + path, {
    method,
    headers: {
      ...(key ? { Authorization: `Bearer ${key}` } : {}),
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
      ...headers,
    },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });
  if (!response.ok) throw new WorkflowError(await readError(response), response.status);
  if (response.status === 204) return undefined as T;
  const text = await response.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

export const workflowApi = {
  catalog: () => request<WorkflowCatalog>('/catalog'),
  state: (key: string) => request<WorkflowState>('/state', { key }),
  /** Verifies a Key against the service. No server-side session is created. */
  verify: (key: string) =>
    request<{ mode: string }>('/state', { key }).then((state) => ({ mode: state.mode })),
  createDraft: (key: string, templateId: string) =>
    request<WorkflowDraft>('/drafts', { method: 'POST', key, body: { templateId } }),
  saveDraft: (
    key: string,
    draft: {
      id: string;
      name: string;
      description: string;
      references: string[];
      audio?: string;
      revision: number;
    },
  ) =>
    request<WorkflowDraft>(`/drafts/${encodeURIComponent(draft.id)}`, {
      method: 'PUT',
      key,
      body: {
        name: draft.name,
        description: draft.description,
        references: draft.references,
        audio: draft.audio,
        revision: draft.revision,
      },
    }),
  quote: (key: string, draftId: string) =>
    request<WorkflowQuote>(`/drafts/${encodeURIComponent(draftId)}/quote`, { key }),
  run: (key: string, draftId: string, quote: WorkflowQuote) =>
    request<WorkflowTask>(`/drafts/${encodeURIComponent(draftId)}/run`, {
      method: 'POST',
      key,
      body: { revision: quote.revision, quote },
      headers: { 'Idempotency-Key': crypto.randomUUID() },
    }),
  cancel: (key: string, taskId: string) =>
    request<WorkflowTask>(`/tasks/${encodeURIComponent(taskId)}/cancel`, {
      method: 'POST',
      key,
      body: {},
    }),
  createTemplate: (
    key: string,
    input: { name: string; fromId?: string; shared?: unknown },
  ) => request<WorkflowTemplate>('/templates', { method: 'POST', key, body: input }),
  deleteTemplate: (key: string, id: string) =>
    request<{ ok: boolean }>(`/templates/${encodeURIComponent(id)}`, {
      method: 'DELETE',
      key,
    }),
  canvasSession: (key: string, kind: string, id: string) =>
    request<{ token: string; expiresAt: number }>('/canvas/session', {
      method: 'POST',
      key,
      body: { kind, id },
    }),
};

/**
 * The canvas is a server-owned ComfyUI page inside a client dialog. The token
 * is scoped to the iframe and never exposed as the API Key. `po` tells the
 * canvas which parent origin may drive its toolbar messages.
 */
export function canvasUrl(kind: string, id: string, token: string, preview: boolean) {
  const query = new URLSearchParams({
    cyapiKind: kind,
    cyapiId: id,
    ct: token,
    po: window.location.origin,
  });
  if (preview) query.set('cyapiPreview', '1');
  return `${WORKFLOW_ORIGIN}/api/workflows/canvas/?${query}`;
}

/** The API Key cannot ride an <img> request, so media is fetched and shown as a blob. */
export async function fetchMedia(
  path: string,
  key?: string,
  signal?: AbortSignal,
): Promise<Blob> {
  const url = path.startsWith('http') ? path : WORKFLOW_ORIGIN + path;
  const response = await fetch(url, {
    headers: key ? { Authorization: `Bearer ${key}` } : {},
    signal,
  });
  if (!response.ok) throw new WorkflowError(await readError(response), response.status);
  return response.blob();
}

export async function fetchText(path: string, key?: string, signal?: AbortSignal) {
  return (await fetchMedia(path, key, signal)).text();
}

export function artifactPath(path: string) {
  return path.startsWith('http') ? path : WORKFLOW_ORIGIN + path;
}

export async function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = filename;
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 30_000);
}

export async function downloadArtifact(
  path: string,
  filename: string,
  key: string,
) {
  await downloadBlob(await fetchMedia(path, key), filename);
}

export async function uploadWorkflowFile(
  key: string,
  file: File,
  contentType?: string,
): Promise<{ id: string }> {
  const response = await fetch(WORKFLOW_API + '/uploads', {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${key}`,
      'Content-Type': contentType || file.type || 'application/octet-stream',
      'Content-Length': String(file.size),
    },
    body: file,
  });
  if (!response.ok) throw new WorkflowError(await readError(response), response.status);
  return (await response.json()) as { id: string };
}

export const uploadReferenceImage = (key: string, file: File) =>
  uploadWorkflowFile(key, file);

export const uploadAudio = (key: string, file: File) =>
  uploadWorkflowFile(key, file, 'audio/wav');

export async function importShare(key: string, file: File): Promise<WorkflowTemplate> {
  if (file.size > 256_000) throw new WorkflowError('分享文件不能超过 256 KB', 413);
  let shared: unknown;
  try {
    shared = JSON.parse(await file.text());
  } catch {
    throw new WorkflowError('分享文件不是有效的 JSON', 400);
  }
  return workflowApi.createTemplate(key, { name: '导入的工作流', shared });
}

export async function exportShare(key: string, id: string, name = 'workflow') {
  const blob = await fetchMedia(`/api/workflows/templates/${encodeURIComponent(id)}/share`, key);
  await downloadBlob(blob, `${name}.json`);
}
