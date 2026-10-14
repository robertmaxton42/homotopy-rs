// The web workers of the app (see homotopy-web/src/worker.rs): the same module as the page's,
// whose start function sees that there is no window and waits for work instead of starting the
// app. There are two of them, told apart by the query of the script's URL:
//
// - `worker.js` does the jobs that would block the page (a homotopy, a theorem, ...);
// - `worker.js?layout` lays diagrams out (homotopy-web/src/layout.rs), and so also loads HiGHS,
//   which `homotopy-core` calls as the global functions `self.Highs_*` that `highs.js` installs.
//
// Each says `ready` once it can be given work, and `failed` if it cannot start.
import init, { worker_message } from './homotopy_web.js';

const layout = self.location.search === '?layout';

const ready = (async () => {
  await init();
  if (layout) {
    const highs = await import('./highs.js');
    // The page's own copy finds highs.wasm next to highs.js; so must this one.
    await highs.default({ locateFile: (file) => new URL(file, import.meta.url).href });
  }
  self.postMessage({ kind: 'ready' });
})();
// If the module cannot be loaded the page falls back to doing the work itself.
ready.catch((error) => self.postMessage({ kind: 'failed', message: String(error) }));

self.onmessage = async (event) => {
  try {
    await ready;
  } catch (error) {
    return;
  }
  worker_message(event.data);
};
