// This file is included in `account.rs`.
//
// Firebase may not be there (its scripts come from gstatic.com, which an extension may block, and
// `static/firebase.js` says whether it started: `window.homotopyCloud`). Everything here that
// needs it throws a plain `Error` in that case, and is imported with `catch` on the Rust side, so
// that it is a message for the person and not a crash.

function cloud() {
  const c = window.homotopyCloud;
  if (!c || !c.ready) {
    throw new Error((c && c.error) || "Firebase is not available.");
  }
  return c;
}

// Why Firebase is not available, or `null` if it is.
export function cloudError() {
  const c = window.homotopyCloud;
  if (c && c.ready) return null;
  return (c && c.error) || "Firebase is not available.";
}

// Tries to load Firebase again. Resolves to why it is not available, or `null` if it is now.
export function retryCloud() {
  if (typeof window.homotopyCloudRetry !== "function") {
    return Promise.resolve(cloudError());
  }
  return window.homotopyCloudRetry();
}

export function getFirebase() {
  return cloud().firebase;
}

export function initUI(onSignInSuccessWithAuthResult) {
  const c = cloud();
  // Initialise Firebase UI
  c.ui.start('#firebaseui-auth-container', {
    signInOptions: [
      // List of OAuth providers supported.
      firebase.auth.GoogleAuthProvider.PROVIDER_ID,
      firebase.auth.GithubAuthProvider.PROVIDER_ID
    ],
    signInFlow: "popup",
    callbacks: {
      signInSuccessWithAuthResult: function (authResult, redirectUrl) {
        onSignInSuccessWithAuthResult(authResult);
        return false; // don't continue to redirect automatically
      },
    }
  });
}

export async function publishPersonal(id) {
  const f = cloud().functions.httpsCallable('publishPersonal');
  return f({ id });
}
