import type {
  IntegrationDescriptor,
  IntegrationEntry,
  IntegrationManifest,
} from '../../lib/api/native.ts';

/** Older cached catalogs remain readable while tutorials are published independently. */
export function integrationManifest(value: unknown): IntegrationManifest {
  const manifest = value as IntegrationManifest;
  if (!manifest || ![2, 3, 4].includes(manifest.schema) || !Array.isArray(manifest.integrations)) {
    throw new Error('接入目录格式不受支持，请更新客户端。');
  }
  return {
    ...manifest,
    integrations: manifest.integrations.map((item) => ({
      ...item,
      actions: (item.actions ?? []).map((action) => ({
        ...action,
        args: action.args ?? {},
        fields: action.fields ?? [],
        secondary: action.secondary ?? false,
      })),
    })),
  };
}

export function integrationComponent(descriptor: IntegrationDescriptor, platform?: string) {
  return (
    descriptor.components?.find((item) => item.platform === platform) ??
    (descriptor.component?.platform === platform ? descriptor.component : undefined)
  );
}

export function integrationLabel(descriptor: IntegrationDescriptor, entry?: IntegrationEntry) {
  if (!entry) return '待检查';
  if (!entry.registered) return '待准备';
  if (
    entry.readiness?.state === 'needs-attention' ||
    entry.requirementsMet === false ||
    entry.conflict
  )
    return '需要处理';
  if (descriptor.skillId) return '待首次验证';
  return entry.readiness?.state === 'ready' || entry.health.ready
    ? '本机连接正常'
    : '已配置 · 待连接';
}
