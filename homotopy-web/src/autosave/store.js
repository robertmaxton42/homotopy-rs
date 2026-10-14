// Where the automatic backups of the project are kept: IndexedDB (a project is megabytes, which
// localStorage cannot hold), in the browser of the person. This file is included in
// `autosave.rs`, which says what a backup is and when one is made.
//
// IndexedDB may not be there, or may refuse: a private window, site data that is blocked or
// cleared, a full disk. Nothing here is called from Rust without `catch`, so that a failure is a
// message that is logged, and never a crash.
//
// Two stores, so that listing the backups does not read them: `meta` holds a small record for each
// (session, time, title, size, clean) and `data` the bytes of the `.hom` file, under the same
// key. The key is the *session* of the tab that made it: a tab keeps one backup of its own, the
// latest, and replaces it as it goes. A save is one transaction, so a backup is replaced whole or
// not at all, and a tab that is closed (or crashes) while saving leaves the previous one alone.
//
// Tabs: each tab holds a Web Lock named for its session for as long as it is open, which is how a
// tab can tell the backups of tabs that are still open (not to be offered, and not to be deleted)
// from those of tabs that are gone. Where Web Locks do not exist every other session counts as
// gone.

const DB_NAME = "homotopy-autosave";
const META = "meta";
const DATA = "data";
const LOCK_PREFIX = "homotopy-tab-";
const SETTING = "homotopy.autosave";

let opened = null;

function open() {
  if (opened) return opened;
  opened = new Promise((resolve, reject) => {
    let request;
    try {
      if (!window.indexedDB) throw new Error("IndexedDB is not available.");
      request = window.indexedDB.open(DB_NAME, 1);
    } catch (error) {
      reject(error);
      return;
    }
    request.onupgradeneeded = () => {
      const db = request.result;
      db.createObjectStore(META, { keyPath: "session" });
      db.createObjectStore(DATA);
    };
    request.onsuccess = () => {
      const db = request.result;
      // Another tab upgrading the database must not wait for this one.
      db.onversionchange = () => {
        db.close();
        opened = null;
      };
      resolve(db);
    };
    request.onerror = () => reject(request.error || new Error("The database could not be opened."));
    request.onblocked = () => reject(new Error("The database is blocked by another tab."));
  });
  // A failure is not remembered: the next try opens it again.
  opened.catch(() => {
    opened = null;
  });
  return opened;
}

function describe(error) {
  if (!error) return "unknown error";
  return error.name ? `${error.name}: ${error.message}` : String(error);
}

// Resolves when the transaction has been committed, and rejects if it was not.
function committed(tx) {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(new Error(describe(tx.error)));
    tx.onabort = () => reject(new Error(describe(tx.error || new Error("The transaction was aborted."))));
  });
}

function request(r) {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(new Error(describe(r.error)));
  });
}

// The sessions of the tabs that are open, or `null` if that cannot be known.
async function liveSessions() {
  try {
    if (!navigator.locks || !navigator.locks.query) return null;
    const { held } = await navigator.locks.query();
    return (held || [])
      .map((lock) => lock.name)
      .filter((name) => name && name.startsWith(LOCK_PREFIX))
      .map((name) => name.slice(LOCK_PREFIX.length));
  } catch (error) {
    return null;
  }
}

// Marks this tab as open for as long as it is.
export function holdLock(session) {
  if (!navigator.locks || !navigator.locks.request) return;
  navigator.locks
    .request(LOCK_PREFIX + session, () => new Promise(() => {}))
    .catch(() => {});
}

// Rejects if the database cannot be opened, which is all that can be known without writing.
export async function probe() {
  await open();
}

// Replaces the backup of the session (bytes: a Uint8Array), and then deletes the backups of tabs
// that are gone beyond the `keepGone` newest, which are the "previous" ones: work from before
// this tab was opened, kept until something newer takes their place.
export async function save(session, title, time, bytes, keepGone) {
  const db = await open();
  const live = new Set((await liveSessions()) || []);
  const tx = db.transaction([META, DATA], "readwrite");
  const done = committed(tx);
  const meta = tx.objectStore(META);
  const data = tx.objectStore(DATA);
  meta.put({ session, time, title, size: bytes.length, clean: false });
  data.put(bytes, session);
  meta.getAll().onsuccess = (event) => {
    const gone = event.target.result
      .filter((record) => record.session !== session && !live.has(record.session))
      .sort((a, b) => b.time - a.time);
    for (const record of gone.slice(keepGone)) {
      meta.delete(record.session);
      data.delete(record.session);
    }
  };
  await done;
}

// The backups that could be restored, as JSON: those that were not made by this session, whose
// tab is gone and that have work in them that has not been exported, newest first.
export async function list(session) {
  const db = await open();
  const tx = db.transaction([META], "readonly");
  const records = await request(tx.objectStore(META).getAll());
  const live = new Set((await liveSessions()) || []);
  const offered = records
    .filter((record) => record.session !== session && !live.has(record.session) && !record.clean)
    .sort((a, b) => b.time - a.time);
  return JSON.stringify(offered);
}

// The bytes of a backup, or `null`.
export async function read(session) {
  const db = await open();
  const tx = db.transaction([DATA], "readonly");
  const bytes = await request(tx.objectStore(DATA).get(session));
  return bytes || null;
}

// Deletes backups (sessions: JSON, an array).
export async function remove(sessions) {
  const db = await open();
  const tx = db.transaction([META, DATA], "readwrite");
  const done = committed(tx);
  for (const session of JSON.parse(sessions)) {
    tx.objectStore(META).delete(session);
    tx.objectStore(DATA).delete(session);
  }
  await done;
}

// Deletes the backups of tabs that are gone, and this tab's own if `includeOwn`; those of tabs
// that are still open are not for this tab to delete.
export async function clear(session, includeOwn) {
  const db = await open();
  const live = new Set((await liveSessions()) || []);
  live.delete(session);
  if (!includeOwn) live.add(session);
  const tx = db.transaction([META, DATA], "readwrite");
  const done = committed(tx);
  const meta = tx.objectStore(META);
  const data = tx.objectStore(DATA);
  meta.getAll().onsuccess = (event) => {
    for (const record of event.target.result) {
      if (live.has(record.session)) continue;
      meta.delete(record.session);
      data.delete(record.session);
    }
  };
  await done;
}

// Says that what the session's backup holds is safe elsewhere now (the person exported it), so
// that it is not offered when the page is next opened. The next backup is not clean.
export async function markClean(session) {
  const db = await open();
  const tx = db.transaction([META], "readwrite");
  const done = committed(tx);
  const meta = tx.objectStore(META);
  meta.get(session).onsuccess = (event) => {
    const record = event.target.result;
    if (record) meta.put({ ...record, clean: true });
  };
  await done;
}

// Whether the person turned autosave off, which is the one setting that is kept from one visit to
// the next (the others are not): a switch that turns itself on again every time is not one.
// Returns "off", "on" or `null` if they never said.
export function readSetting() {
  try {
    return window.localStorage.getItem(SETTING);
  } catch (error) {
    return null;
  }
}

export function writeSetting(on) {
  try {
    window.localStorage.setItem(SETTING, on ? "on" : "off");
  } catch (error) {
    // Private windows and the like: it is not remembered.
  }
}
