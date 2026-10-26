import type {
  IntegrationDescriptor,
  IntegrationEntry,
  IntegrationManifest,
  IntegrationServicesView,
  IntegrationView,
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
    entry.usable === false ||
    entry.requirementsMet === false ||
    entry.conflict
  )
    return '需要处理';
  if (descriptor.skillId) {
    if (entry.service?.state === 'connected') return '服务已连接 · 待任务验证';
    if (entry.service?.state === 'waiting') return '等待启动 Blender 服务';
    return entry.addonReady === false ? '待软件内操作' : '已配置 · 待验证';
  }
  return entry.readiness?.state === 'ready' || entry.health.ready
    ? '本机连接正常'
    : '已配置 · 待连接';
}

/** A late probe must never validate a different workspace/configuration. */
export function integrationServiceView(view: IntegrationView, result: IntegrationServicesView) {
  if (view.workspace !== result.workspace) return view;
  return {
    ...view,
    entries: view.entries.map((entry) => {
      const probe = result.entries.find((item) => item.id === entry.id);
      if (!entry.service || !probe) return entry;
      const service =
        entry.mcpRevision === probe.mcpRevision
          ? probe.service
          : {
              state: 'unverified' as const,
              message: 'MCP 配置已变化，请点击「检查状态」重新读取；旧配置的连接结果已失效。',
              checkedAt: probe.service.checkedAt,
            };
      return {
        ...entry,
        service,
        readiness: entry.readiness && {
          ...entry.readiness,
          checks: entry.readiness.checks.map((check) =>
            check.id === 'software-service'
              ? {
                  ...check,
                  state:
                    service.state === 'connected' ? ('ready' as const) : ('unverified' as const),
                  message: service.message,
                }
              : check,
          ),
        },
      };
    }),
  };
}

export function integrationServiceFailure(view: IntegrationView, message: string) {
  if (!view.workspace) return view;
  return integrationServiceView(view, {
    workspace: view.workspace,
    entries: view.entries
      .filter((entry) => entry.service)
      .map((entry) => ({
        id: entry.id,
        mcpRevision: entry.mcpRevision,
        service: { state: 'unverified', message, checkedAt: Date.now() },
      })),
  });
}

export function integrationCanPrepare(descriptor: IntegrationDescriptor, entry?: IntegrationEntry) {
  if (entry?.requirementsMet === false || entry?.compatible === false) return false;
  if (descriptor.skillId) {
    if (['changed', 'conflict', 'incompatible'].includes(entry?.relatedMcp?.state ?? ''))
      return false;
    return (
      !entry?.registered ||
      entry.usable === false ||
      entry.addonReady === false ||
      (!!descriptor.relatedMcpId &&
        (!entry.relatedMcp?.configured || entry.relatedMcp.runtimeReady === false))
    );
  }
  return entry?.canInstall !== false && (!entry?.registered || entry.updateAvailable);
}

export function integrationPrepareLabel(
  descriptor: IntegrationDescriptor,
  entry?: IntegrationEntry,
) {
  if (!entry) return '检查状态后接入';
  if (entry.compatible === false) return '当前工具暂不支持';
  if (entry.requirementsMet === false) return '前置条件待处理';
  if (['changed', 'conflict', 'incompatible'].includes(entry.relatedMcp?.state ?? ''))
    return 'MCP 配置需检查';
  if (descriptor.skillId)
    return integrationCanPrepare(descriptor, entry)
      ? entry.registered
        ? '补全接入'
        : '一键接入'
      : '配置已准备';
  return entry.updateAvailable ? '更新接入配置' : entry.registered ? '连接已配置' : '安装并配置';
}

/** Cached server tutorials may still describe the old manual uvx/setup flow. */
export function integrationSetupSteps(descriptor: IntegrationDescriptor) {
  if (descriptor.id === 'blender' && descriptor.skillId && descriptor.relatedMcpId === 'blender') {
    return [
      {
        title: '准备 Blender',
        description: `安装 Blender ${descriptor.minimumVersion} 或更新版本，并至少打开一次，让 Blender 创建用户配置目录。`,
      },
      {
        title: '点击「一键接入」',
        description:
          '自动安装 Skill、独立 Python 与 MCP 依赖，配置当前 AI 工具，并为检测到的 Blender 版本准备插件文件。无需另装 uv 或运行 setup；已有插件文件不会被覆盖。',
      },
      {
        title: '在 Blender 中启用并启动服务',
        description:
          '在编辑 → 偏好设置 → 插件中启用 MCP for Blender（已有插件可重启 Blender 后检查）。回到 3D 视图按 N，打开「MCP for Blender」侧栏，点击「Connect to MCP server」；显示「Connected on port …」才是已启动。仅勾选插件不会启动服务。工具箱会自动检测，也可点击右侧「检查状态」。',
      },
      {
        title: '重新打开 AI 会话并验证',
        description:
          '重启当前 AI 工具并新建会话，再发送下方示例请求。确认能读取场景或完成一次操作，才算实际连通；配置已准备不代表调用已验证。',
      },
    ];
  }
  return (
    descriptor.guide?.steps ??
    descriptor.steps.map((description, i) => ({ title: `步骤 ${i + 1}`, description }))
  );
}

export function integrationPreparedNotice(
  descriptor: IntegrationDescriptor,
  entry?: IntegrationEntry,
) {
  if (
    !entry?.registered ||
    entry.usable === false ||
    entry.readiness?.state === 'needs-attention'
  ) {
    return { text: '接入尚未完成，请查看检查项；已完成的步骤已保留，可补全接入。', error: true };
  }
  if (descriptor.relatedMcpId) {
    if (!entry.relatedMcp?.configured || entry.relatedMcp.runtimeReady === false)
      return { text: '技能已安装，但 MCP 尚未准备完成，请查看检查项后补全接入。', error: true };
    const software = entry.readiness?.checks.find(
      (check) => check.id === 'software-service',
    )?.message;
    return {
      text: `Skill、MCP 运行环境与当前 AI 工具配置已准备。${software ?? '请按教程启动软件内服务'}。随后重新打开 AI 会话完成首次实际调用验证。`,
    };
  }
  return {
    text: '所需能力与当前 AI 工具配置已准备，请按教程检查软件，并在新会话中完成首次实际调用验证。',
  };
}
