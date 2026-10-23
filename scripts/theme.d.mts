import type { Plugin } from 'vite';

/** The stylesheets that make up the running application. */
export declare const STYLE_SHEETS: string[];

/** Vite plugin that turns the literal colours into theme variables. */
export declare function themePlugin(): Plugin;

/** Rewrite `src/app/theme.css` from the current stylesheets. */
export declare function writeThemeFile(): number;
