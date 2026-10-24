import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { collectPalette, renderTheme, transformCss } from '../scripts/theme.mjs';
import { WINDOW_BACKGROUND } from '../src/lib/appearance.ts';
import {
  WORKSPACE_PALETTES,
  renderWorkspaceTheme,
  workspaceVariables,
} from '../scripts/workspace-palettes.mjs';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const appDirectory = path.join(root, 'src', 'app');

function read(name: string): string {
  // Git may check the file out with CRLF depending on local configuration;
  // the generator always emits LF, so compare on normalised text.
  return readFileSync(path.join(appDirectory, name), 'utf8').replaceAll('\r\n', '\n');
}

test('every colour in the running stylesheets has a dark variant', () => {
  const palette = collectPalette();
  assert.ok(palette.size > 100, `expected a full palette, found ${palette.size}`);
  for (const [hex, entry] of palette) {
    assert.match(hex, /^[0-9a-f]{6}$/);
    assert.equal(entry.light, `#${hex}`);
    assert.match(entry.surface, /^#[0-9a-f]{6}$/);
    assert.match(entry.foreground, /^#[0-9a-f]{6}$/);
  }
});

test('the light appearance keeps the designed colours', () => {
  const palette = collectPalette();
  assert.equal(palette.get('f7f8f6')?.light, '#f7f8f6');
  assert.equal(workspaceVariables('codex', 'light').sidebar, '#1c3c32');
  assert.equal(workspaceVariables('codex', 'light').mark, '#d1e8a9');
});

test('backgrounds become dark and text becomes light', () => {
  const palette = collectPalette();
  // The application background turns into a dark surface.
  const background = palette.get('f7f8f6');
  assert.ok(background);
  assert.notEqual(background.surface, background.light);
  // Body text turns into a light ink.
  const text = palette.get('293c32');
  assert.ok(text);
  assert.notEqual(text.foreground, text.light);
  // Codex retains the original sidebar and brand identity in both appearances.
  const workspace = workspaceVariables('codex', 'dark');
  assert.equal(workspace.sidebar, '#1c3c32');
  assert.equal(workspace.mark, '#d1e8a9');
  assert.equal(workspace['mark-ink'], '#254d37');
});

test('the stylesheet keeps the same colours after the transform', () => {
  const palette = collectPalette();
  const source = read('native.css');
  const transformed = transformCss(source, palette);
  // The light values survive as the fallback of every variable.
  for (const match of source.matchAll(/#([0-9a-fA-F]{3,6})\b/g)) {
    const raw = match[1].length === 3 ? match[1].replace(/./g, (c) => c + c) : match[1];
    assert.ok(
      transformed.includes(`#${raw.toLowerCase()}`) || transformed.includes(raw),
      `${raw} lost its fallback value`,
    );
  }
  // No literal colour is left behind in a declaration.
  assert.ok(!/:[^;{}]*#[0-9a-fA-F]{6}\b/.test(transformed.replace(/var\([^)]*\)/g, '')));
});

test('the checked in palette matches the current stylesheets', () => {
  const palette = collectPalette();
  assert.equal(read('theme.css'), renderTheme(palette));
});

test('the palette follows the system and honours an explicit choice', () => {
  const css = renderTheme(collectPalette());
  assert.match(css, /color-scheme: light dark;/);
  assert.match(css, /@media \(prefers-color-scheme: dark\)/);
  assert.match(css, /:root:not\(\[data-theme="light"\]\)/);
  assert.match(css, /:root\[data-theme="dark"\]/);
});

function contrast(a: string, b: string): number {
  const luminance = (hex: string) => {
    const channels = hex
      .slice(1)
      .match(/../g)!
      .map((channel) => {
        const value = parseInt(channel, 16) / 255;
        return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
      });
    return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
  };
  const values = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (values[0] + 0.05) / (values[1] + 0.05);
}

test('each workspace has complete, readable light and dark accents', () => {
  const source = read('native.css') + read('diagnostics.css');
  const references = [...source.matchAll(/var\(--workspace-([a-z-]+)\)/g)].map((match) => match[1]);
  for (const workspace of Object.keys(WORKSPACE_PALETTES)) {
    for (const appearance of ['light', 'dark']) {
      const vars = workspaceVariables(workspace, appearance);
      for (const name of references) assert.ok(vars[name], `${workspace}/${appearance}: ${name}`);
      const background = appearance === 'light' ? '#ffffff' : '#1e2226';
      const pairs = [
        [vars.accent, background],
        ['#ffffff', vars.solid],
        ['#ffffff', vars.hover],
        [vars['tint-ink'], vars.tint],
        [vars.accent, vars.tint],
        [vars['sidebar-ink'], vars.sidebar],
        [vars['sidebar-faint'], vars.sidebar],
        [vars['sidebar-strong'], vars['sidebar-active']],
        [vars['sidebar-muted'], vars['sidebar-control']],
        [vars['mark-ink'], vars.mark],
      ];
      for (const [ink, surface] of pairs) {
        assert.ok(
          contrast(ink, surface) >= 4.5,
          `${workspace}/${appearance}: ${ink} on ${surface}`,
        );
      }
    }
  }
});

test('workspace accents follow system appearance without overriding explicit light', () => {
  const css = renderWorkspaceTheme();
  for (const workspace of Object.keys(WORKSPACE_PALETTES)) {
    assert.ok(css.includes(`:root[data-workspace="${workspace}"] {`));
    assert.ok(css.includes(`:root[data-workspace="${workspace}"]:not([data-theme="light"])`));
    assert.ok(css.includes(`:root[data-workspace="${workspace}"][data-theme="dark"]`));
  }
  // The identity layer must not recolour status colours, charts or page surfaces.
  assert.doesNotMatch(css, /--t-|--surface-muted|--line:/);
});

test('the native window background follows the page surface', () => {
  const palette = collectPalette();
  const surface = palette.get('f7f8f6');
  assert.ok(surface);
  assert.equal(WINDOW_BACKGROUND.light, surface.light);
  assert.equal(WINDOW_BACKGROUND.dark, surface.surface);
});
