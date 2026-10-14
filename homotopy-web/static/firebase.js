// This script initialises Firebase, if the Firebase scripts (see index.html) could be loaded.
//
// Firebase is only used for cloud storage (the Account drawer, and opening projects by URL). The
// scripts come from gstatic.com, and are not there for people who block them (an extension such
// as NoScript or uBlock Origin) or are offline, so nothing here may throw, and the app has to
// work without it: it asks `window.homotopyCloud` whether Firebase is `ready` (and, if not,
// `error` says why) before it touches Firebase (`account_script.js`).
//
// `window.homotopyCloudRetry()` loads the scripts that are missing again and initialises
// Firebase, for when the person has allowed them since, so that they do not have to reload the
// page and lose their work. It resolves to the error (or `null` if Firebase is ready now).

(function () {
  // Your web app's Firebase configuration
  // For Firebase JS SDK v7.20.0 and later, measurementId is optional
  const firebaseConfig = {
    apiKey: "AIzaSyBCtkQM2P7eBAFtKWnlGfyTNyTHE8y5wXY",
    authDomain: "homotopy-io.firebaseapp.com",
    databaseURL: "https://homotopy-io.firebaseio.com",
    projectId: "homotopy-io",
    storageBucket: "homotopy-io.appspot.com",
    messagingSenderId: "872831343483",
    appId: "1:872831343483:web:4de585eb01b14a27ad3bde",
    measurementId: "G-CYGQVSSM6Q"
  };

  // What each of the scripts in index.html (`data-firebase`) provides, to see which are missing.
  // Analytics and performance monitoring are not needed for anything the app does.
  const provides = {
    app: () => typeof firebase !== "undefined",
    analytics: () => typeof firebase !== "undefined" && typeof firebase.analytics === "function",
    auth: () => typeof firebase !== "undefined" && typeof firebase.auth === "function",
    firestore: () => typeof firebase !== "undefined" && typeof firebase.firestore === "function",
    functions: () => typeof firebase !== "undefined" && typeof firebase.functions === "function",
    performance: () => typeof firebase !== "undefined" && typeof firebase.performance === "function",
    storage: () => typeof firebase !== "undefined" && typeof firebase.storage === "function",
    ui: () => typeof firebaseui !== "undefined",
  };
  const required = ["app", "auth", "firestore", "functions", "storage", "ui"];
  const LOAD_TIMEOUT_MS = 15000;

  const cloud = window.homotopyCloud = {
    ready: false,
    error: "Firebase has not been started.",
    // Set when ready: the Firebase namespace, the sign-in UI and the functions service.
    firebase: null,
    ui: null,
    functions: null,
  };

  let emulatorsUsed = false;

  function initialise() {
    cloud.ready = false;
    cloud.firebase = cloud.ui = cloud.functions = null;

    const missing = required.filter((name) => !provides[name]());
    if (missing.length > 0) {
      cloud.error = "The Firebase scripts from gstatic.com were not loaded (missing: " +
        missing.join(", ") + ").";
      return;
    }

    try {
      console.log("Loading firebase...");
      const app = firebase.apps.length > 0 ? firebase.app() : firebase.initializeApp(firebaseConfig);

      // Not needed for anything the app does, so not worth giving up for.
      for (const name of ["analytics", "performance"]) {
        try {
          if (provides[name]()) app[name]();
        } catch (e) {
          console.warn("Firebase " + name + " is unavailable:", e);
        }
      }

      const auth = app.auth();
      const firestore = app.firestore();
      const storage = app.storage();
      const functions = app.functions("us-east1");

      if (location.hostname === "localhost" && !emulatorsUsed) {
        console.log("localhost detected, using firebase emulators");
        auth.useEmulator("http://127.0.0.1:9099");
        firestore.useEmulator("127.0.0.1", 8080);
        storage.useEmulator("127.0.0.1", 9199);
        functions.useEmulator("127.0.0.1", 5001);
        emulatorsUsed = true;
      }

      cloud.ui = firebaseui.auth.AuthUI.getInstance() || new firebaseui.auth.AuthUI(auth);
      cloud.functions = functions;
      cloud.firebase = app.firebase || firebase;
      cloud.ready = true;
      cloud.error = null;
    } catch (e) {
      console.warn("Firebase could not be started:", e);
      cloud.ready = false;
      cloud.error = "Firebase could not be started: " + (e && e.message ? e.message : String(e));
    }
  }

  // Loads again what is in index.html and is missing, in order, and waits for it to be done.
  function load(element) {
    return new Promise((resolve) => {
      const fresh = document.createElement(element.tagName.toLowerCase());
      for (const attribute of element.attributes) fresh.setAttribute(attribute.name, attribute.value);
      const done = () => { clearTimeout(timer); resolve(); };
      const timer = setTimeout(done, LOAD_TIMEOUT_MS);
      fresh.addEventListener("load", done);
      fresh.addEventListener("error", done);
      if (fresh.tagName === "SCRIPT") fresh.async = false; // keep the order
      element.replaceWith(fresh);
    });
  }

  let retrying = null;

  window.homotopyCloudRetry = function () {
    if (cloud.ready) return Promise.resolve(null);
    if (retrying) return retrying;
    retrying = (async () => {
      try {
        const elements = Array.from(document.querySelectorAll("[data-firebase]"));
        const missing = elements.filter((element) => {
          const name = element.getAttribute("data-firebase");
          return name === "css" ? !element.sheet : !provides[name]();
        });
        // Everything depends on the app, which has to come first; the rest may follow together.
        const app = missing.filter((element) => element.getAttribute("data-firebase") === "app");
        await Promise.all(app.map(load));
        if (provides.app()) {
          await Promise.all(missing.filter((element) => !app.includes(element)).map(load));
        }
      } catch (e) {
        console.warn("Loading the Firebase scripts again failed:", e);
      }
      initialise();
      return cloud.error;
    })().finally(() => { retrying = null; });
    return retrying;
  };

  initialise();
})();
