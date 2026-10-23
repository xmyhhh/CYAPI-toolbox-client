import { useCallback, useEffect, useRef, useState } from 'react';
import { nativeAvailable, nativeCall, SERVER_URL } from '../../lib/api/native';
import type {
  ConfigValue,
  IntegrationAction,
  IntegrationDescriptor,
  IntegrationManifest,
  IntegrationView,
} from '../../lib/api/native';
import { integrationManifest } from './model';

type Workspace = 'codex' | 'claude' | 'opencode';
export function useIntegrations(workspace: Workspace) {
  const [view, setView] = useState<IntegrationView>();
  const [preview, setPreview] = useState<IntegrationManifest>();
  const [busy, setBusy] = useState('');
  const [notice, setNotice] = useState<{ text: string; error?: boolean }>();
  const [outputs, setOutputs] = useState<Record<string, unknown>>({});
  const generation = useRef(0);
  const read = useCallback(
    async (action = 'integrations-read') => {
      if (nativeAvailable) return nativeCall<IntegrationView>(action, { workspace });
      const response = await fetch(`${SERVER_URL}/api/integrations`);
      if (!response.ok) throw new Error('接入目录暂不可用，请稍后刷新。');
      return integrationManifest(await response.json());
    },
    [workspace],
  );
  useEffect(() => {
    const token = ++generation.current;
    setView(undefined);
    setBusy('integrations-read');
    setNotice(undefined);
    setOutputs({});
    void read()
      .then((result) => {
        if (generation.current !== token) return;
        if ('manifest' in result) setView(result);
        else setPreview(result);
      })
      .catch((error) => {
        if (generation.current === token)
          setNotice({ text: String(error instanceof Error ? error.message : error), error: true });
      })
      .finally(() => {
        if (generation.current === token) setBusy('');
      });
    return () => {
      generation.current += 1;
    };
  }, [read]);

  async function update(action: string, descriptor?: IntegrationDescriptor) {
    const token = ++generation.current;
    setBusy(action);
    setNotice(undefined);
    try {
      const result =
        action === 'integrations-read' || action === 'integrations-refresh'
          ? await read(action)
          : await nativeCall<IntegrationView>(action, {
              workspace,
              integrationId: descriptor?.id,
              configurationRevision: view?.entries.find((entry) => entry.id === descriptor?.id)
                ?.mcpRevision,
            });
      if (generation.current !== token) return;
      if ('manifest' in result) setView(result);
      else setPreview(result);
      if (action === 'integration-connect' || action === 'integration-adopt') {
        setNotice({
          text: '所需能力已准备，请按教程检查软件，并在 AI 工具的新会话中完成首次验证。',
        });
      } else if (action === 'integration-disconnect')
        setNotice({
          text: descriptor?.skillId ? '技能已卸载，原文件已备份。' : '接入配置已移除。',
        });
      else if (action === 'integrations-refresh') setNotice({ text: '接入目录已刷新。' });
    } catch (error) {
      if (generation.current === token)
        setNotice({ text: String(error instanceof Error ? error.message : error), error: true });
    } finally {
      if (generation.current === token) setBusy('');
    }
  }
  async function run(
    descriptor: IntegrationDescriptor,
    action: IntegrationAction,
    values: Record<string, ConfigValue> = {},
  ) {
    const token = ++generation.current;
    setBusy(action.id);
    setNotice(undefined);
    try {
      const result = await nativeCall<unknown>('integration-action', {
        workspace,
        integrationId: descriptor.id,
        actionId: action.id,
        values,
      });
      if (generation.current !== token) return;
      setOutputs((previous) => ({ ...previous, [`${descriptor.id}:${action.id}`]: result }));
      const next = await read();
      if (generation.current === token && 'manifest' in next) setView(next);
    } catch (error) {
      if (generation.current === token)
        setNotice({ text: String(error instanceof Error ? error.message : error), error: true });
    } finally {
      if (generation.current === token) setBusy('');
    }
  }
  return {
    view: view?.workspace && view.workspace !== workspace ? undefined : view,
    manifest: view?.manifest ?? preview,
    busy,
    notice,
    setNotice,
    outputs,
    update,
    run,
  };
}
