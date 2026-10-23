import { Copy, Palette, Settings2, Trash2 } from 'lucide-react';
import { canManageInstallation } from '../../lib/installations';
import type { NativeRecord } from '../../lib/api/native';
import type { InstalledSkill } from './collections';

const targetLabels = { codex: 'Codex', claude: 'Claude Code', opencode: 'OpenCode' };
const sourceLabels = { codex: 'Codex 自带', system: '系统环境', managed: '工具箱共享' };

export function InstalledSkillRow({
  entry,
  workspace,
  busy,
  configurable,
  onCopy,
  onConfigure,
  onRemove,
}: {
  entry: InstalledSkill;
  workspace: NativeRecord['target'];
  busy: boolean;
  configurable: boolean;
  onCopy: (path: string) => void;
  onConfigure: (record: NativeRecord) => void;
  onRemove: (record: NativeRecord) => void;
}) {
  const { record, exists, managed } = entry;
  const canManage = canManageInstallation(entry, workspace);
  return (
    <article className="n-record">
      <span className="n-skill-icon">
        <Palette size={23} />
      </span>
      <div>
        <h2>
          {record.title}
          {record.version && <span className="n-version">v{record.version}</span>}
          <span className="n-pill">
            {targetLabels[record.target]} · {record.scope === 'global' ? '全局' : '项目'}
          </span>
          {managed === false && <span className="n-pill">本机已有</span>}
          {record.target !== workspace && <span className="n-pill">兼容目录 · 在原工作区管理</span>}
          {!exists && <span className="n-pill n-warning">文件已变化</span>}
        </h2>
        <code>{record.path}</code>
        <p className="n-record-meta">
          {record.runtimes
            .map(
              (runtime) => `${runtime.kind} ${runtime.version} · ${sourceLabels[runtime.source]}`,
            )
            .join(' / ')}
          {record.installedAt > 0 && (
            <span>{new Date(record.installedAt).toLocaleDateString('zh-CN')}</span>
          )}
        </p>
      </div>
      {canManage && configurable && (
        <button className="n-button" disabled={busy || !exists} onClick={() => onConfigure(record)}>
          <Settings2 size={15} />
          配置
        </button>
      )}
      <button
        className="n-icon-button"
        onClick={() => onCopy(record.path)}
        aria-label="复制安装路径"
        title="复制路径"
      >
        <Copy size={16} />
      </button>
      {canManage && (
        <button className="n-button" disabled={busy} onClick={() => onRemove(record)}>
          <Trash2 size={16} />
          卸载
        </button>
      )}
    </article>
  );
}
