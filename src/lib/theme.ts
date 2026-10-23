// Follows the system appearance and lets the user override it.
//
// The stylesheets answer `prefers-color-scheme`, so an unset value follows the
// system. An explicit choice is stored on the document element as
// `data-theme` and wins over the system in both directions.
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { WINDOW_BACKGROUND } from './appearance';

export type ThemePreference = 'system' | 'light' | 'dark';

const KEY = 'cyapi-toolbox.theme.v1';

export function loadTheme(): ThemePreference {
  try {
    const value = localStorage.getItem(KEY);
    if (value === 'light' || value === 'dark' || value === 'system') return value;
  } catch {
    // A blocked storage falls back to the system appearance.
  }
  return 'system';
}

export function applyTheme(preference: ThemePreference): void {
  const root = document.documentElement;
  if (preference === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', preference);
  try {
    localStorage.setItem(KEY, preference);
  } catch {
    // The choice still applies to the running window.
  }
}

/** The appearance a preference resolves to right now. */
export function resolveTheme(preference: ThemePreference): 'light' | 'dark' {
  if (preference !== 'system') return preference;
  return window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

/** The appearance the page currently uses. */
export function currentTheme(): 'light' | 'dark' {
  return resolveTheme(loadTheme());
}

/**
 * Applies the appearance to the native window, so the title bar, the traffic
 * lights and the frame around the page follow the same choice as the page
 * itself. The window keeps the system appearance while the preference follows
 * it, and the background is refreshed when the system appearance changes.
 *
 * Returns a function that stops listening for system appearance changes.
 */
export function syncNativeTheme(preference: ThemePreference): () => void {
  if (!isTauri()) return () => {};
  const current = getCurrentWindow();
  const apply = async () => {
    const effective = resolveTheme(preference);
    try {
      // A null theme lets the window follow the system again.
      await current.setTheme(preference === 'system' ? null : preference);
      await current.setBackgroundColor(WINDOW_BACKGROUND[effective]);
    } catch {
      // The window keeps the system appearance when it refuses the change.
    }
  };
  void apply();
  const media = window.matchMedia?.('(prefers-color-scheme: dark)');
  const onChange = () => {
    void apply();
  };
  media?.addEventListener('change', onChange);
  return () => {
    media?.removeEventListener('change', onChange);
  };
}
