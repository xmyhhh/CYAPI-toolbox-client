import { useCallback, useEffect, useRef, useState } from 'react';
import { nativeAvailable, nativeCall, SERVER_URL } from '../../lib/api/native';
import type {
  ConfigValue,
  IntegrationAction,
  IntegrationDescriptor,
  IntegrationManifest,
  IntegrationServicesView,
  IntegrationView,
} from '../../lib/api/native';
import {
  integrationManifest,
  integrationPreparedNotice,
  integrationServiceView,
  integrationServiceFailure,
} from './model';

type Workspace = 'codex' | 'claude' | 'opencode';
export function useIntegrations(workspace: Workspace, onInstallationsChanged: () => Promise<void>) {
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

  const hasService = !!view?.entries.some((entry) => entry.service);
  useEffect(() => {
    if (!nativeAvailable || !hasService || busy) return;
    let active = true;
    let inFlight = false;
    const refresh = async () => {
      if (!active || inFlight || document.visibilityState === 'hidden') return;
      inFlight = true;
      const token = generation.current;
      try {
        // Only probe the addon: no catalog downloads or full installation scan.
        const result = await nativeCall<IntegrationServicesView>('integration-services-read', {
          workspace,
        });
        if (active && generation.current === token)
          setView((previous) => previous && integrationServiceView(previous, result));
      } catch {
        if (active && generation.current === token)
          setView(
            (previous) =>
              previous &&
              integrationServiceFailure(
                previous,
                '服务状态检测失败，请点击「检查状态」重试；这不代表 Blender 服务未启动。',
              ),
          );
      } finally {
        inFlight = false;
      }
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 5000);
    const onFocus = () => void refresh();
    window.addEventListener('focus', onFocus);
    document.addEventListener('visibilitychange', onFocus);
    return () => {
      active = false;
      window.clearInterval(timer);
      window.removeEventListener('focus', onFocus);
      document.removeEventListener('visibilitychange', onFocus);
    };
  }, [workspace, busy, hasService]);

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
      if (
        (action === 'integration-connect' || action === 'integration-adopt') &&
        descriptor &&
        'manifest' in result
      ) {
        setNotice(
          integrationPreparedNotice(
            descriptor,
            result.entries.find((entry) => entry.id === descriptor.id),
          ),
        );
      } else if (action === 'integration-disconnect')
        setNotice({
          text: descriptor?.skillId
            ? '技能已卸载，原文件已备份。MCP 配置与共享运行环境保留，可在 MCP 管理中移除。'
            : '接入配置已移除。',
        });
      else if (action === 'integrations-refresh') setNotice({ text: '接入目录已刷新。' });
    } catch (error) {
      if (generation.current === token)
        setNotice({ text: String(error instanceof Error ? error.message : error), error: true });
    } finally {
      // A failed MCP step may follow a successful Skill installation. Always
      // reload both snapshots, never leave another page showing the old state.
      const mutating = action.startsWith('integration-');
      const refreshed = mutating
        ? await Promise.allSettled([read(), onInstallationsChanged()])
        : [];
      if (generation.current === token) {
        const next = refreshed[0];
        if (next?.status === 'fulfilled' && 'manifest' in next.value) setView(next.value);
        if (refreshed.some((result) => result.status === 'rejected'))
          setNotice((previous) => ({
            text: `${previous?.text ?? ''} 状态刷新失败，请点击检查状态重试。`.trim(),
            error: true,
          }));
      }
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
