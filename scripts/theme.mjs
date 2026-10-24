// Builds the dark palette for the desktop client.
//
// The stylesheets use literal colours. At build time every colour becomes a CSS
// custom property, and a dark variant is derived from the role the colour
// plays: a background becomes a dark surface, body text becomes light, and
// panels that are already dark keep their value.
//
// Run `npm run theme` after changing a colour to refresh `src/app/theme.css`.
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderWorkspaceTheme } from './workspace-palettes.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const appDirectory = path.join(here, '..', 'src', 'app');
const themeFile = path.join(appDirectory, 'theme.css');

/** The stylesheets that make up the running application. */
export const STYLE_SHEETS = [
  'native.css',
  'usage-statistics.css',
  'diagnostics.css',
  'codex-analysis.css',
  'agent-log-analysis.css',
  'cyapi-balance.css',
];

/** Colours that keep their light value in the dark theme. */
const KEEP_LIGHT = new Set(['d1e8a9', '254d37']);

/**
 * Hand picked dark values for colours the automatic rule cannot judge. The
 * chart series keep their identity and only move far enough to stay readable
 * on a dark background.
 */
const DARK_OVERRIDES = new Map([
  ['2c6b50', '5fae86'],
  ['86a86c', '9dc07f'],
  ['bdce91', 'c9dc9d'],
  ['cba970', 'd9b982'],
  ['8b9fa2', 'a3b5b8'],
  ['d9e2d0', 'cfd8c6'],
]);

/**
 * The dark surfaces of the layout, from the page to the hairline. A macOS
 * window keeps the page darkest and lifts every surface above it, so the
 * light palette is turned upside down here.
 */
const DARK_PAGE = '16181a';
const DARK_CARD = '1e2226';
const DARK_RAISED = '262c30';
const DARK_BORDER = '333a40';

/**
 * The dark inks, from the body text to the faintest hint. The light palette
 * uses tints of the same green for its text, so the dark palette reads them
 * as steps of one neutral grey.
 */
const DARK_INK = 'e8eaec';
const DARK_INK_MUTED = 'b0b7bd';
const DARK_INK_FAINT = '929aa0';

/**
 * The surfaces that carry the layout. The automatic rule reads the lightness
 * of a colour, but the page and a card are both near white in the light
 * palette, and only the design knows which one is which.
 */
const DARK_SURFACES = new Map([
  ['ffffff', DARK_CARD], // cards, panels and the page heading
  ['f7f8f6', DARK_PAGE], // the page itself
  ['f7f9f4', '1a1d21'], // code blocks sit below the surface
]);

/** The inks that the design relies on. The accent keeps the green of the
 * brand, but it has to carry the weight of a link on a dark page.
 */
const DARK_FOREGROUNDS = new Map([
  ['29644d', '5fc79b'], // the accent of the brand
]);

/**
 * How far the channels of an ink may drift before it stops reading as a
 * neutral. The light palette writes its text in tints of one green.
 */
const INK_CHROMA = 28;

/** The colours a stylesheet may write as a name instead of a hex value. */
const NAMED_COLOURS = new Map([
  ['white', 'ffffff'],
  ['black', '000000'],
]);

/** Properties whose colour follows the surface direction: light to dark. */
const SURFACE_PROPERTIES = new Set([
  'background',
  'background-color',
  'border',
  'border-color',
  'border-top',
  'border-right',
  'border-bottom',
  'border-left',
  'border-top-color',
  'border-right-color',
  'border-bottom-color',
  'border-left-color',
  'outline-color',
]);

/** Properties whose colour follows the text direction: dark to light. */
const FOREGROUND_PROPERTIES = new Set([
  'color',
  'fill',
  'stroke',
  'text-decoration-color',
  'accent-color',
]);

function hexToRgb(hex) {
  const value = hex.length === 3 ? hex.replace(/./g, (c) => c + c) : hex;
  return [
    parseInt(value.slice(0, 2), 16),
    parseInt(value.slice(2, 4), 16),
    parseInt(value.slice(4, 6), 16),
  ];
}

function rgbToHex([r, g, b]) {
  return [r, g, b]
    .map((channel) =>
      Math.max(0, Math.min(255, Math.round(channel)))
        .toString(16)
        .padStart(2, '0'),
    )
    .join('');
}

function rgbToHsl([r, g, b]) {
  const red = r / 255;
  const green = g / 255;
  const blue = b / 255;
  const max = Math.max(red, green, blue);
  const min = Math.min(red, green, blue);
  const lightness = (max + min) / 2;
  if (max === min) return [0, 0, lightness];
  const delta = max - min;
  const saturation = lightness > 0.5 ? delta / (2 - max - min) : delta / (max + min);
  let hue;
  if (max === red) hue = ((green - blue) / delta + (green < blue ? 6 : 0)) / 6;
  else if (max === green) hue = ((blue - red) / delta + 2) / 6;
  else hue = ((red - green) / delta + 4) / 6;
  return [hue, saturation, lightness];
}

