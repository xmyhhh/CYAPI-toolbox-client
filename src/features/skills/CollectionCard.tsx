import { useEffect, useId, useState } from 'react';
import type { ReactNode } from 'react';
import { ChevronDown, Package } from 'lucide-react';
import type { SkillCollection } from './collections';

export function CollectionCard({
  collection,
  count,
  description,
  meta,
  searching = false,
  busy,
  onManage,
  children,
}: {
  collection: SkillCollection;
  count: number;
  description?: string;
  meta: ReactNode;
  searching?: boolean;
  busy: boolean;
  onManage?: () => void;
  children: ReactNode;
}) {
  const [expanded, setExpanded] = useState(searching);
  const membersId = useId();
  useEffect(() => setExpanded(searching), [searching]);
  return (
    <section className="n-collection" aria-label={collection.title}>
      <div className="n-skill-row n-collection-header">
        <span className="n-skill-icon">
          <Package size={24} strokeWidth={1.6} />
        </span>
        <div className="n-skill-info">
          <h2>
            {collection.title}
            <span className="n-pill">技能套件</span>
          </h2>
          <p title={description ?? collection.description}>
            {description ?? collection.description}
          </p>
          <div className="n-skill-meta">{meta}</div>
        </div>
        <div className="n-collection-actions">
          <button
            className="n-button"
            aria-expanded={expanded}
            aria-controls={membersId}
            onClick={() => setExpanded(!expanded)}
          >
            <ChevronDown size={15} className={expanded ? 'n-chevron-open' : ''} />
            {expanded ? '收起模块' : `展开 ${count} 个模块`}
          </button>
          {onManage && (
            <button className="n-button n-primary" disabled={busy} onClick={onManage}>
              <Package size={15} />
              管理套件
            </button>
          )}
        </div>
      </div>
      {expanded && (
        <div className="n-collection-members" id={membersId}>
          {children}
        </div>
      )}
    </section>
  );
}
