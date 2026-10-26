// Open /tests/model-settings-ui.html with the Vite dev server running.
// Uses real components with in-memory drafts only. Never reads or writes native configuration.
import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ModelPicker } from '../src/app/ModelPicker';
import {
  CodexModelSettings,
  emptySettingDraft,
  sameDraft,
  settingsPayload,
} from '../src/app/CodexModelSettings';
import type { CodexSettingDraft } from '../src/app/CodexModelSettings';
import { ModelReasoningSettings } from '../src/app/ModelReasoningSettings';
import { OpenCodeReasoningLevels } from '../src/app/OpenCodeReasoningLevels';
import { ContextWindowSettings } from '../src/app/ContextWindowSettings';
import {
  selectedEffortChanges,
  selectedContextChanges,
  selectedReasoningLevelChanges,
} from '../src/lib/model-settings';
import '../src/app/native.css';
import '../src/app/theme.css';

const originals: Record<string, CodexSettingDraft> = {
  sol: {
    contextWindow: '',
    levels: ['low', 'medium', 'high', 'xhigh', 'max'],
    defaultLevel: 'medium',
  },
};
const snapshots: Record<string, { model: string; models: string[]; payload: unknown }> = {};
let writes = 0;

function Fixture({ tool }: { tool: 'Codex' | 'OpenCode' }) {
  const [model, setModel] = useState('sol');
  const [models, setModels] = useState(['beam-free', 'luna', 'sol']);
  const [drafts, setDrafts] = useState(originals);
  const [efforts, setEfforts] = useState<Record<string, string>>({});
  const [contexts, setContexts] = useState<Record<string, string>>({});
  const [levels, setLevels] = useState<Record<string, string>>({});
  snapshots[tool] = {
    model,
    models,
    payload:
      tool === 'Codex'
        ? settingsPayload(drafts, models)
        : {
            efforts: selectedEffortChanges(efforts, models, model),
            contexts: selectedContextChanges(contexts, models, model),
            levels: selectedReasoningLevelChanges(levels, models, model),
          },
  };
  return (
    <section className="n-panel" id={tool}>
      <h2 style={{ marginBottom: 20 }}>{tool}</h2>
      <section className="n-api-section">
        <ModelPicker
          id={`${tool}-model`}
          value={model}
          onChange={setModel}
          selectedModels={models}
          onModelsChange={setModels}
          baseUrl=""
          apiKey=""
          api="openai"
          required
          renderModelSummary={() => '推理自动 · 上下文沿用原值'}
          renderModelSettings={(id, index) =>
            tool === 'Codex' ? (
              <CodexModelSettings
                id={`codex-${index}`}
                model={id}
                draft={drafts[id] ?? emptySettingDraft}
                original={originals[id]}
                onChange={(value) => {
                  writes++;
                  setDrafts((old) => ({ ...old, [id]: value }));
                }}
                onReset={() =>
                  setDrafts((old) => ({ ...old, [id]: originals[id] ?? emptySettingDraft }))
                }
              />
            ) : (
              <div className="n-opencode-model-editor">
                <div className="n-model-editor-grid">
                  <ModelReasoningSettings
                    inline
                    id={`effort-${index}`}
                    model={id}
                    original="max"
                    value={efforts[id] ?? 'max'}
                    api="openai"
                    onChange={(value) => {
                      writes++;
                      setEfforts((old) => ({ ...old, [id]: value }));
                    }}
                    onReset={() => setEfforts({})}
                  />
                  <ContextWindowSettings
                    inline
                    id={`context-${index}`}
                    scope={`${id} 的`}
                    original={null}
                    draft={contexts[id]}
                    onChange={(value) => {
                      writes++;
                      setContexts((old) => ({ ...old, [id]: value }));
                    }}
                    onReset={() => setContexts({})}
                  />
                </div>
                <OpenCodeReasoningLevels
                  inline
                  id={`levels-${index}`}
                  model={id}
                  original={['low', 'medium', 'high', 'xhigh', 'max']}
                  draft={levels[id]}
                  disabled={false}
                  conflict={false}
                  editable
                  onChange={(value) => {
                    writes++;
                    setLevels((old) => ({ ...old, [id]: value }));
                  }}
                  onReset={() => setLevels({})}
                />
              </div>
            )
          }
        />
      </section>
    </section>
  );
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 30));
const check = (condition: unknown, message: string) => {
  if (!condition) throw new Error(message);
};
async function click(selector: string) {
  const button = document.querySelector<HTMLButtonElement>(selector);
  check(button && !button.disabled, `Missing/enabled button: ${selector}`);
  button!.click();
  await tick();
}
async function input(selector: string, value: string) {
  const element = document.querySelector<HTMLInputElement>(selector)!;
  check(element, `Missing input: ${selector}`);
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(element, value);
  element.dispatchEvent(new Event('input', { bubbles: true }));
  await tick();
}

