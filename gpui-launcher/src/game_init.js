// Injected into the game window before the page runs (wrapped as `(function (C) { … })(config)`).
// C = { account: {username, token} | null, servers: [...], join: {name, address} | null, mods: {files, binary} }
'use strict';
if (window.top !== window) return;

// ── window.modsBridge: the launcher's mods folder, read-only ──────────────
// Same contract as the Electron app's preload (src/js/net/minecraft/server/preload.js).
const files = C.mods.files || {};
const binary = C.mods.binary || {};
const all = Object.keys(files).concat(Object.keys(binary));
const norm = (p) => String(p || '').replace(/\\/g, '/').replace(/^\/+/, '');
const bytes = (b64) => {
  const s = atob(b64);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
};
const readOnly = async () => { throw new Error('Mods are managed in Breakmine Launcher.'); };
Object.defineProperty(window, 'modsBridge', {
  configurable: false,
  value: Object.freeze({
    isPackaged: false,
    loadFile: async (f) => (Object.prototype.hasOwnProperty.call(files, norm(f)) ? files[norm(f)] : null),
    loadBinaryFile: async (f) => (Object.prototype.hasOwnProperty.call(binary, norm(f)) ? bytes(binary[norm(f)]) : null),
    listDir: async (dir) => {
      let d = norm(dir);
      if (d && !d.endsWith('/')) d += '/';
      return all.filter((p) => p.startsWith(d));
    },
    fileExists: async (f) => all.includes(norm(f)),
    getFileSize: async (f) => {
      const k = norm(f);
      if (k in files) return new TextEncoder().encode(files[k]).length;
      if (k in binary) return Math.floor((binary[k].length * 3) / 4);
      return 0;
    },
    saveFile: readOnly,
    saveBinaryFile: readOnly,
    deleteFile: readOnly,
    openFolder: () => {},
    reportGameState: () => {},
  }),
});

// ── sign-in, servers, auto-join (once the game has booted) ────────────────
const waitFor = (get, ms) => new Promise((resolve, reject) => {
  const start = Date.now();
  (function poll() {
    let v;
    try { v = get(); } catch (e) { v = null; }
    if (v) return resolve(v);
    if (Date.now() - start > ms) return reject(new Error('timed out'));
    setTimeout(poll, 150);
  })();
});
const log = (...a) => console.log('[Breakmine Launcher]', ...a);
const fileName = (name) => String(name).replaceAll(' ', '_').replaceAll('/', '_') + '.json';
// Quick Tunnels use secure WebSockets on port 443. Keep the shared address short
// in the launcher while providing the port expected by older game builds.
const gameAddress = (address) => {
  const value = String(address || '').trim();
  return /^[a-z0-9-]+\.trycloudflare\.com$/i.test(value) ? value + ':443' : value;
};

async function boot() {
  const api = await waitFor(() => window.BreakmineLauncher, 90000);
  const app = await waitFor(() => (window.app && window.app.fs && window.app.settings ? window.app : null), 90000);

  if (C.account) {
    try {
      await api.accounts.loginWithToken({ username: C.account.username, token: C.account.token });
      log('signed in as', C.account.username);
    } catch (e) {
      log('sign-in failed:', e && e.message);
    }
  }

  // Mirror the launcher's servers into the in-game Multiplayer list (same file shape as "Add Server").
  for (const s of C.servers || []) {
    try {
      const address = gameAddress(s.address);
      await app.fs.saveFile(JSON.stringify({ name: s.name, details: address, address, motd: s.motd || '', protocol: s.protocol }), 'servers/' + fileName(s.name));
    } catch (e) {
      log('could not add server', s.name, e && e.message);
    }
  }

  if (!C.join) return;
  try {
    app.settings.serverAddress = gameAddress(C.join.address);
    app.settings.save();
  } catch (e) { /* prefill only */ }

  // Main menu → Multiplayer → the matching server. Needs a signed-in game (Multiplayer is disabled otherwise).
  const target = gameAddress(C.join.address).toLowerCase();
  try {
    const menu = await waitFor(() => {
      const scr = app.currentScreen;
      const b = scr && Array.isArray(scr.buttonList) && scr.buttonList.find((x) => x && x.string === 'Multiplayer');
      return b && b.enabled ? b : null;
    }, 90000);
    menu.onPress();
    const mp = await waitFor(() => {
      const scr = app.currentScreen;
      const slots = scr && typeof scr.joinServer === 'function' && scr.worldSlotContainer && scr.worldSlotContainer.slotList;
      if (!slots) return null;
      const i = slots.findIndex((x) => gameAddress(x.worldDate).toLowerCase() === target);
      return i >= 0 ? { scr, i } : null;
    }, 15000);
    mp.scr.joinServer(mp.i);
    log('joining', C.join.address);
  } catch (e) {
    log('auto-join skipped:', e && e.message, '— the server is in the Multiplayer list.');
  }
}

boot().catch((e) => log('launcher hookup skipped:', e && e.message));