function hslToRgb([hue, saturation, lightness]) {
  if (saturation === 0) {
    const value = lightness * 255;
    return [value, value, value];
  }
  const q =
    lightness < 0.5
      ? lightness * (1 + saturation)
      : lightness + saturation - lightness * saturation;
  const p = 2 * lightness - q;
  const channel = (t) => {
    let value = t;
    if (value < 0) value += 1;
    if (value > 1) value -= 1;
    if (value < 1 / 6) return p + (q - p) * 6 * value;
    if (value < 1 / 2) return q;
    if (value < 2 / 3) return p + (q - p) * (2 / 3 - value) * 6;
    return p;
  };
  return [channel(hue + 1 / 3) * 255, channel(hue) * 255, channel(hue - 1 / 3) * 255];
}

/**
 * How far the channels of a colour drift apart. A true grey keeps them
 * together, while a colour that means something spreads them.
 */
function chroma(hex) {
  const [red, green, blue] = hexToRgb(hex);
  return Math.max(red, green, blue) - Math.min(red, green, blue);
}

/**
 * The pale surfaces of the light palette are tints of the same green. They
 * read as neutrals, so the dark palette treats them as neutrals as well and
 * keeps its colour for the surfaces that mean something.
 */
function isNeutralSurface(hue, spread) {
  const degrees = hue * 360;
  return degrees >= 60 && degrees <= 180 && spread <= 30;
}

/**
 * A surface that carries no colour of its own follows the dark layout: the
 * page is the darkest, a card sits above it and a hairline sits above that.
 */
function neutralSurface(lightness) {
  if (lightness >= 0.975) return DARK_CARD;
  if (lightness >= 0.925) return DARK_RAISED;
  return DARK_BORDER;
}

/** An ink that carries no colour of its own follows the dark text steps. */
function neutralInk(lightness) {
  if (lightness >= 0.5) return DARK_INK_FAINT;
  if (lightness >= 0.28) return DARK_INK_MUTED;
  return DARK_INK;
}

/** A background becomes a dark surface; a dark panel keeps its value. */
function surfaceVariant(hex) {
  if (KEEP_LIGHT.has(hex)) return hex;
  if (DARK_SURFACES.has(hex)) return DARK_SURFACES.get(hex);
  if (DARK_OVERRIDES.has(hex)) return DARK_OVERRIDES.get(hex);
  const [hue, saturation, lightness] = rgbToHsl(hexToRgb(hex));
  if (lightness < 0.55) return hex;
  if (isNeutralSurface(hue, chroma(hex))) return neutralSurface(lightness);
  let targetLightness;
  let targetSaturation;
  if (lightness >= 0.93) {
    targetLightness = 0.115 + (1 - lightness) * 0.3;
    targetSaturation = Math.min(0.3, saturation * 1.5 + 0.02);
  } else if (lightness >= 0.82) {
    targetLightness = 0.17 + (0.92 - lightness) * 0.55;
    targetSaturation = Math.min(0.34, saturation * 1.35 + 0.02);
  } else if (lightness >= 0.7) {
    targetLightness = 0.22 + (0.82 - lightness) * 0.55;
    targetSaturation = Math.min(0.4, saturation * 1.25 + 0.02);
  } else {
    targetLightness = 0.27 + (0.7 - lightness) * 0.55;
    targetSaturation = Math.min(0.45, saturation * 1.15 + 0.02);
  }
  return rgbToHex(hslToRgb([hue, targetSaturation, targetLightness]));
}

/** Body text becomes light; a light ink keeps its value. */
function foregroundVariant(hex) {
  if (KEEP_LIGHT.has(hex)) return hex;
  if (DARK_FOREGROUNDS.has(hex)) return DARK_FOREGROUNDS.get(hex);
  if (DARK_OVERRIDES.has(hex)) return DARK_OVERRIDES.get(hex);
  const [hue, saturation, lightness] = rgbToHsl(hexToRgb(hex));
  if (lightness >= 0.62) return hex;
  if (chroma(hex) <= INK_CHROMA) return neutralInk(lightness);
  const targetLightness =
    lightness >= 0.45 ? 0.66 + (0.62 - lightness) * 0.2 : 0.76 + (0.45 - lightness) * 0.2;
  const targetSaturation = saturation * (lightness >= 0.45 ? 0.92 : 0.85);
  return rgbToHex(hslToRgb([hue, targetSaturation, targetLightness]));
}

