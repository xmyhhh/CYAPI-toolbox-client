import { useState } from 'react';
import {
  ArrowRight,
  CheckCircle2,
  Circle,
  CircleHelp,
  ExternalLink,
  LoaderCircle,
  Plug,
  RefreshCw,
  Unplug,
} from 'lucide-react';
import { nativeAvailable } from '../lib/api/native';
import {
  integrationCanPrepare,
  integrationComponent,
  integrationLabel,
  integrationPrepareLabel,
} from '../features/integrations/model';
import { useIntegrations } from '../features/integrations/useIntegrations';
import { IntegrationTutorial } from '../features/integrations/IntegrationTutorial';
import { VersionPanel } from '../features/integrations/VersionPanel';
import { IntegrationLink } from '../features/integrations/IntegrationLink';
import { ActionForm, ActionResult } from '../features/integrations/ActionForms';
import type { IntegrationEntry } from '../lib/api/native';

type Workspace = 'codex' | 'claude' | 'opencode';
const workspaceNames = { codex: 'Codex', claude: 'Claude Code', opencode: 'OpenCode' };

type Props = {
  workspace: Workspace;
  onManageMcp: (id: string, installed?: boolean) => void;
  onManageSkill: (id: string) => void;
  onInstallationsChanged: () => Promise<void>;
};

function checksFor(entry?: IntegrationEntry) {
  return (
    entry?.readiness?.checks ?? [
      {
        id: 'capability',
        title: '所需能力',
        state: entry?.registered ? 'ready' : 'unverified',
        message: entry?.health.message ?? '点击检查状态，或按教程完成准备。',
      },
      {
        id: 'first-use',
        title: '首次任务验证',
        state: 'unverified',
        message: '在所选 AI 工具的新会话中完成一次示例任务。',
      },
    ]
  );
}

