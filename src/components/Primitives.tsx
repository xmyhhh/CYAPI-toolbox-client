import { useEffect, useId, useRef } from 'react';
import type { ReactNode } from 'react';
import {
  Blocks,
  Braces,
  Check,
  CodeXml,
  FileText,
  GitBranch,
  Palette,
  ScanSearch,
  Sparkles,
  X,
} from 'lucide-react';
import type { Skill, TargetId } from '../skills/types';

const icons = {
  terminal: CodeXml,
  palette: Palette,
  file: FileText,
  git: GitBranch,
  search: ScanSearch,
  blocks: Blocks,
};

export function SkillAvatar({ skill, large = false }: { skill: Skill; large?: boolean }) {
  const Icon = icons[skill.icon];
  return (
    <span className={`skill-avatar avatar-${skill.icon}${large ? ' large' : ''}`}>
      <Icon size={large ? 29 : 23} strokeWidth={1.65} />
    </span>
  );
}

export function AgentLogo({ target }: { target: TargetId }) {
  return (
    <span aria-hidden="true" className={`agent-logo ${target}`}>
      {target === 'codex' ? (
        <CodeXml size={17} />
      ) : target === 'claude' ? (
        <Sparkles size={18} />
      ) : (
        <Braces size={17} />
      )}
    </span>
  );
}

export function Modal({
  title,
  children,
  onClose,
  wide = false,
  busy = false,
  dismissWhileBusy = false,
  className = '',
  descriptionId,
}: {
  title: ReactNode;
  children: ReactNode;
  onClose: () => void;
  wide?: boolean;
  busy?: boolean;
  /** Keeps the dialog closable while `busy`, so a task can move to the background. */
  dismissWhileBusy?: boolean;
  className?: string;
  descriptionId?: string;
}) {
  const titleId = useId();
  const panel = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  const busyRef = useRef(busy);
  const dismissRef = useRef(dismissWhileBusy);
  onCloseRef.current = onClose;
  busyRef.current = busy;
  dismissRef.current = dismissWhileBusy;
  const locked = busy && !dismissWhileBusy;
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const node = panel.current;
    node?.focus();
    const keydown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !(busyRef.current && !dismissRef.current)) {
        onCloseRef.current();
      }
      if (event.key !== 'Tab' || !node) return;
      const focusables = Array.from(
        node.querySelectorAll<HTMLElement>(
          'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex="0"]',
        ),
      ).filter((element) => element.getClientRects().length > 0);
      if (!focusables.length) {
        event.preventDefault();
        return;
      }
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (event.shiftKey && (document.activeElement === first || document.activeElement === node)) {
        event.preventDefault();
        last.focus();
      } else if (
        !event.shiftKey &&
        (document.activeElement === last || document.activeElement === node)
      ) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener('keydown', keydown);
    return () => {
      document.removeEventListener('keydown', keydown);
      previous?.focus();
    };
  }, []);
  return (
    <div
      className="modal-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !locked) onClose();
      }}
    >
      <div
        className={`modal ${wide ? 'modal-wide' : ''} ${className}`}
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={descriptionId}
        tabIndex={-1}
      >
        <div className="modal-header">
          <h2 id={titleId}>{title}</h2>
          <button className="icon-button" onClick={onClose} disabled={locked} aria-label="关闭对话框">
            <X size={20} />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}

export function RestartPrompt({
  target,
  onClose,
  children,
}: {
  target: string;
  onClose: () => void;
  children?: ReactNode;
}) {
  const descriptionId = useId();
  return (
    <Modal
      title={
        <>
          <span className="n-restart-success" aria-hidden="true">
            <Check size={18} strokeWidth={2.4} />
          </span>
          配置已保存
        </>
      }
      onClose={onClose}
      className="n-restart-dialog"
      descriptionId={descriptionId}
    >
      <div className="n-restart-body" id={descriptionId}>
        {children ?? (
          <>
            <p>请完全关闭 {target}。</p>
            <p>重新打开后，新配置即可生效。</p>
          </>
        )}
      </div>
      <div className="n-dialog-footer">
        <button type="button" className="n-button n-primary" onClick={onClose}>
          知道了
        </button>
      </div>
    </Modal>
  );
}

export function EmptyState({
  icon,
  title,
  description,
  action,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <span className="empty-icon">{icon}</span>
      <h3>{title}</h3>
      <p>{description}</p>
      {action}
    </div>
  );
}

export const formatDate = (value: string) =>
  new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(new Date(value));