function stripComments(css) {
  return css.replace(/\/\*[\s\S]*?\*\//g, '');
}

/** Split a stylesheet into its top level rules. */
function readRules(css) {
  const rules = [];
  let index = 0;
  while (index < css.length) {
    const open = css.indexOf('{', index);
    if (open < 0) break;
    const selector = css.slice(index, open).trim();
    let depth = 1;
    let cursor = open + 1;
    while (cursor < css.length && depth > 0) {
      if (css[cursor] === '{') depth += 1;
      else if (css[cursor] === '}') depth -= 1;
      cursor += 1;
    }
    rules.push({ selector, body: css.slice(open + 1, cursor - 1) });
    index = cursor;
  }
  return rules;
}

function readStylesheets() {
  return STYLE_SHEETS.map((name) => ({
    name,
    css: stripComments(readFileSync(path.join(appDirectory, name), 'utf8')),
  }));
}

/** The role a declaration gives to the colour it contains. */
function roleOf(property) {
  if (SURFACE_PROPERTIES.has(property)) return 'surface';
  if (FOREGROUND_PROPERTIES.has(property)) return 'foreground';
  return null;
}

/**
 * Normalise a colour to its six digit form. Colours with an alpha channel
 * keep their literal value, because they only appear in shadows and overlays
 * that keep their own colour in both appearances.
 */
function normaliseHex(raw) {
  if (raw.length === 3) return raw.replace(/./g, (c) => c + c).toLowerCase();
  if (raw.length === 6) return raw.toLowerCase();
  return null;
}

/** Every plain colour in a value, as six digit hex values. */
function coloursIn(value) {
  const found = [];
  for (const match of value.matchAll(/#([0-9a-fA-F]{3,8})\b/g)) {
    const hex = normaliseHex(match[1]);
    if (hex) found.push(hex);
  }
  for (const match of value.matchAll(/\b(white|black)\b/gi)) {
    const hex = NAMED_COLOURS.get(match[1].toLowerCase());
    if (hex) found.push(hex);
  }
  return found;
}

/**
 * Collect every colour with the role it plays. A colour used as both a
 * background and a text needs one dark variant per role.
 *
 * A colour that is declared as a custom property inherits the role of the
 * declarations that read that property, so `--green` follows the same rules as
 * a literal `color` value would.
 */
export function collectRoles(sheets = readStylesheets()) {
  const roles = new Map();
  const entry = (hex) => {
    const key = hex.toLowerCase();
    if (!roles.has(key)) {
      roles.set(key, {
        surface: false,
        foreground: false,
        directSurface: false,
        directForeground: false,
      });
    }
    return roles.get(key);
  };
  const customRoles = new Map();
  const customValues = new Map();
  for (const sheet of sheets) {
    for (const rule of readRules(sheet.css)) {
      for (const match of rule.body.matchAll(/([-a-zA-Z0-9_-]+)\s*:\s*([^;{}]+)/g)) {
        const property = match[1].toLowerCase();
        const role = roleOf(property);
        const custom = property.startsWith('--');
        for (const colour of coloursIn(match[2])) {
          if (custom) {
            customValues.set(property, colour);
            continue;
          }
          const record = entry(colour);
          if (role === 'surface') {
            record.surface = true;
            record.directSurface = true;
          }
          if (role === 'foreground') {
            record.foreground = true;
            record.directForeground = true;
          }
        }
        if (!custom) {
          for (const reference of match[2].matchAll(/var\(\s*(--[a-zA-Z0-9-]+)/g)) {
            if (!customRoles.has(reference[1])) {
              customRoles.set(reference[1], { surface: false, foreground: false });
            }
            if (role) customRoles.get(reference[1])[role] = true;
          }
        }
      }
    }
  }
  for (const [name, hex] of customValues) {
    const role = customRoles.get(name);
    const record = entry(hex);
    if (!role) {
      // The property is read from the interface code, so the colour follows
      // the text direction and stays readable on a dark background.
      record.foreground = true;
      continue;
    }
    if (role.surface) record.surface = true;
    if (role.foreground) record.foreground = true;
  }
  return roles;
}

/**
 * A foreground that sits on a background which keeps its light value has to
 * keep its own value as well, otherwise the pair loses its contrast.
 */
function collectChipForeground(sheets) {
  const keep = new Set();
  for (const sheet of sheets) {
    for (const rule of readRules(sheet.css)) {
      const backgrounds = [];
      const foregrounds = [];
      for (const match of rule.body.matchAll(/([-a-zA-Z0-9_-]+)\s*:\s*([^;{}]+)/g)) {
        const role = roleOf(match[1].toLowerCase());
        for (const colour of coloursIn(match[2])) {
          if (role === 'surface') backgrounds.push(colour);
          else if (role === 'foreground') foregrounds.push(colour);
        }
      }
      if (!backgrounds.some((colour) => KEEP_LIGHT.has(colour))) continue;
      for (const colour of foregrounds) keep.add(colour);
    }
  }
  return keep;
}

/** The light and dark value of every colour, keyed by the hex value. */
export function collectPalette(sheets = readStylesheets()) {
  const roles = collectRoles(sheets);
  const chipForeground = collectChipForeground(sheets);
  const palette = new Map();
  for (const [hex, role] of roles) {
    const surface = role.surface ? surfaceVariant(hex) : hex;
    const foreground = role.foreground ? foregroundVariant(hex) : hex;
    const keptForeground = chipForeground.has(hex) ? hex : foreground;
    // Only a colour that is written directly in both roles can be split. A
    // colour that reaches the page through a custom property keeps one value.
    const mixed = role.directSurface && role.directForeground;
    // A colour that only fills a surface keeps the surface value, a colour
    // that only writes text keeps the ink, and a colour that does both
    // follows the ink, because the reader meets that one the most.
    const primary = keptForeground !== hex ? keptForeground : surface;
    const dark = mixed ? primary : role.foreground ? keptForeground : surface;
    palette.set(hex, {
      light: `#${hex}`,
      surface: `#${surface}`,
      foreground: `#${keptForeground}`,
      dark: `#${dark}`,
      mixed,
    });
  }
  return palette;
}

/** The custom property name that carries a colour in the given role. */
export function variableName(hex, role, mixed) {
  const base = `--t-${hex.toLowerCase()}`;
  if (!mixed) return base;
  return role === 'foreground' ? `${base}-f` : `${base}-s`;
}

/** Render the palette as the stylesheet that defines every custom property. */
export function renderTheme(palette) {
  const light = [];
  const dark = [];
  for (const [hex, entry] of [...palette.entries()].sort()) {
    light.push(`  --t-${hex}: ${entry.light};`);
    if (entry.dark !== entry.light) dark.push(`  --t-${hex}: ${entry.dark};`);
    if (entry.mixed) {
      light.push(`  --t-${hex}-s: ${entry.light};`);
      light.push(`  --t-${hex}-f: ${entry.light};`);
      dark.push(`  --t-${hex}-s: ${entry.surface};`);
      dark.push(`  --t-${hex}-f: ${entry.foreground};`);
    }
  }
  return (
    [
      '/* Generated by scripts/theme.mjs. Run `npm run theme` after changing a colour. */',
      ':root {',
      '  color-scheme: light dark;',
      ...light,
      '}',
      '',
      '@media (prefers-color-scheme: dark) {',
      '  :root:not([data-theme="light"]) {',
      ...dark,
      '  }',
      '}',
      '',
      ':root[data-theme="dark"] {',
      ...dark,
      '}',
      '',
    ].join('\n') + renderWorkspaceTheme()
  );
}

/**
 * Replaces every colour in a stylesheet with the custom property that carries
 * its dark variant.
 */
export function transformCss(css, palette) {
  return css.replace(
    /([-a-zA-Z0-9_-]+)(\s*:\s*)([^;{}]+)/g,
    (match, property, separator, value) => {
      const role = roleOf(property.toLowerCase());
      const variable = (key) => {
        const entry = palette.get(key);
        if (!entry) return null;
        const name = variableName(key, role, entry.mixed);
        return `var(${name}, ${entry.light})`;
      };
      const replaced = value
        .replace(/#([0-9a-fA-F]{3,8})\b/g, (hit, raw) => {
          const key = normaliseHex(raw);
          if (!key) return hit;
          return variable(key) ?? hit;
        })
        .replace(/\b(white|black)\b/gi, (hit, raw) => {
          const key = NAMED_COLOURS.get(raw.toLowerCase());
          if (!key) return hit;
          return variable(key) ?? hit;
        });
      return `${property}${separator}${replaced}`;
    },
  );
}

/** Vite plugin that turns the literal colours into theme variables. */
export function themePlugin() {
  let palette = collectPalette();
  const refresh = () => {
    palette = collectPalette();
  };
  return {
    name: 'cyapi-theme',
    configureServer(server) {
      const update = (file) => {
        if (!file.endsWith('.css') || file.endsWith('theme.css')) return;
        refresh();
        server.ws.send({ type: 'full-reload' });
      };
      server.watcher.on('change', update);
      server.watcher.on('add', update);
    },
    transform(code, id) {
      if (!id.endsWith('.css')) return null;
      if (id.endsWith('theme.css')) return null;
      return { code: transformCss(code, palette), map: null };
    },
  };
}

/** Rewrite `src/app/theme.css` from the current stylesheets. */
export function writeThemeFile() {
  const palette = collectPalette();
  writeFileSync(themeFile, renderTheme(palette));
  return palette.size;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))
) {
  const count = writeThemeFile();
  console.log(`Wrote ${path.relative(process.cwd(), themeFile)} with ${count} colours.`);
}