export function Integrations({
  workspace,
  onManageMcp,
  onManageSkill,
  onInstallationsChanged,
}: Props) {
  const [selected, setSelected] = useState('');
  const { view, manifest, busy, notice, setNotice, outputs, update, run } = useIntegrations(
    workspace,
    onInstallationsChanged,
  );
  const descriptor =
    manifest?.integrations.find((item) => item.id === selected) ?? manifest?.integrations[0];
  const entry = view?.entries.find((item) => item.id === descriptor?.id);
  const component = descriptor ? integrationComponent(descriptor, view?.platform) : undefined;
  const platformMissing = !!descriptor && !descriptor.skillId && !component;
  const disabled = !!busy || !nativeAvailable;
  const canInstall =
    !!view && !!descriptor && !platformMissing && integrationCanPrepare(descriptor, entry);
  const agentName = workspaceNames[workspace];
  return (
    <div className="n-integrations">
      {notice && (
        <div
          className={`n-alert ${notice.error ? 'n-error' : 'n-success'}`}
          role={notice.error ? 'alert' : 'status'}
        >
          {notice.error ? <CircleHelp size={16} /> : <CheckCircle2 size={16} />}
          <span>{notice.text}</span>
          <button onClick={() => setNotice(undefined)} aria-label="关闭提示">
            ×
          </button>
        </div>
      )}
      <div className="n-integration-toolbar">
        <p>选择软件，一键准备 Skill、运行依赖和当前 AI 工具的 MCP 配置。</p>
        <button
          className="n-button"
          disabled={!!busy}
          onClick={() => void update('integrations-refresh')}
        >
          <RefreshCw size={14} className={busy === 'integrations-refresh' ? 'n-spin' : ''} />
          {busy === 'integrations-refresh' ? '正在刷新…' : '刷新接入目录'}
        </button>
      </div>
      <div className="n-software-picker" role="group" aria-label="选择软件">
        {manifest?.integrations.map((item) => (
          <button
            key={item.id}
            aria-pressed={descriptor?.id === item.id}
            disabled={!!busy}
            onClick={() => {
              setSelected(item.id);
              setNotice(undefined);
            }}
          >
            <span>
              <strong>{item.name}</strong>
              <ArrowRight size={16} />
            </span>
            <small>{item.guide?.summary ?? item.capabilities.join(' · ')}</small>
            <em>{item.compatibility?.supported ?? `${item.name} ${item.minimumVersion}`}</em>
          </button>
        ))}
      </div>
      {!descriptor && (
        <div className="n-empty" role="status">
          {busy ? '正在获取接入目录…' : '暂无接入目录，请刷新后重试。'}
        </div>
      )}
      {descriptor && (
        <>
          <div className="n-setup-heading">
            <div>
              <h2>
                {descriptor.name}
                <span className={`n-pill ${entry?.readiness?.state === 'ready' ? 'n-ready' : ''}`}>
                  {integrationLabel(descriptor, entry)}
                </span>
              </h2>
              <p>{descriptor.guide?.summary ?? descriptor.capabilities.join(' · ')}</p>
            </div>
            <IntegrationLink
              href={`https://tool.cyapi.cyou/?guide=integrations&software=${encodeURIComponent(descriptor.id)}`}
            >
              在线教程
              <ExternalLink size={13} />
            </IntegrationLink>
          </div>
          <VersionPanel descriptor={descriptor} entry={entry} />
          <div className="n-setup-layout">
            <div className="n-setup-main">
              <IntegrationTutorial key={descriptor.id} descriptor={descriptor} />
              {!!descriptor.actions.length && (
                <section className="n-panel n-integration-tutorial">
                  <h3>验证本机读取与操作</h3>
                  <p className="n-config-note">
                    这里检查接入组件的实际返回；AI 会话中的调用请按教程另行验证。
                  </p>
                  {descriptor.actions.map((action) => (
                    <div key={action.id}>
                      <ActionForm
                        action={action}
                        entry={entry}
                        disabled={disabled || !entry?.downloaded}
                        run={(a, values) => void run(descriptor, a, values)}
                      />
                      {outputs[`${descriptor.id}:${action.id}`] !== undefined && (
                        <ActionResult
                          action={action}
                          value={outputs[`${descriptor.id}:${action.id}`]}
                        />
                      )}
                    </div>
                  ))}
                </section>
              )}
            </div>
            <aside className="n-panel n-setup-readiness" aria-label="接入检查与安装">
              <div className="n-readiness-heading">
                <h3>接入检查</h3>
                <button
                  className="n-button"
                  disabled={!!busy}
                  onClick={() => void update('integrations-read')}
                >
                  <RefreshCw size={13} className={busy === 'integrations-read' ? 'n-spin' : ''} />
                  检查状态
                </button>
              </div>
              <p className="n-config-note">
                当前 AI 工具：<strong>{agentName}</strong>
                {entry?.service && (
                  <>
                    <br />
                    Blender 服务每 5 秒自动检测；切回工具箱时也会刷新。
                  </>
                )}
              </p>
              <ol className="n-readiness-checks">
                {checksFor(entry).map((check) => (
                  <li key={check.id} data-state={check.state}>
                    {check.state === 'ready' ? (
                      <CheckCircle2 size={17} />
                    ) : check.state === 'blocked' ? (
                      <CircleHelp size={17} />
                    ) : (
                      <Circle size={17} />
                    )}
                    <div>
                      <strong>{check.title}</strong>
                      <p>{check.message}</p>
                    </div>
                  </li>
                ))}
              </ol>
              {platformMissing && (
                <p className="n-notice">
                  当前系统尚无接入组件。已提供：
                  {descriptor.components?.map((item) => item.platform).join(' / ') ||
                    descriptor.component?.platform ||
                    '暂无'}
                  。
                </p>
              )}
              {entry?.state === 'conflict' ||
              entry?.state === 'changed' ||
              ['conflict', 'changed', 'incompatible'].includes(entry?.relatedMcp?.state ?? '') ? (
                <p className="n-notice">已有配置与目录不一致，请在 MCP 管理中检查后再处理。</p>
              ) : null}
              <div className="n-setup-buttons">
                <button
                  className="n-button n-primary"
                  disabled={disabled || !canInstall}
                  onClick={() => void update('integration-connect', descriptor)}
                >
                  {busy === 'integration-connect' ? (
                    <LoaderCircle size={15} className="n-spin" />
                  ) : (
                    <Plug size={15} />
                  )}
                  {busy === 'integration-connect'
                    ? '正在准备技能、依赖与配置…'
                    : platformMissing
                      ? '当前系统暂无组件'
                      : integrationPrepareLabel(descriptor, entry)}
                </button>
                {descriptor.relatedMcpId && (
                  <button
                    className="n-button"
                    disabled={disabled}
                    onClick={() =>
                      onManageMcp(descriptor.relatedMcpId!, entry?.relatedMcp?.configured)
                    }
                  >
                    管理 {descriptor.name} MCP
                    <ArrowRight size={14} />
                  </button>
                )}
                {descriptor.skillId && (
                  <button
                    className="n-button"
                    disabled={disabled}
                    onClick={() => onManageSkill(descriptor.skillId!)}
                  >
                    管理技能与更新
                    <ArrowRight size={14} />
                  </button>
                )}
                {descriptor.mcp && (
                  <button
                    className="n-button"
                    onClick={() => onManageMcp(descriptor.mcp!.name, true)}
                  >
                    管理 MCP 配置
                    <ArrowRight size={14} />
                  </button>
                )}
                {entry?.managed && (
                  <button
                    className="n-button"
                    disabled={disabled}
                    onClick={() => void update('integration-disconnect', descriptor)}
                  >
                    <Unplug size={14} />
                    {descriptor.skillId ? '卸载技能' : '移除接入配置'}
                  </button>
                )}
              </div>
              <p className="n-config-note">
                {descriptor.relatedMcpId
                  ? '自动安装 Skill、隔离运行环境，并配置当前 AI 工具；不修改其他 AI 工具。软件内启用插件与启动服务仍需按检查项完成。'
                  : '安装配置后，按检查项完成软件内操作。'}
                然后重新打开 {agentName} 会话，再发送教程中的示例请求。
              </p>
              <details className="n-integration-help">
                <summary>安装信息与许可</summary>
                <dl className="n-integration-status">
                  {entry?.health.rows.map((row) => (
                    <div key={row.label}>
                      <dt>{row.label}</dt>
                      <dd>{row.value}</dd>
                    </div>
                  ))}
                  {component && (
                    <div>
                      <dt>组件版本</dt>
                      <dd>
                        {component.version} · {component.platform}
                      </dd>
                    </div>
                  )}
                  <div>
                    <dt>安装资源</dt>
                    <dd>{entry?.downloaded ? '已缓存' : '安装时下载'}</dd>
                  </div>
                </dl>
                {component?.licenseText && <pre className="n-license">{component.licenseText}</pre>}
              </details>
            </aside>
          </div>
        </>
      )}
    </div>
  );
}
