import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { call, native, onEvent } from "./bridge.js";
import * as Icon from "./icons.jsx";
import { Dot, clientSummary } from "./ui.jsx";
import { isUpdating } from "./Update.jsx";
import Home from "./Home.jsx";
import Zones from "./Zones.jsx";
import Retail from "./Retail.jsx";
import Settings from "./Settings.jsx";
import Mods from "./Mods.jsx";

const SCREENS = [
  ["home", "Home", Icon.Home],
  ["retail", "Retail client", Icon.Crystal],
  ["zones", "Zones", Icon.Globe],
  ["settings", "Settings", Icon.Gear],
  ["mods", "Mods", Icon.Blocks],
];

function useSystemDark() {
  const mq = useMemo(() => window.matchMedia("(prefers-color-scheme: dark)"), []);
  const [dark, setDark] = useState(mq.matches);
  useEffect(() => {
    const f = () => setDark(mq.matches);
    mq.addEventListener("change", f);
    return () => mq.removeEventListener("change", f);
  }, [mq]);
  return dark;
}

export default function App() {
  const [screen, setScreen] = useState(() => new URLSearchParams(location.search).get("screen") || "home");
  const [init, setInit] = useState(null);
  const [settings, setSettings] = useState(null);
  const [zones, setZones] = useState(null);
  const [art, setArt] = useState([]);
  const [game, setGame] = useState(null);
  const [retail, setRetail] = useState(null);
  const [busy, setBusy] = useState(null); // "retail" | "explore" | "stop" while a launch or stop is in flight
  const [upd, setUpd] = useState({ check: { status: "idle" }, job: { phase: "idle", progress: {} }, restore: { phase: "idle" }, backup: null, blocked: [] });
  const [toast, setToast] = useState(null);
  const [fatal, setFatal] = useState(null);
  const toastTimer = useRef();
  const systemDark = useSystemDark();

  const notify = useCallback((text, err = false) => {
    clearTimeout(toastTimer.current);
    setToast({ text, err });
    toastTimer.current = setTimeout(() => setToast(null), err ? 7000 : 3500);
  }, []);

  const loadRetail = useCallback(() => call("retail.info").then(setRetail).catch((e) => notify(e.message, true)), [notify]);
  const refresh = useCallback(
    () =>
      call("status")
        .then((s) => {
          setGame(s.game);
          setRetail((r) => (r ? { ...r, pid: s.retail.pid } : r));
          setUpd((u) => ((u.blocked ?? []).join() === (s.retail.blocked ?? []).join() ? u : { ...u, blocked: s.retail.blocked ?? [] }));
        })
        .catch(() => {}),
    [],
  );

  useEffect(() => {
    call("init")
      .then((i) => {
        setInit(i);
        setSettings(i.settings);
        setGame(i.game);
      })
      .catch((e) => setFatal(e.message));
    call("zones").then(setZones).catch((e) => {
      setZones([]);
      notify(e.message, true);
    });
    loadRetail();
    fetch("./zones/index.json")
      .then((r) => (r.ok ? r.json() : []))
      .then(setArt)
      .catch(() => setArt([]));
  }, [notify, loadRetail]);

  // Updates: the backend pushes the whole state on every change; replies only carry errors.
  useEffect(() => onEvent((e) => e.type === "update" && setUpd((u) => ({ ...u, ...e.state }))), []);
  useEffect(() => {
    call("update.state")
      .then((s) => setUpd((u) => ({ ...u, ...s })))
      .catch(() => {})
      .then(() => call("update.check"))
      .then((s) => !native && setUpd((u) => ({ ...u, ...s })))
      .catch(() => {});
  }, []);
  const phase = `${upd.job.phase}/${upd.restore.phase}`;
  useEffect(() => {
    if (["done/idle", "idle/done", "failed/idle"].includes(phase)) loadRetail();
  }, [phase, loadRetail]);
  const act = useMemo(() => {
    const run = (cmd) => () => call(cmd).catch((e) => notify(e.message, true));
    return { check: run("update.check"), plan: run("update.plan"), apply: run("update.apply"), cancel: run("update.cancel"), dismiss: run("update.dismiss"), restore: run("update.restore") };
  }, [notify]);

  // Process state: fast while something runs or starts, slow otherwise. Client files every 15 s.
  const live = game?.running || retail?.pid != null || busy || isUpdating(upd);
  useEffect(() => {
    const t = setInterval(refresh, live ? 1500 : 3000);
    return () => clearInterval(t);
  }, [refresh, live]);
  useEffect(() => {
    const t = setInterval(loadRetail, 15000);
    return () => clearInterval(t);
  }, [loadRetail]);

  const theme = settings?.theme ?? "system";
  const dark = theme === "dark" || (theme === "system" && systemDark);
  useEffect(() => {
    document.documentElement.dataset.theme = dark ? "dark" : "light";
  }, [dark]);

  const update = useCallback(
    async (patch) => {
      try {
        setSettings(await call("settings.set", patch));
      } catch (e) {
        notify(e.message, true);
      }
    },
    [notify],
  );

  const playRetail = useCallback(async () => {
    if (busy) return;
    setBusy("retail");
    try {
      const r = await call("retail.play");
      setRetail((s) => ({ ...s, pid: r.pid }));
      notify("PlayOnline opened");
    } catch (e) {
      notify(e.message, true);
    } finally {
      setBusy(null);
      refresh();
    }
  }, [busy, notify, refresh]);

  const explore = useCallback(
    async (zoneId) => {
      if (busy) return;
      setBusy("explore");
      try {
        const r = await call("explore", zoneId == null ? {} : { zone_id: zoneId });
        setSettings((s) => ({ ...s, zone_id: r.zone, recent: [r.zone, ...s.recent.filter((z) => z !== r.zone)].slice(0, 8) }));
      } catch (e) {
        notify(e.message, true);
      } finally {
        setBusy(null);
        refresh();
      }
    },
    [busy, notify, refresh],
  );

  const stopExplore = useCallback(async () => {
    if (busy) return;
    setBusy("stop");
    try {
      setGame(await call("explore.stop"));
    } catch (e) {
      notify(e.message, true);
    } finally {
      setBusy(null);
      refresh();
    }
  }, [busy, notify, refresh]);

  useEffect(() => {
    const onKey = (e) => {
      if (e.ctrlKey && !e.altKey && e.key >= "1" && e.key <= String(SCREENS.length)) {
        e.preventDefault();
        setScreen(SCREENS[Number(e.key) - 1][0]);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const zoneById = useMemo(() => new Map((zones || []).map((z) => [z.id, z])), [zones]);
  // One image per zone: the first entry that is not an alternate view.
  const artById = useMemo(() => {
    const m = new Map();
    for (const a of art) if (!a.alt && !m.has(a.id)) m.set(a.id, a);
    return m;
  }, [art]);
  const heroArt = useMemo(() => art.find((a) => a.hero) ?? art[0], [art]);

  if (fatal) {
    return (
      <div className="soon">
        <div>
          <Icon.Alert />
          <h2>Signet could not start</h2>
          <p className="selectable">{fatal}</p>
        </div>
      </div>
    );
  }
  if (!settings) return null;

  const client = clientSummary(retail, busy === "retail", upd);
  const viewerZone = game?.running ? zoneById.get(game.zone)?.name : null;
  const ctx = { init, settings, update, zones, zoneById, art, artById, heroArt, game, retail, client, busy, playRetail, explore, stopExplore, notify, go: setScreen, upd, act };

  return (
    <div className="shell">
      <aside className="side">
        <div className="brand">
          <Icon.Mark className="brand-mark" />
          <span className="brand-name">Signet</span>
        </div>
        <nav className="nav" aria-label="Sections">
          {SCREENS.map(([id, label, Ico], i) => (
            <button key={id} type="button" className="nav-item" aria-current={screen === id ? "page" : undefined} onClick={() => setScreen(id)} aria-keyshortcuts={`Control+${i + 1}`}>
              <Ico />
              {label}
              <span className="kbd" aria-hidden="true">
                Ctrl {i + 1}
              </span>
            </button>
          ))}
        </nav>
        <div className="side-foot">
          <button type="button" className="side-status" onClick={() => setScreen("retail")}>
            <Dot state={client.dot} />
            <div>
              <b>FINAL FANTASY XI</b>
              <span>{client.short}</span>
            </div>
          </button>
          {game?.running && (
            <button type="button" className="side-status" onClick={() => setScreen("zones")}>
              <Dot state="ready" />
              <div>
                <b>Zone viewer</b>
                <span>{viewerZone ?? `Zone ${game.zone}`}</span>
              </div>
            </button>
          )}
          <div className="side-ver">
            v{init?.version}
            {native ? "" : " · browser preview"}
          </div>
        </div>
      </aside>
      <main className="main">
        <section key={screen} className="screen enter" aria-label={SCREENS.find((s) => s[0] === screen)?.[1]}>
          {screen === "home" && <Home {...ctx} />}
          {screen === "zones" && <Zones {...ctx} />}
          {screen === "retail" && <Retail {...ctx} />}
          {screen === "settings" && <Settings {...ctx} />}
          {screen === "mods" && <Mods />}
        </section>
      </main>
      {toast && (
        <div className={`toast panel ${toast.err ? "err" : ""}`} role={toast.err ? "alert" : "status"}>
          {toast.err && <Icon.Alert />}
          {toast.text}
        </div>
      )}
    </div>
  );
}
