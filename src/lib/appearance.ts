/**
 * The window background in both appearances. It matches the page surface, so
 * the area around the page never shows a light frame in the dark appearance.
 *
 * The dark value is the surface variant of the light one, so the palette in
 * `scripts/theme.mjs` stays the single source of truth.
 */
export const WINDOW_BACKGROUND = {
  light: '#f7f8f6',
  dark: '#16181a',
} as const;
