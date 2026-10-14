// The web worker that does the jobs that would otherwise block the page (see
// homotopy-web/src/worker.rs). It is the same module as the page's: its start function sees that
// there is no window and waits for jobs instead of starting the app.
import init, { worker_message } from './homotopy_web.js';

const ready = init();
// If the module cannot be loaded the page falls back to doing the jobs itself.
ready.catch((error) => self.postMessage({ kind: 'failed', message: String(error) }));

self.onmessage = async (event) => {
  try {
    await ready;
  } catch (error) {
    return;
  }
  worker_message(event.data);
};
