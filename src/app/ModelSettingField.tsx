import { useState } from 'react';
import type { ReactNode } from 'react';
import { ChevronDown } from 'lucide-react';

/** Shared presentation only; opening a field never changes its configuration. */
export function ModelSettingField({
  id,
  label,
  summary,
  changed,
  disabled,
  inline = false,
  children,
}: {
  id: string;
  label: string;
  summary: ReactNode;
  changed?: boolean;
  disabled?: boolean;
  inline?: boolean;
  children: ReactNode;
}) {
  const [editing, setEditing] = useState(false);
  return (
    <div className={`n-model-reasoning${inline ? ' n-setting-inline' : ''}`}>
      <div className="n-model-effort-summary">
        <span>
          {label}
          {!inline && (
            <>
              ：<strong>{summary}</strong>
            </>
          )}
        </span>
        {changed && <span className="n-model-pending">待保存</span>}
        {!inline && (
          <button
            type="button"
            className="n-inline-action"
            disabled={disabled}
            aria-expanded={editing}
            aria-controls={`${id}-editor`}
            aria-label={`配置${label}`}
            onClick={() => setEditing(!editing)}
          >
            {editing ? '收起' : '配置'}
            <ChevronDown size={12} className={editing ? 'n-chevron-open' : ''} />
          </button>
        )}
      </div>
      {(inline || editing) && (
        <div className="n-model-effort-editor n-config-field" id={`${id}-editor`}>
          {children}
        </div>
      )}
    </div>
  );
}

export function SettingHelp({ children }: { children: ReactNode }) {
  return (
    <details className="n-setting-help">
      <summary>设置说明</summary>
      <div>{children}</div>
    </details>
  );
}
