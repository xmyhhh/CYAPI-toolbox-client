import { useMemo, useState } from 'react';
import {
  AlertCircle,
  ArrowRight,
  Check,
  CheckCircle2,
  Download,
  FileCode2,
  Folder,
  LoaderCircle,
  ShieldCheck,
} from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import { AgentLogo, Modal, SkillAvatar } from '../../components/Primitives';
import { installer } from '../../lib/api/installer';
import { targets } from '../../skills/targets';
import { manifestBody } from '../../skills/manifest';
import type { InstallRecord, Scope, Settings, Skill, TargetId } from '../../skills/types';

const steps = ['读取 Skill 元数据', '检查目标与冲突', '模拟暂存与发布', '保存本机演示记录'];

export function InstallDialog({
  skill,
  settings,
  installed,
  onClose,
  onInstalled,
}: {
  skill: Skill;
  settings: Settings;
  installed: InstallRecord[];
  onClose: () => void;
  onInstalled: (record: InstallRecord, replaced: boolean) => void;
}) {
  const [target, setTarget] = useState<TargetId>(settings.defaultTarget);
  const [scope, setScope] = useState<Scope>(settings.scope);
  const [projectPath, setProjectPath] = useState(settings.projectPath);
  const [replace, setReplace] = useState(false);
  const [tab, setTab] = useState<'overview' | 'readme'>('overview');
  const [step, setStep] = useState(-1);
  const [status, setStatus] = useState<'ready' | 'running' | 'success'>('ready');
  const [error, setError] = useState('');
  const preview = useMemo(() => {
    try {
      return {
        plan: installer.preview(skill, { defaultTarget: target, scope, projectPath }, installed),
      };
    } catch (reason) {
      return { error: (reason as Error).message };
    }
  }, [skill, target, scope, projectPath, installed]);
  const plan = preview.plan;

  async function install() {
    if (!plan || status !== 'ready') return;
    setError('');
    setStatus('running');
    try {
      const record = await installer.install(plan, replace, setStep);
      onInstalled(record, Boolean(plan.conflict));
      setStatus('success');
    } catch (reason) {
      setError((reason as Error).message);
      setStatus('ready');
      setStep(-1);
    }
  }

  return (
    <Modal
      title={status === 'success' ? '模拟安装完成' : '安装 Skill'}
      onClose={onClose}
      busy={status === 'running'}
      wide
    >
      {status === 'success' ? (
        <>
          <div className="success-state">
            <span className="success-icon">
              <CheckCircle2 size={35} />
            </span>
            <h3>{skill.title} 已添加</h3>
            <p>已保存到 {targets.find((item) => item.id === target)?.name} 的演示安装列表。</p>
            <code>{plan?.path}</code>
            <div className="notice">
              <ShieldCheck size={16} />
              这是一次模拟安装，实际 Agent 目录未发生变化。
            </div>
          </div>
          <div className="modal-footer">
            <span className="muted">可在「已安装」中管理演示记录</span>
            <button className="button primary" onClick={onClose}>
              完成 <Check size={16} />
            </button>
          </div>
        </>
      ) : (
        <>
          <div className="install-summary">
            <SkillAvatar skill={skill} large />
            <div>
              <h3>{skill.title}</h3>
              <span className="mono muted">
                {skill.name} <span className="summary-dot">·</span> {skill.version}
              </span>
            </div>
            <span className="tag source-tag">
              {skill.source === 'example' ? '示例 Skill' : '本地导入'}
            </span>
          </div>
          <div className="dialog-tabs">
            <button
              className={tab === 'overview' ? 'active' : ''}
              onClick={() => setTab('overview')}
            >
              安装配置
            </button>
            <button className={tab === 'readme' ? 'active' : ''} onClick={() => setTab('readme')}>
              <FileCode2 size={15} /> SKILL.md
            </button>
          </div>
          <div className="modal-body">
            {tab === 'readme' ? (
              <div className="markdown">
                <ReactMarkdown
                  allowedElements={[
                    'h1',
                    'h2',
                    'h3',
                    'h4',
                    'p',
                    'ul',
                    'ol',
                    'li',
                    'blockquote',
                    'code',
                    'pre',
                    'strong',
                    'em',
                    'hr',
                    'br',
                  ]}
                  unwrapDisallowed
                >
                  {manifestBody(skill.markdown)}
                </ReactMarkdown>
              </div>
            ) : (
              <>
                <p className="skill-description">{skill.description}</p>
                <fieldset disabled={status === 'running'} className="install-fields">
                  <legend className="field-label">安装到</legend>
                  <div className="target-options">
                    {targets.map((item) => (
                      <button
                        key={item.id}
                        className={`target-option ${target === item.id ? 'selected' : ''}`}
                        aria-pressed={target === item.id}
                        onClick={() => {
                          setTarget(item.id);
                          setReplace(false);
                        }}
                      >
                        <AgentLogo target={item.id} />
                        <span>
                          <strong>{item.name}</strong>
                          <small>{item.subtitle}</small>
                        </span>
                        <span className="radio-mark">
                          {target === item.id && <Check size={12} />}
                        </span>
                      </button>
                    ))}
                  </div>
                  <span className="field-label scope-label">作用范围</span>
                  <div className="scope-options">
                    <label>
                      <input
                        type="radio"
                        name="scope"
                        checked={scope === 'global'}
                        onChange={() => {
                          setScope('global');
                          setReplace(false);
                        }}
                      />
                      <span>
                        全局安装<small>所有项目都可以使用</small>
                      </span>
                    </label>
                    <label>
                      <input
                        type="radio"
                        name="scope"
                        checked={scope === 'project'}
                        onChange={() => {
                          setScope('project');
                          setReplace(false);
                        }}
                      />
                      <span>
                        项目安装<small>仅在指定项目中使用</small>
                      </span>
                    </label>
                  </div>
                  {scope === 'project' && (
                    <label className="field-label project-field">
                      项目路径
                      <input
                        className="text-input mono"
                        value={projectPath}
                        onChange={(event) => {
                          setProjectPath(event.target.value);
                          setReplace(false);
                        }}
                        placeholder="C:\Projects\my-app"
                      />
                    </label>
                  )}
                </fieldset>
                <div className="path-preview">
                  <span>
                    <Folder size={15} /> 预计安装路径
                  </span>
                  <code>{plan?.path ?? '请先填写有效的项目路径'}</code>
                </div>
                {preview.error && (
                  <div className="form-error" role="alert">
                    <AlertCircle size={16} />
                    {preview.error}
                  </div>
                )}
                {plan?.conflict && (
                  <label className="conflict-notice">
                    <input
                      type="checkbox"
                      checked={replace}
                      disabled={status === 'running'}
                      onChange={(event) => setReplace(event.target.checked)}
                    />
                    <span>
                      目标已有同名演示记录，确认替换。
                      <small>此操作只更新工具内的模拟安装记录。</small>
                    </span>
                  </label>
                )}
              </>
            )}
            {status === 'running' && (
              <div className="install-progress" aria-live="polite">
                {steps.map((label, index) => (
                  <div className={index <= step ? 'current' : ''} key={label}>
                    {index < step ? (
                      <CheckCircle2 size={16} />
                    ) : index === step ? (
                      <LoaderCircle size={16} className="spin" />
                    ) : (
                      <span className="step-dot" />
                    )}
                    <span>{label}</span>
                    {index === step && <small>进行中</small>}
                  </div>
                ))}
              </div>
            )}
            {error && (
              <div className="form-error" role="alert">
                <AlertCircle size={16} />
                {error}
              </div>
            )}
            <div className="notice">
              <ShieldCheck size={16} />
              <span>演示模式 · 仅模拟流程，不会复制文件或修改 Agent 目录。</span>
            </div>
          </div>
          <div className="modal-footer">
            <button className="button secondary" onClick={onClose} disabled={status === 'running'}>
              取消
            </button>
            <button
              className="button primary"
              onClick={install}
              disabled={!plan || Boolean(plan.conflict && !replace) || status === 'running'}
            >
              {status === 'running' ? (
                <>
                  <LoaderCircle size={16} className="spin" />
                  模拟安装中…
                </>
              ) : (
                <>
                  <Download size={16} />
                  {plan?.conflict ? '替换并模拟安装' : '开始模拟安装'}
                  <ArrowRight size={15} />
                </>
              )}
            </button>
          </div>
        </>
      )}
    </Modal>
  );
}
