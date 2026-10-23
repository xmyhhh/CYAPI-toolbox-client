import { ArrowDownToLine, CheckCircle2, Palette, RefreshCw, Settings2 } from 'lucide-react';
import { skillCategory } from '../../lib/catalog';
import type { ServerSkill } from '../../lib/api/native';
import type { CatalogSkill } from './collections';

export function SkillRow({
  entry,
  busy,
  onSelect,
  onUpdate,
  onConfigure,
}: {
  entry: CatalogSkill;
  busy: boolean;
  onSelect: (skill: ServerSkill) => void;
  onUpdate: (id: string) => void;
  onConfigure: (skill: ServerSkill) => void;
}) {
  const { skill, offlineReady, installed, installedPaths, updateAvailable } = entry;
  return (
    <article className="n-skill-row">
      <span className="n-skill-icon">
        <Palette size={23} strokeWidth={1.6} />
      </span>
      <div className="n-skill-info">
        <h2>
          {skill.title}
          <span className="n-pill">{skillCategory(skill)}</span>
          {updateAvailable && <span className="n-pill n-warning">可更新</span>}
        </h2>
        <p title={skill.description}>{skill.description}</p>
        <div className="n-skill-meta">
          {skill.requirements.map((requirement) => (
            <span key={requirement.kind}>
              {requirement.kind === 'node' ? 'Node' : 'Python'} ≥ {requirement.minimum}
            </span>
          ))}
          {!installed && (
            <span className={offlineReady ? 'n-ready' : ''}>
              {skill.distribution === 'source-only'
                ? '官方获取'
                : offlineReady
                  ? '可离线安装'
                  : '未下载'}
            </span>
          )}
        </div>
      </div>
      {skill.configuration && (
        <button
          className="n-icon-button"
          aria-label={`配置 ${skill.title}`}
          title="配置"
          disabled={busy}
          onClick={() => onConfigure(skill)}
        >
          <Settings2 size={17} />
        </button>
      )}
      {updateAvailable && (
        <button className="n-button n-primary" disabled={busy} onClick={() => onUpdate(skill.id)}>
          <RefreshCw size={15} />
          更新
        </button>
      )}
      <button
        className={`n-button ${installed || skill.distribution === 'source-only' ? '' : 'n-primary'}`}
        title={installedPaths?.join('\n')}
        onClick={() => onSelect(skill)}
        disabled={busy}
      >
        {installed ? <CheckCircle2 size={15} /> : <ArrowDownToLine size={15} />}
        {installed ? '已安装' : skill.distribution === 'source-only' ? '官方来源' : '安装'}
      </button>
    </article>
  );
}
