import { version } from '../../package.json';

/** Product metadata shared by the sidebar footer, the about panel and the help window. */
export const APP_VERSION = version;
export const APP_NAME = 'CYAPI Toolbox';
export const APP_VENDOR = 'CYAPI';
/** The official website. It is always opened in the system browser, never inside the app. */
export const OFFICIAL_SITE = 'https://www.cyapi.cyou';
/** Download page published by the toolbox site. */
export const DOWNLOAD_PAGE = 'https://tool.cyapi.cyou/#download';
export const COPYRIGHT_YEAR = new Date().getFullYear();
/** Short form used in the narrow sidebar footer. */
export const COPYRIGHT_SHORT = `© ${COPYRIGHT_YEAR} ${APP_NAME}`;
/** Full statement used in the about panel and the help window. */
export const COPYRIGHT_TEXT = `© ${COPYRIGHT_YEAR} ${APP_NAME} · ${APP_VENDOR} 保留所有权利`;
