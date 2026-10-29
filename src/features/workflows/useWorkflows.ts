import { useCallback, useEffect, useRef, useState } from 'react';
import { WorkflowError, workflowApi } from './api';
import {
  addKey as buildKey,
  loadKeys,
  saveKeys,
  type StoredKey,
} from './keyStore';
import type {
  WorkflowCatalog,
  WorkflowDraft,
  WorkflowMode,
  WorkflowState,
  WorkflowTask,
  WorkflowTemplate,
} from './types';

export interface Snapshot {
  categories: WorkflowState['categories'];
  templates: WorkflowTemplate[];
  drafts: WorkflowDraft[];
  tasks: WorkflowTask[];
  mode: WorkflowMode;
  catalogRevision?: string;
}

export const emptySnapshot = (): Snapshot => ({
  categories: [],
  templates: [],
  drafts: [],
  tasks: [],
  mode: 'mock',
});

export interface Notice {
  id: number;
  text: string;
  error?: boolean;
}

export type WorkflowCall = <T>(action: (key: string) => Promise<T>) => Promise<T>;

const errorText = (error: unknown) =>
  error instanceof Error ? error.message : '操作失败，请稍后重试';

const isState = (value: WorkflowState | WorkflowCatalog): value is WorkflowState =>
  'drafts' in value;

export function useWorkflows() {
  const initial = useRef(loadKeys());
  const [keys, setKeys] = useState<StoredKey[]>(initial.current.keys);
  const [active, setActive] = useState<StoredKey | null>(
    initial.current.keys.find((entry) => entry.id === initial.current.activeId) || null,
  );
  const keysRef = useRef(keys);
  const activeRef = useRef(active);
  keysRef.current = keys;
  activeRef.current = active;
  const [snapshot, setSnapshot] = useState<Snapshot>(emptySnapshot);
  const [online, setOnline] = useState(true);
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const noticeTimer = useRef<number | undefined>(undefined);

  const notify = useCallback((text: string, error = false) => {
    window.clearTimeout(noticeTimer.current);
    setNotice({ id: Date.now(), text, error });
    noticeTimer.current = window.setTimeout(() => setNotice(null), 4500);
  }, []);

  useEffect(
    () => () => {
      window.clearTimeout(noticeTimer.current);
    },
    [],
  );

  const commit = useCallback((nextKeys: StoredKey[], nextActive: StoredKey | null) => {
    keysRef.current = nextKeys;
    activeRef.current = nextActive;
    setKeys(nextKeys);
    setActive(nextActive);
    saveKeys(
      nextKeys,
      nextActive && !nextActive.temporary ? nextActive.id : null,
    );
  }, []);

  const invalidate = useCallback(
    (message = 'Key 已失效，请重新选择') => {
      activeRef.current = null;
      setActive(null);
      saveKeys(keysRef.current, null);
      setSnapshot(emptySnapshot());
      setOnline(true);
      notify(message, true);
    },
    [notify],
  );

  const refresh = useCallback(
    async (override?: string, quiet = false): Promise<Snapshot | null> => {
      const key = override ?? activeRef.current?.key;
      try {
        const next: WorkflowState | WorkflowCatalog = key
          ? await workflowApi.state(key)
          : await workflowApi.catalog();
        const state = isState(next) ? next : null;
        const snapshot: Snapshot = {
          categories: next.categories || [],
          templates: next.templates || [],
          drafts: state?.drafts || [],
          tasks: state?.tasks || [],
          mode: next.mode || 'mock',
          catalogRevision: next.catalogRevision,
        };
        setSnapshot(snapshot);
        setOnline(true);
        setReady(true);
        return snapshot;
      } catch (error) {
        if (error instanceof WorkflowError && error.status === 401 && key) {
          invalidate();
          setReady(true);
          return null;
        }
        setOnline(false);
        setReady(true);
        if (!quiet) notify(errorText(error), true);
        return null;
      }
    },
    [invalidate, notify],
  );

  const activeKey = active?.key || '';

  useEffect(() => {
    let disposed = false;
    const tick = () => {
      if (!disposed) void refresh(undefined, true);
    };
    const timer = window.setInterval(tick, activeKey ? 1500 : 15000);
    void refresh(undefined, true);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [activeKey, refresh]);

  const call = useCallback<WorkflowCall>(
    async (action) => {
      const key = activeRef.current?.key;
      if (!key) throw new Error('请先在 Key 管理中激活一个工作流 Key');
      try {
        return await action(key);
      } catch (error) {
        if (error instanceof WorkflowError && error.status === 401) invalidate();
        throw error;
      }
    },
    [invalidate],
  );

  const activate = useCallback(
    async (entry: StoredKey) => {
      setBusy(true);
      try {
        const session = await workflowApi.verify(entry.key);
        const nextKeys = entry.temporary
          ? keysRef.current
          : keysRef.current.some((item) => item.id === entry.id)
            ? keysRef.current
            : [...keysRef.current, entry];
        commit(nextKeys, entry);
        setSnapshot(emptySnapshot());
        await refresh(entry.key);
        return session;
      } finally {
        setBusy(false);
      }
    },
    [commit, refresh],
  );

  const activateStored = useCallback(
    async (id: string) => {
      const entry = keysRef.current.find((item) => item.id === id);
      if (!entry) throw new Error('Key 配置不存在，请重新添加');
      const session = await activate(entry);
      notify(`已激活 ${entry.name}`);
      return session;
    },
    [activate, notify],
  );

  const addAndActivate = useCallback(
    async (name: string, value: string, remember: boolean) => {
      const entry = remember
        ? buildKey(keysRef.current, name, value)
        : {
            id: `temporary-${crypto.randomUUID()}`,
            name: name.trim().slice(0, 40) || '临时 Key',
            key: value.trim(),
            createdAt: Date.now(),
            temporary: true,
          };
      const session = await activate(entry);
      notify(remember ? 'Key 已保存并激活' : '临时 Key 已激活');
      return session;
    },
    [activate, notify],
  );

  const renameKey = useCallback(
    (id: string, name: string) => {
      const trimmed = name.trim().slice(0, 40);
      if (!trimmed) throw new Error('名称不能为空');
      const next = keysRef.current.map((entry) =>
        entry.id === id ? { ...entry, name: trimmed } : entry,
      );
      const current = activeRef.current;
      commit(
        next,
        current && current.id === id ? { ...current, name: trimmed } : current,
      );
      notify('Key 名称已更新');
    },
    [commit, notify],
  );

  const removeKey = useCallback(
    async (id: string) => {
      const wasActive = activeRef.current?.id === id;
      const next = keysRef.current.filter((entry) => entry.id !== id);
      commit(next, wasActive ? null : activeRef.current);
      if (wasActive) {
        setSnapshot(emptySnapshot());
        await refresh();
        notify('Key 配置已移除');
      } else {
        notify('Key 配置已移除');
      }
    },
    [commit, notify, refresh],
  );

  const deactivate = useCallback(async () => {
    commit(keysRef.current, null);
    setSnapshot(emptySnapshot());
    await refresh();
    notify('已停用当前 Key');
  }, [commit, notify, refresh]);

  return {
    keys,
    active,
    activeKey,
    snapshot,
    mode: snapshot.mode,
    online,
    ready,
    busy,
    notice,
    notify,
    refresh,
    call,
    activateStored,
    addAndActivate,
    renameKey,
    removeKey,
    deactivate,
    setBusy,
    dismissNotice: () => setNotice(null),
  };
}

export type WorkflowsController = ReturnType<typeof useWorkflows>;
