import React from "react";
import * as Icon from "./icons.jsx";
import { Dot, Seg, Stepper, ZoneArt, fmtDay } from "./ui.jsx";
import { UpdatePrompt, isUpdating } from "./Update.jsx";

export default function Home({ init, settings, update, zoneById, art, artById, heroArt, game, retail, client, busy, playRetail, explore, stopExplore, go, upd, act }) {
  const zone = zoneById.get(settings.zone_id);
  const viewing = game?.running;
  const [min, max] = init?.speed ?? [-50, 100];
  const polRunning = client.state === "running";
  const noPol = retail ? !retail.viewer.found : false;

  // The selected zone first, then recent zones, then zones that have art.
  const picks = [settings.zone_id, ...settings.recent, ...art.filter((a) => !a.alt).map((a) => a.id)]
    .filter((id, i, all) => all.indexOf(id) === i && zoneById.has(id))
    .slice(0, 5);
  const lastExit = game?.lastExit;

  return (
    <>
      <div className="hero">
        <ZoneArt art={heroArt} className="hero-art" />
        <div className="hero-top">
          <button type="button" className="chip glass-dark" onClick={() => go("retail")}>
            <Dot state={client.dot} />
            {client.short}
            {polRunning && <span className="chip-dim">pid {retail.pid}</span>}
          </button>
          <span className="grow" />
          {heroArt && <span className="credit">{heroArt.name} · drawn by Signet</span>}
        </div>
        <div className="hero-bottom">
          <div className="hero-title">
            <div className="eyebrow">Retail · PlayOnline</div>
            <h1>FINAL FANTASY XI</h1>
            <div className="hero-meta">
              {retail?.installedDate ? `Installed build ${fmtDay(retail.installedDate)}` : retail ? "Installed build unknown" : "Reading the install"}
            </div>
          </div>
          <div className="hero-controls glass-dark play-card">
            {!polRunning && <UpdatePrompt upd={upd} retail={retail} act={act} go={go} />}
            {polRunning && <p className="play-note quiet">PlayOnline is open. Close it there to play again.</p>}
            <button
              type="button"
              className="btn gold big"
              onClick={playRetail}
              disabled={polRunning || busy === "retail" || noPol || !retail || isUpdating(upd)}
            >
              {polRunning ? <Dot state="ready" /> : <Icon.Play />}
              {busy === "retail" ? "Opening PlayOnline" : polRunning ? "PlayOnline running" : "Play FINAL FANTASY XI"}
            </button>
          </div>
        </div>
      </div>

      <section className="panel explore" aria-label="Zone viewer">
        <button type="button" className="explore-thumb" onClick={() => go("zones")} aria-label={`Change zone (now ${zone?.name ?? settings.zone_id})`}>
          <ZoneArt art={artById.get(viewing ? game.zone : settings.zone_id)} />
          <Icon.Swap />
        </button>
        <div className="explore-what">
          <span className="label">{viewing ? "Zone viewer running" : "Zone viewer · offline"}</span>
          <b>{viewing ? zoneById.get(game.zone)?.name ?? `Zone ${game.zone}` : zone?.name ?? (zoneById.size ? "Unknown zone" : "Loading zones")}</b>
        </div>
        {!viewing && lastExit && lastExit.code !== 0 && (
          <span className="chip warn" title={lastExit.line}>
            Last run exited ({lastExit.code})
          </span>
        )}
        <span className="grow" />
        <div className="ctl">
          <span>Camera</span>
          <Seg
            label="Camera mode"
            value={settings.camera_mode}
            options={[
              ["retail", "Retail"],
              ["free", "Free"],
            ]}
            onChange={(v) => update({ camera_mode: v })}
          />
        </div>
        <div className="ctl">
          <span>Speed</span>
          <Stepper label="Movement speed" value={settings.speed_mod} min={min} max={max} onChange={(v) => update({ speed_mod: v })} />
        </div>
        <div className="sep" />
        {viewing ? (
          <button type="button" className="btn" onClick={stopExplore} disabled={busy === "stop"}>
            <Icon.Stop />
            {busy === "stop" ? "Closing" : "Close viewer"}
          </button>
        ) : (
          <button type="button" className="btn" onClick={() => explore()} disabled={!zone || busy === "explore"}>
            <Icon.Compass />
            {busy === "explore" ? "Opening" : "Explore"}
          </button>
        )}
      </section>

      {picks.length > 1 && (
        <div className="strip" role="group" aria-label="Zones to explore">
          {picks.map((id) => (
            <button key={id} type="button" className="tile" aria-current={id === settings.zone_id} onClick={() => update({ zone_id: id })} aria-label={`Select ${zoneById.get(id)?.name}`}>
              <ZoneArt art={artById.get(id)} />
              <span>{zoneById.get(id)?.name}</span>
            </button>
          ))}
        </div>
      )}
    </>
  );
}
