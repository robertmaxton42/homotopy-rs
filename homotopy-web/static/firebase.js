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
//
// `window.homotopyCloudProbe()` asks Firebase whether the project accepts requests from this page
// (its API key is restricted to the sites the project's owner allows, and the same goes for sign-in
// with a popup), which is the one failure that is neither a missing script nor something to retry.
// It does so once, when the Account drawer is opened, and not at load.
//
// Using another Firebase project, and the emulators:
//  - The configuration below is homotopy.io's own project, which only accepts requests from
//    homotopy.io and a few other sites (and localhost). To use your own project, put its web
//    configuration (the object that the Firebase console gives you) in the browser as JSON:
//    `localStorage.setItem("homotopy-firebase-config", JSON.stringify({apiKey: ..., ...}))`, and
//    reload. `localStorage.removeItem("homotopy-firebase-config")` goes back.
//  - The local Firebase emulators (auth 9099, firestore 8080, storage 9199, functions 5001, on
//    127.0.0.1) are only used if asked for: by opening the page with `?emulators` in its address,
//    or with `localStorage.setItem("homotopy-firebase-emulators", "1")`.

(function () {
  // Your web app's Firebase configuration
  // For Firebase JS SDK v7.20.0 and later, measurementId is optional
  const defaultConfig = {
    apiKey: "AIzaSyBCtkQM2P7eBAFtKWnlGfyTNyTHE8y5wXY",
    authDomain: "homotopy-io.firebaseapp.com",
    databaseURL: "https://homotopy-io.firebaseio.com",
    projectId: "homotopy-io",
    storageBucket: "homotopy-io.appspot.com",
    messagingSenderId: "872831343483",
    appId: "1:872831343483:web:4de585eb01b14a27ad3bde",
    measurementId: "G-CYGQVSSM6Q"
  };

  // The configuration of another project, if the person has put one in the browser (see above).
  function configuration() {
    try {
      const custom = JSON.parse(localStorage.getItem("homotopy-firebase-config"));
      if (custom && typeof custom === "object" && custom.apiKey && custom.projectId) {
        return custom;
      }
    } catch (e) {
      console.warn("Ignoring homotopy-firebase-config:", e);
    }
    return defaultConfig;
  }

  function emulatorsWanted() {
    try {
      if (new URLSearchParams(location.search).has("emulators")) return true;
      return localStorage.getItem("homotopy-firebase-emulators") === "1";
    } catch (e) {
      return false;
    }
  }

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
    // Set (to the same message as `error`) when Firebase is there but the project does not accept
    // this page (see `probe`): trying again will not help.
    rejected: false,
    // Set when ready: the Firebase namespace, the sign-in UI and the functions service.
    firebase: null,
    ui: null,
    functions: null,
  };

  let emulatorsUsed = false;
  let probing = null;

  function initialise() {
    cloud.ready = false;
    cloud.rejected = false;
    probing = null;
    cloud.firebase = cloud.ui = cloud.functions = null;

    const missing = required.filter((name) => !provides[name]());
    if (missing.length > 0) {
      cloud.error = "The Firebase scripts from gstatic.com were not loaded (missing: " +
        missing.join(", ") + ").";
      return;
    }

    try {
      console.log("Loading firebase...");
      const app = firebase.apps.length > 0 ? firebase.app() : firebase.initializeApp(configuration());

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

      if (emulatorsWanted() && !emulatorsUsed) {
        console.log("Using the Firebase emulators on 127.0.0.1");
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

  // What to tell the person when the project does not accept this page.
  function rejection(config) {
    const project = config.projectId === "homotopy-io" ? "homotopy.io Firebase project"
      : "Firebase project " + config.projectId;
    return (config.projectId === "homotopy-io" ? "Cloud storage only works on homotopy.io: " : "") +
      "this copy is served from " + location.origin + ", which the " + project +
      " doesn't accept. To use cloud storage here, configure your own Firebase project " +
      "(see the notes at the top of static/firebase.js).";
  }

  // If an error from the Firebase SDK is the project not accepting this page (for the key that
  // is restricted to other sites: `auth/requests-from-referer-<site>-are-blocked`, and for sign-in
  // from a domain that is not authorised: `auth/unauthorized-domain`), says so, and makes
  // Firebase unavailable. Otherwise `null`.
  cloud.explain = function (error) {
    const text = String((error && (error.code || "")) + " " + (error && error.message || error));
    if (/requests-from-referer|unauthorized-domain|API_KEY_HTTP_REFERRER_BLOCKED/i.test(text)) {
      cloud.ready = false;
      cloud.rejected = true;
      cloud.error = rejection(configuration());
      return cloud.error;
    }
    // Calls to emulators that are not running fail like a lost connection does.
    if (emulatorsUsed && /network-request-failed|unavailable|failed to fetch|networkerror|refused/i.test(text)) {
      return "The Firebase emulators on 127.0.0.1 (auth 9099, firestore 8080, storage 9199, " +
        "functions 5001) could not be reached: " + String(error && error.message || error) +
        " They are only used because the page was opened with ?emulators (or " +
        "homotopy-firebase-emulators is set in localStorage); remove that to use the real project.";
    }
    return null;
  };

  // Whether the project accepts this page. This asks what the Firebase SDK asks first when it
  // signs someone in (the project's configuration, with the same API key), so that the answer is
  // the one sign-in would get, before the person has gone through the sign-in options to find it
  // out. Resolves to `null` if it does, or why not; and if it does not, Firebase is made
  // unavailable with that message (`cloud.rejected`). Not being able to ask (offline, a slow
  // network, an answer that is not understood) is not a rejection.
  function probe() {
    if (!cloud.ready || emulatorsUsed) return Promise.resolve(null);
    if (probing) return probing;
    const config = configuration();
    const origin = location.origin;
    const reject = (why) => {
      cloud.ready = false;
      cloud.rejected = true;
      cloud.error = rejection(config);
      console.warn("Firebase does not accept this page:", why);
      return cloud.error;
    };
    probing = (async () => {
      try {
        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), 8000);
        const response = await fetch(
          "https://identitytoolkit.googleapis.com/v1/projects?key=" + encodeURIComponent(config.apiKey),
          { signal: controller.signal });
        clearTimeout(timer);
        const body = await response.json();
        if (body && body.error) {
          const text = JSON.stringify(body.error);
          // "Requests from referer http://127.0.0.1:5000/ are blocked." (API_KEY_HTTP_REFERRER_BLOCKED)
          if (/REFERRER_BLOCKED|referer.*blocked/i.test(text)) return reject("the API key is restricted to other sites");
          return null;
        }
        if (body && Array.isArray(body.authorizedDomains) &&
            !body.authorizedDomains.includes(location.hostname)) {
          return reject(location.hostname + " is not an authorised domain, so signing in would fail");
        }
      } catch (e) {
        console.warn("Could not ask Firebase whether this page is accepted:", e);
      }
      return null;
    })();
    return probing;
  }

  window.homotopyCloudProbe = probe;

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
    if (cloud.rejected) return Promise.resolve(cloud.error); // not a matter of scripts
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
