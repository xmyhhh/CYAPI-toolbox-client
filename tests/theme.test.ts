import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { collectPalette, renderTheme, transformCss } from '../scripts/theme.mjs';
import { WINDOW_BACKGROUND } from '../src/lib/appearance.ts';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const appDirectory = path.join(root, 'src', 'app');

function read(name: string): string {
  return readFileSync(path.join(appDirectory, name), 'utf8');
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
  assert.equal(palette.get('1c3c32')?.light, '#1c3c32');
  assert.equal(palette.get('d1e8a9')?.light, '#d1e8a9');
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
  // The sidebar and the brand mark keep their own values.
  assert.equal(palette.get('1c3c32')?.surface, '#1c3c32');
  assert.equal(palette.get('d1e8a9')?.surface, '#d1e8a9');
  assert.equal(palette.get('254d37')?.foreground, '#254d37');
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

test('the native window background follows the page surface', () => {
  const palette = collectPalette();
  const surface = palette.get('f7f8f6');
  assert.ok(surface);
  assert.equal(WINDOW_BACKGROUND.light, surface.light);
  assert.equal(WINDOW_BACKGROUND.dark, surface.surface);
});
