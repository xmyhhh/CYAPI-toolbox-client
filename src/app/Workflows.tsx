import { useEffect, useRef, useState } from 'react';
import { LoaderCircle, RefreshCw } from 'lucide-react';
import { nativeAvailable } from '../lib/api/native';
import { syncWorkflowView, WORKFLOW_URL } from '../lib/api/workflows';
import type { WorkflowLayout } from '../lib/api/workflows';
import { isCreationSection } from '../lib/workbenches';
import type { CreationSection } from '../lib/workbenches';
import { resolveTheme } from '../lib/theme';
import type { ThemePreference } from '../lib/theme';
import './workflows.css';

export function Workflows({
  initialSection,
  navigation,
  onSectionChange,
  onLockedChange,
  theme,
  visible,
}: {
  initialSection: CreationSection;
  navigation?: { section: CreationSection; id: number };
  onSectionChange: (section: CreationSection) => void;
  onLockedChange: (locked: boolean) => void;
  theme: ThemePreference;
  visible: boolean;
}) {
  const host = useRef<HTMLDivElement>(null);
  const session = useRef(crypto.randomUUID());
  const current = useRef({
    initialSection,
    navigation,
    onSectionChange,
    onLockedChange,
    theme,
    visible,
  });
  current.current = { initialSection, navigation, onSectionChange, onLockedChange, theme, visible };
  const update = useRef<() => void>(() => {});
  const [ready, setReady] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!nativeAvailable) return;
    let active = true,
      inFlight = false,
      again = false;
    let last: WorkflowLayout;
    const sync = async () => {
      if (!active) return;
      if (inFlight) {
        again = true;
        return;
      }
      const props = current.current;
      const rect = host.current?.getBoundingClientRect();
      const shown = Boolean(props.visible && rect && rect.width > 0 && rect.height > 0);
      last = {
        visible: shown,
        x: Math.max(0, rect?.x || 0),
        y: Math.max(0, rect?.y || 0),
        width: Math.max(0, rect?.width || 0),
        height: Math.max(0, rect?.height || 0),
        section: props.navigation?.section || props.initialSection,
        theme: resolveTheme(props.theme),
        navigationId: `${session.current}-${props.navigation?.id || 0}`,
      };
      inFlight = true;
      try {
        const result = await syncWorkflowView(last);
        if (!active) return;
        if (result.url !== WORKFLOW_URL) throw new Error('工作流服务地址校验失败。');
        if (shown) {
          setReady(result.view.ready);
          if (isCreationSection(result.view.section)) props.onSectionChange(result.view.section);
          props.onLockedChange(result.view.locked);
        }
        setError('');
      } catch (cause) {
        if (active) setError(cause instanceof Error ? cause.message : String(cause));
      } finally {
        inFlight = false;
        if (again && active) {
          again = false;
          void sync();
        }
      }
    };
    update.current = () => void sync();
    const resize = new ResizeObserver(update.current);
    if (host.current) resize.observe(host.current);
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    media.addEventListener('change', update.current);
    window.addEventListener('resize', update.current);
    const timer = setInterval(update.current, 750);
    void sync();
    return () => {
      active = false;
      clearInterval(timer);
      resize.disconnect();
      media.removeEventListener('change', update.current);
      window.removeEventListener('resize', update.current);
      if (last) void syncWorkflowView({ ...last, visible: false }).catch(() => {});
    };
  }, []);
  useEffect(() => {
    update.current();
  }, [visible, navigation, theme]);

  return (
    <div className="wf-workspace" ref={host} aria-label="工作流服务">
      {(!ready || error) && (
        <section className="wf-offline" role="status">
          {error ? <RefreshCw size={24} /> : <LoaderCircle className="n-spin" size={24} />}
          <h2>{error ? '工作流服务暂不可用' : nativeAvailable ? '正在连接工作流服务…' : '打开工作流服务'}</h2>
          <p>{error || '无需 Key 即可浏览功能与流程；运行前请在 Key 管理中激活。'}</p>
          <a className="n-button" href={WORKFLOW_URL} target="_blank" rel="noreferrer">
            在浏览器中打开
          </a>
          <p>关闭客户端或画布不会停止服务，已提交的任务继续执行。</p>
          <p>工作流 Key 与工具接入独立；任务费用和执行模式以工作流页面为准。</p>
        </section>
      )}
    </div>
  );
}
