// Native bridge: WebView2 postMessage out, `window.__signetReply` back.
// In a plain browser (Vite dev only) a read-only preview backend serves a real snapshot.

export const native = Boolean(window.__SIGNET_NATIVE__ && window.ipc);
// Transparent page only when Windows painted Mica behind it (Windows 11).
if (native && window.__SIGNET_MICA__) document.documentElement.classList.add("native");

let seq = 1;
const pending = new Map();
const listeners = new Set();

window.__signetReply = (m) => {
  const p = pending.get(m.id);
  if (!p) return;
  pending.delete(m.id);
  if (m.ok) p.resolve(m.result);
  else p.reject(new Error(m.error));
};
window.__signetEvent = (e) => listeners.forEach((fn) => fn(e));

export function onEvent(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

let preview;
async function previewCall(cmd, args) {
  if (import.meta.env.DEV) {
    if (!preview) preview = import("./preview.js").then((m) => m.createPreview());
    return (await preview)(cmd, args);
  }
  throw new Error("The Signet bridge is unavailable.");
}

export function call(cmd, args = {}) {
  if (!native) return previewCall(cmd, args);
  const id = seq++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    window.ipc.postMessage(JSON.stringify({ id, cmd, args }));
  });
}
