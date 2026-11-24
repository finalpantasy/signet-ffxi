import React, { useEffect, useMemo, useRef, useState } from "react";
import * as Icon from "./icons.jsx";
import { Seg, ZoneArt } from "./ui.jsx";

const norm = (s) => s.toLowerCase().normalize("NFKD").replace(/[^a-z0-9 ]/g, "");

export default function Zones({ settings, update, zones, zoneById, artById, game, busy, explore, stopExplore }) {
  const [q, setQ] = useState("");
  const [tab, setTab] = useState("all");
  const [sel, setSel] = useState(settings.zone_id);
  const search = useRef();
  const list = useRef();

  const shown = useMemo(() => {
    if (!zones) return [];
    let base = zones;
    if (tab === "favorites") base = settings.favorites.map((id) => zoneById.get(id)).filter(Boolean);
    if (tab === "recent") base = settings.recent.map((id) => zoneById.get(id)).filter(Boolean);
    const n = norm(q.trim());
    if (!n) return base;
    return base.filter((z) => norm(z.name).includes(n) || String(z.id) === n);
  }, [zones, zoneById, tab, q, settings.favorites, settings.recent]);

  useEffect(() => {
    const onKey = (e) => {
      if (e.key === "/" && document.activeElement !== search.current) {
        e.preventDefault();
        search.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    list.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "center" });
  }, [sel, shown]);

  const move = (d) => {
    if (!shown.length) return;
    const i = shown.findIndex((z) => z.id === sel);
    const next = shown[Math.max(0, Math.min(shown.length - 1, (i < 0 ? -1 : i) + d))];
    setSel(next.id);
  };
  const onListKey = (e) => {
    if (e.key === "ArrowDown") (e.preventDefault(), move(1));
    else if (e.key === "ArrowUp") (e.preventDefault(), move(-1));
    else if (e.key === "Enter" && sel != null) update({ zone_id: sel });
    else if (e.key.toLowerCase() === "f" && sel != null && e.target.tagName !== "INPUT") update({ favorite: [sel, !settings.favorites.includes(sel)] });
  };

  const z = zoneById.get(sel);
  const fav = (id) => settings.favorites.includes(id);
  const current = sel === settings.zone_id;

  return (
    <>
      <div className="head">
        <h1>Zones</h1>
        <span className="sub">{zones ? `${zones.length} in this install · offline viewer` : "Reading the install"}</span>
        <span className="grow" />
        <Seg
          label="Filter"
          value={tab}
          options={[
            ["all", "All"],
            ["favorites", "Favorites"],
            ["recent", "Recent"],
          ]}
          onChange={setTab}
        />
        <label className="search">
          <Icon.Search />
          <span className="sr">Search zones</span>
          <input
            ref={search}
            className="field"
            type="search"
            placeholder="Search by name or id"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Enter") onListKey(e);
              if (e.key === "Escape") setQ("");
            }}
            spellCheck="false"
          />
          {!q && <span className="kbd-hint">/</span>}
        </label>
      </div>
      <div className="zones">
        <div className="panel list" ref={list} role="listbox" aria-label="Zones" tabIndex={0} onKeyDown={onListKey} aria-activedescendant={sel != null ? `zone-${sel}` : undefined}>
          {shown.map((zone) => (
            <div
              key={zone.id}
              id={`zone-${zone.id}`}
              role="option"
              aria-selected={zone.id === sel}
              className="row"
              onClick={() => setSel(zone.id)}
              onDoubleClick={() => update({ zone_id: zone.id })}
            >
              <span className="id mono">{zone.id}</span>
              <span className="name">{zone.name}</span>
              {game?.running && zone.id === game.zone ? <span className="current">Open in viewer</span> : zone.id === settings.zone_id && <span className="current">Selected</span>}
              {artById.has(zone.id) && <span className="art-mark" title="Has zone art" />}
              <button
                type="button"
                className="star"
                tabIndex={-1}
                aria-pressed={fav(zone.id)}
                aria-label={fav(zone.id) ? `Remove ${zone.name} from favorites` : `Add ${zone.name} to favorites`}
                onClick={(e) => {
                  e.stopPropagation();
                  update({ favorite: [zone.id, !fav(zone.id)] });
                }}
              >
                <Icon.Star filled={fav(zone.id)} />
              </button>
            </div>
          ))}
          {zones && !shown.length && (
            <div className="empty">
              {tab === "favorites" && !q ? "Star a zone to pin it here." : tab === "recent" && !q ? "Zones you explore show up here." : "0 zones match."}
            </div>
          )}
        </div>
        <aside className="panel preview" aria-label="Selected zone">
          {artById.get(sel) ? <ZoneArt art={artById.get(sel)} className="preview-art" alt="" /> : <div className="preview-art fill" aria-hidden="true" />}
          {z ? (
            <div className="preview-body">
              <div className="label on-art-2">
                Zone {z.id}
                {artById.has(z.id) ? "" : " · no image yet"}
              </div>
              <h2>{z.name}</h2>
              <div className="preview-actions">
                <button type="button" className="btn glass-dark" onClick={() => update({ zone_id: z.id })} disabled={current}>
                  {current ? "Selected" : "Select"}
                </button>
                <button type="button" className="btn glass-dark icon" aria-pressed={fav(z.id)} aria-label={fav(z.id) ? "Remove from favorites" : "Add to favorites"} onClick={() => update({ favorite: [z.id, !fav(z.id)] })}>
                  <Icon.Star filled={fav(z.id)} style={{ color: fav(z.id) ? "var(--gold-hi)" : undefined }} />
                </button>
                <span className="grow" />
                {game?.running ? (
                  <button type="button" className="btn glass-dark strong" onClick={stopExplore} disabled={busy === "stop"}>
                    <Icon.Stop />
                    {busy === "stop" ? "Closing" : "Close viewer"}
                  </button>
                ) : (
                  <button type="button" className="btn glass-dark strong" onClick={() => explore(z.id)} disabled={busy === "explore"}>
                    <Icon.Compass />
                    {busy === "explore" ? "Opening" : "Explore here"}
                  </button>
                )}
              </div>
            </div>
          ) : (
            <div className="preview-body">Pick a zone from the list.</div>
          )}
        </aside>
      </div>
    </>
  );
}
