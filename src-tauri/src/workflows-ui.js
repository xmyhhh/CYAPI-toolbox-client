// UI-only adapter for the top-level Service WebView. No native API or secrets.
// Canvas frames are deliberately excluded. The remote page keeps its own fetch,
// first-party HTTPS Cookie, CSRF, file picker, downloads and busy-state handling.
(() => {
  if (
    window.top !== window ||
    location.origin !== 'https://api.cyapi.cyou' ||
    location.pathname !== '/workflows/'
  )
    return;
  const sections = new Set(['library', 'drafts', 'queue', 'results']);
  let requestId, pending;
  const locked = () =>
    Boolean(
      document.querySelector(
        '#modal[open], #app[inert], #app[aria-busy="true"], [data-action][disabled]:not([data-action="canvas-save"])',
      ),
    );
  Object.defineProperty(window, '__cyapiWorkflowUi', {
    value: Object.freeze({
      sync(message) {
        if (!sections.has(message.section) || !['light', 'dark'].includes(message.theme))
          return { ready: false, section: null, locked: false };
        document.documentElement.dataset.embedded = 'true';
        document.documentElement.dataset.theme = message.theme;
        if (requestId !== message.navigationId) {
          requestId = message.navigationId;
          pending = message.section;
        }
        const login = document.querySelector('#login-form');
        let active = document.querySelector('.tabs .active')?.dataset.action;
        // Clicking the service's own tab preserves its save-before-navigation and
        // busy/upload/modal guards. Never reload a draft or replay a submission.
        if (pending && !login && !locked() && active) {
          if (pending !== active)
            document.querySelector(`.tabs [data-action="${pending}"]`)?.click();
          pending = undefined;
          active = document.querySelector('.tabs .active')?.dataset.action;
        }
        return {
          ready: Boolean(login || active),
          section: sections.has(active) ? active : null,
          locked: locked(),
        };
      },
    }),
  });
})();
