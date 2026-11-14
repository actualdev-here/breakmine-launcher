// Read-only status. Never send account or token data to the host.
(() => {
  if (window.top !== window) return;
  let busy = false;
  async function report() {
    if (busy || !window.ipc || !window.BreakmineLauncher) return;
    const api = window.BreakmineLauncher;
    if (typeof api.game?.getState !== 'function' || typeof api.server?.getPlayers !== 'function') return;
    busy = true;
    try {
      const state = await api.game.getState();
      const server = await api.server.getPlayers();
      const value = {
        state: typeof state?.state === 'string' ? state.state.slice(0, 80) : null,
        world: typeof state?.world === 'string' ? state.world.slice(0, 80) : null,
        paused: state?.paused === true,
        singleplayer: typeof state?.singleplayer === 'boolean' ? state.singleplayer : null,
        connected: server?.connected === true,
        players: server?.connected && Array.isArray(server.players)
          ? server.players.slice(0, 128).map(p => String(p?.username || '').slice(0, 80)) : [],
      };
      window.ipc.postMessage(JSON.stringify(value));
    } catch (_) { /* Older game versions can omit these methods. */ }
    finally { busy = false; }
  }
  setInterval(report, 3000);
})();
