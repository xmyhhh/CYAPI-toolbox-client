import type { McpLocalEntry, McpView } from '../../lib/api/native';

type Workspace = McpView['workspace'];
const workspaceNames: Record<Workspace, string> = {
  codex: 'Codex',
  claude: 'Claude Code',
  opencode: 'OpenCode',
};

/** A direct action is also consent to edit an existing local configuration. */
export function mcpConfirmation(
  action: string,
  workspace: Workspace,
  entry?: McpLocalEntry,
): string | undefined {
  if (!entry || !['mcp-enable', 'mcp-disable', 'mcp-remove'].includes(action)) return;
  const agent = workspaceNames[workspace];
  if (action === 'mcp-remove')
    return `从 ${agent} 移除 ${entry.name} 的 MCP 接入？\n只删除此软件中的接入配置，不会卸载 MCP 服务或影响其他软件。`;
  if (entry.managed) return;
  const verb = action === 'mcp-enable' ? '启用' : '停用';
  return `${verb} ${entry.name}？\n将修改 ${agent} 中的现有配置，修改前会自动备份，其他配置保持不变。`;
}
