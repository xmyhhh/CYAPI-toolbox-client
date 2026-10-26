import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { mcpConfirmation } from '../src/features/mcp/actions.ts';
import type { McpLocalEntry } from '../src/lib/api/native.ts';

const external: McpLocalEntry = {
  name: 'personal-mcp',
  revision: 'entry-revision',
  transport: 'stdio',
  target: 'personal-tool --serve',
  enabled: true,
  managed: false,
  readOnlyReason: null,
  catalogId: null,
};

test('existing entries ask for consent in the action, not a separate adoption', () => {
  const disable = mcpConfirmation('mcp-disable', 'codex', external);
  assert.match(disable!, /停用 personal-mcp/);
  assert.match(disable!, /修改 Codex 中的现有配置/);
  assert.match(disable!, /自动备份/);
  const enable = mcpConfirmation('mcp-enable', 'opencode', external);
  assert.match(enable!, /启用 personal-mcp/);
  assert.match(enable!, /OpenCode/);
  assert.doesNotMatch(enable!, /接管/);
});

test('managed entries toggle without repeated confirmation', () => {
  for (const action of ['mcp-enable', 'mcp-disable'])
    assert.equal(mcpConfirmation(action, 'codex', { ...external, managed: true }), undefined);
});

test('removal always explains its scope for both existing and managed entries', () => {
  for (const managed of [true, false]) {
    const message = mcpConfirmation('mcp-remove', 'claude', { ...external, managed });
    assert.match(message!, /Claude Code 移除 personal-mcp 的 MCP 接入/);
    assert.match(message!, /不会卸载 MCP 服务或影响其他软件/);
  }
});

test('reads and unrelated actions do not ask to modify a configuration', () => {
  for (const action of ['mcp-view', 'mcp-install', 'mcp-local-save', 'mcp-local-remove'])
    assert.equal(mcpConfirmation(action, 'codex', external), undefined);
  assert.equal(mcpConfirmation('mcp-disable', 'codex'), undefined);
});

test('MCP UI hides management internals and keeps read-only entries actionless', () => {
  const source = readFileSync(new URL('../src/app/McpManager.tsx', import.meta.url), 'utf8');
  const integrations = readFileSync(
    new URL('../src/app/Integrations.tsx', import.meta.url),
    'utf8',
  );
  assert.doesNotMatch(source, /接管|工具箱管理|外部配置|mcp-adopt/);
  assert.doesNotMatch(integrations, /接管现有配置/);
  assert.match(source, /if \(entry\.readOnlyReason\)/);
  assert.match(source, /移除接入/);
  assert.match(source, /if \(confirmation && !window\.confirm\(confirmation\)\) return;/);
});