async function run() {
  await tick();
  check(sameDraft(undefined, emptySettingDraft), 'New untouched model must not show pending edits');
  for (const tool of ['Codex', 'OpenCode']) {
    const before = JSON.stringify(snapshots[tool].payload);
    await click(`#${tool} [aria-label="配置 beam-free"]`);
    await click(`#${tool} [aria-label="配置 luna"]`);
    check(
      document.querySelectorAll(`#${tool} .n-model-settings:not([hidden])`).length === 1,
      'Only one editor should be open',
    );
    check(
      JSON.stringify(snapshots[tool].payload) === before && writes === 0,
      'Opening/switching must not create edits',
    );
  }
  await input('#codex-1-context', '200000');
  await click('#Codex [aria-label="配置 sol"]');
  await click('#Codex [aria-label="配置 luna"]');
  check(
    document.querySelector<HTMLInputElement>('#codex-1-context')!.value === '200000',
    'Draft must survive switching editors',
  );
  check(
    (snapshots.Codex.payload as Record<string, { contextWindow: number }>).luna.contextWindow ===
      200000,
    'Codex context must serialize as tokens',
  );
  await click('#Codex [aria-label="配置 sol"]');
  const medium = [...document.querySelectorAll<HTMLLabelElement>('#Codex .n-codex-levels label')]
    .find((label) => label.textContent === 'medium')!
    .querySelector<HTMLInputElement>('input')!;
  medium.click();
  await tick();
  check(
    (snapshots.Codex.payload as Record<string, { defaultLevel: unknown }>).sol.defaultLevel ===
      null,
    'Removing the selected default level must return the model to automatic',
  );
  await input('#Codex [aria-label="给 sol 添加自定义思考等级"]', 'future-level');
  await click('#Codex .n-codex-model-editor .n-model-manual button');
  const defaultLevel = document.querySelector<HTMLSelectElement>('#codex-2-default')!;
  defaultLevel.value = 'future-level';
  defaultLevel.dispatchEvent(new Event('change', { bubbles: true }));
  await tick();
  const codexSol = (
    snapshots.Codex.payload as Record<string, { defaultLevel: string; reasoningLevels: string[] }>
  ).sol;
  check(
    codexSol.defaultLevel === 'future-level' && codexSol.reasoningLevels.includes('future-level'),
    'Custom levels must remain selectable as defaults',
  );
  const reset = [
    ...document.querySelectorAll<HTMLButtonElement>('#Codex .n-codex-model-editor button'),
  ].find((button) => button.textContent?.includes('撤销'))!;
  reset.click();
  await tick();
  check(
    (snapshots.Codex.payload as Record<string, { defaultLevel: string }>).sol.defaultLevel ===
      'medium',
    'Undo must restore the published default',
  );
  await input('#effort-1', 'xhigh');
  await input('#context-1', '400000');
  await input('#levels-1', 'low, high, custom');
  await click('#OpenCode [aria-label="配置 sol"]');
  await click('#OpenCode [aria-label="配置 luna"]');
  check(
    JSON.stringify(snapshots.OpenCode.payload) ===
      JSON.stringify({
        efforts: { luna: 'xhigh' },
        contexts: { luna: 400000 },
        levels: { luna: ['low', 'high', 'custom'] },
      }),
    'OpenCode defaults, menu levels, and contexts must remain separate patches',
  );
  await input('#context-1', '');
  check(
    (snapshots.OpenCode.payload as { contexts: Record<string, unknown> }).contexts.luna === null,
    'Blank context must clear explicitly',
  );
  check(
    document.querySelector<HTMLButtonElement>('#OpenCode [aria-label="移除 sol"]')!.disabled,
    'Default model must remain protected',
  );
  const select = document.querySelector<HTMLSelectElement>('#OpenCode-model')!;
  select.value = 'luna';
  select.dispatchEvent(new Event('change', { bubbles: true }));
  await tick();
  check(snapshots.OpenCode.model === 'luna', 'Default selector must change the default');
  await click('#OpenCode [aria-label="移除 sol"]');
  check(
    !snapshots.OpenCode.models.includes('sol'),
    'Old default must be removable after switching',
  );
  await click('#Codex [aria-controls="Codex-model-add"]');
  await input('#Codex [aria-label="手动添加模型 ID"]', 'custom-model');
  const add = [
    ...document.querySelectorAll<HTMLButtonElement>('#Codex .n-model-add .n-model-manual button'),
  ].find((button) => button.textContent?.includes('添加'))!;
  add.click();
  await tick();
  check(
    snapshots.Codex.models.includes('custom-model'),
    'Manual model addition must remain available',
  );
  return { passed: true, snapshots };
}

declare global {
  interface Window {
    runModelSettingsUiTests: typeof run;
  }
}
window.runModelSettingsUiTests = run;
document.documentElement.dataset.workspace = 'codex';
document.body.style.cssText = 'max-width: 1050px; margin: 24px auto; padding: 0 20px';
createRoot(document.getElementById('root')!).render(
  <>
    <Fixture tool="Codex" />
    <Fixture tool="OpenCode" />
  </>,
);
