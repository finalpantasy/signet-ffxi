// Browser preview backend (Vite dev only). Serves a snapshot written by
// `signet-launcher --snapshot ui/launcher/dev-data/<name>.json`, so screens show real data.
// Actions that would start a process are refused here.
// Browser preview only: it never starts PlayOnline or the zone viewer.

const snaps = import.meta.glob("../dev-data/*.json", { import: "default" });

export async function createPreview() {
  const q = new URLSearchParams(location.search);
  const name = q.get("snap") || "snapshot";
  const load = snaps[`../dev-data/${name}.json`];
  if (!load) throw new Error(`No preview snapshot dev-data/${name}.json`);
  const snap = structuredClone(await load());
  const s = snap.init.settings;
  if (q.get("theme")) s.theme = q.get("theme");
  if (q.get("zone")) s.zone_id = Number(q.get("zone"));
  if (q.get("recent")) s.recent = q.get("recent").split(",").map(Number);
  if (q.get("fav")) s.favorites = q.get("fav").split(",").map(Number);
  // State overrides for screenshots: ?pol=<pid> (PlayOnline running), ?viewer=<zone id>
  // (zone viewer running), ?client=current|outdated|missing|unknown, ?exit=<code>.
  const retail = snap.retail;
  const game = snap.init.game;
  if (q.get("pol")) retail.pid = Number(q.get("pol"));
  if (q.get("client") === "missing") retail.viewer.found = false;
  else if (q.get("client")) retail.state = q.get("client");
  if (q.get("viewer")) Object.assign(game, { running: true, pid: 41872, zone: Number(q.get("viewer")) });
  if (q.get("exit")) game.lastExit = { code: Number(q.get("exit")), line: "signet-render: zone has no collision mesh" };
  const wait = (v, ms = 30) => new Promise((r) => setTimeout(() => r(structuredClone(v)), ms));
  return async (cmd, args) => {
    switch (cmd) {
      case "init":
        return wait(snap.init);
      case "zones":
        return wait(snap.zones);
      case "retail.info":
        return wait(retail, q.get("slow") ? 600000 : 30);
      case "status":
        return wait({ game, retail: { pid: retail.pid ?? null } });
      case "update.state":
      case "update.check":
        return wait({
          check: { status: q.get("update") || "available", installed: retail.installed, server: "30261102_1", message: null },
          job: { phase: "idle", progress: {} },
          restore: { phase: "idle" },
          backup: null,
          blocked: [],
        });
      case "settings.set": {
        const { favorite, ...rest } = args;
        Object.assign(s, rest);
        if (favorite) {
          s.favorites = s.favorites.filter((z) => z !== favorite[0]);
          if (favorite[1]) s.favorites.unshift(favorite[0]);
        }
        return wait(s);
      }
      default:
        throw new Error("Browser preview: this action runs only in the Signet window.");
    }
  };
}
