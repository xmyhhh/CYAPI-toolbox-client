import type { ReactNode } from 'react';
import {
  ArrowUpRight,
  Check,
  CheckCircle2,
  Copy,
  Download,
  FolderOpen,
  Globe2,
  LoaderCircle,
  Monitor,
  Moon,
  Palette,
  RefreshCw,
  Sun,
  WifiOff,
  Zap,
} from 'lucide-react';
import type { UpdateStatus } from '../lib/api/update';
import type { ThemePreference } from '../lib/theme';
import { APP_NAME, COPYRIGHT_TEXT, DOWNLOAD_PAGE, OFFICIAL_SITE } from '../lib/app-info';
import './app-settings.css';

const themes = [
  { value: 'system', label: '跟随系统', description: '随系统自动切换', Icon: Monitor },
  { value: 'light', label: '浅色', description: '明亮、清晰的界面', Icon: Sun },
  { value: 'dark', label: '深色', description: '柔和的深色界面', Icon: Moon },
] as const;

export function AppSettings({
  appVersion,
  theme,
  onThemeChange,
  dataPath,
  onCopy,
  onOpenSite,
  updates,
}: {
  appVersion: string;
  theme: ThemePreference;
  onThemeChange: (theme: ThemePreference) => void;
  dataPath?: string;
  onCopy: (text: string) => void;
  onOpenSite: (url: string) => void;
  updates: {
    status?: UpdateStatus;
    checking: boolean;
    error: string;
    checkedAt?: number;
    busy: boolean;
    onCheck: () => void;
    banner: ReactNode;
  };
}) {
  const { status, checking, error, checkedAt } = updates;
  const UpdateIcon = checking
    ? LoaderCircle
    : error
      ? WifiOff
      : status?.hasUpdate
        ? Download
        : status
          ? CheckCircle2
          : RefreshCw;
  const updateTitle = checking
    ? '正在检查更新'
    : error
      ? '暂时无法检查更新'
      : status?.hasUpdate
        ? `新版本 v${status.latest} 已发布`
        : status
          ? '已是最新版本'
          : '检查可用更新';

  return (
    <div className="n-settings">
      <div className="n-settings-intro">
        <div>
          <h2>偏好与应用</h2>
          <p>调整工具箱外观，管理应用更新与本地数据。</p>
        </div>
        <span className="n-settings-scope">所有工作区通用</span>
      </div>

      <section
        className="n-settings-card n-settings-appearance"
        aria-labelledby="settings-appearance"
      >
        <div className="n-settings-section-title">
          <span className="n-settings-icon" aria-hidden="true">
            <Palette size={18} />
          </span>
          <div>
            <h2 id="settings-appearance">外观</h2>
            <p>选择你习惯的界面风格，立即生效并保存在本机。</p>
          </div>
        </div>
        <div className="n-settings-themes" role="group" aria-label="外观">
          {themes.map(({ value, label, description, Icon }) => (
            <button
              key={value}
              className={`n-settings-theme${theme === value ? ' is-selected' : ''}`}
              aria-pressed={theme === value}
              onClick={() => onThemeChange(value)}
            >
              <span className="n-settings-theme-top" aria-hidden="true">
                <Icon size={20} />
                <span className="n-settings-theme-check">
                  {theme === value && <Check size={12} />}
                </span>
              </span>
              <strong>{label}</strong>
              <small>{description}</small>
            </button>
          ))}
        </div>
      </section>

      <div className="n-settings-grid">
        <section
          className="n-settings-card n-settings-updates"
          aria-labelledby="settings-updates"
          aria-busy={checking}
        >
          <div className="n-settings-card-heading">
            <div className="n-settings-section-title">
              <span className="n-settings-icon" aria-hidden="true">
                <RefreshCw size={18} />
              </span>
              <h2 id="settings-updates">软件更新</h2>
            </div>
            <button
              className="n-button"
              disabled={checking || updates.busy}
              onClick={updates.onCheck}
            >
              <RefreshCw size={14} className={checking ? 'n-spin' : ''} />
              {checking ? '正在检查…' : '检查更新'}
            </button>
          </div>
          <div
            className={`n-settings-update-status${error && !checking ? ' is-error' : ''}`}
            role={error && !checking ? 'alert' : 'status'}
          >
            <UpdateIcon size={20} className={checking ? 'n-spin' : ''} aria-hidden="true" />
            <div>
              <strong>{updateTitle}</strong>
              <p>
                {checking
                  ? '正在获取最新版本信息，请稍候。'
                  : error
                    ? status?.source === 'cached'
                      ? '当前显示上次获取的信息，可稍后重试。'
                      : '请检查网络连接，或前往官网下载。'
                    : '打开设置时会自动检查，也可手动检查。'}
              </p>
            </div>
          </div>
          <dl className="n-settings-versions">
            <div>
              <dt>当前版本</dt>
              <dd>v{appVersion}</dd>
            </div>
            {status && !checking && (
              <div>
                <dt>{status.source === 'cached' ? '上次获取版本' : '最新版本'}</dt>
                <dd>v{status.latest}</dd>
              </div>
            )}
          </dl>
          {!checking && error && <p className="n-error-text">{error}</p>}
          {updates.banner}
          <p className="n-settings-caption">
            {!checking && checkedAt
              ? `上次检查：${new Date(checkedAt).toLocaleString('zh-CN', { hour12: false })}`
              : '更新不会更改你的模型接入配置。'}
          </p>
        </section>

        <section className="n-settings-card n-settings-storage" aria-labelledby="settings-storage">
          <div className="n-settings-section-title">
            <span className="n-settings-icon" aria-hidden="true">
              <FolderOpen size={18} />
            </span>
            <h2 id="settings-storage">本地数据</h2>
          </div>
          <p className="n-settings-description">工具箱的安装记录、下载缓存与备份保存在此目录。</p>
          <div className="n-settings-path">
            <span>数据目录</span>
            <div className="n-path-box">
              <code>{dataPath || '数据目录暂不可用'}</code>
              <button
                className="n-icon-button"
                disabled={!dataPath}
                onClick={() => dataPath && onCopy(dataPath)}
                aria-label="复制数据路径"
                title="复制路径"
              >
                <Copy size={16} />
              </button>
            </div>
          </div>
          <p className="n-settings-caption">各工具的配置仍保存在各自的配置目录中。</p>
        </section>
      </div>

      <section className="n-settings-card n-settings-about" aria-labelledby="settings-about">
        <div className="n-settings-product">
          <span className="n-settings-product-mark" aria-hidden="true">
            <Zap size={24} fill="currentColor" />
          </span>
          <div>
            <div className="n-settings-product-name">
              <h2 id="settings-about">{APP_NAME}</h2>
              <span>v{appVersion}</span>
            </div>
            <p>CYAPI 官方 AI 工作流工具箱 · 模型接入 / Skill / MCP / 软件接入</p>
          </div>
        </div>
        <div className="n-settings-links">
          <button className="n-button" onClick={() => onOpenSite(OFFICIAL_SITE)}>
            <Globe2 size={15} />
            官方网站
            <ArrowUpRight size={13} />
          </button>
          <button className="n-button" onClick={() => onOpenSite(DOWNLOAD_PAGE)}>
            <Download size={15} />
            下载客户端
            <ArrowUpRight size={13} />
          </button>
        </div>
        <div className="n-settings-about-footer">
          <span>{COPYRIGHT_TEXT}</span>
          <button
            onClick={() => onCopy(OFFICIAL_SITE)}
            aria-label="复制官网地址"
            title="复制官网地址"
          >
            {OFFICIAL_SITE}
            <Copy size={12} />
          </button>
        </div>
      </section>
    </div>
  );
}
