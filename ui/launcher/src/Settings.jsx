import React from "react";
import { call } from "./bridge.js";
import * as Icon from "./icons.jsx";
import { Seg, speedText } from "./ui.jsx";

export default function Settings({ init, settings, update, notify }) {
  const [min, max] = init?.speed ?? [-50, 100];
  const res = init?.resolutions ?? [];
  const open = (target) => call("open", { target }).catch((e) => notify(e.message, true));
  const fill = `${((settings.speed_mod - min) / (max - min)) * 100}%`;

  return (
    <>
      <div className="head">
        <h1>Settings</h1>
      </div>
      <div className="settings">
        <section className="panel group">
          <h2>Appearance</h2>
          <div className="setting">
            <div className="what">
              <b>Theme</b>
              <span>System follows Windows.</span>
            </div>
            <Seg
              label="Theme"
              value={settings.theme}
              options={[
                ["system", "System"],
                ["light", "Light"],
                ["dark", "Dark"],
              ]}
              onChange={(v) => update({ theme: v })}
            />
          </div>
        </section>

        <section className="panel group">
          <h2>Zone viewer</h2>
          <div className="setting">
            <div className="what">
              <b>Camera</b>
              <span>Retail follows the character. Free flies anywhere.</span>
            </div>
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
          <div className="setting">
            <div className="what">
              <b>Movement speed</b>
              <span>Even steps, like gear and buffs.</span>
            </div>
            <input
              type="range"
              min={min}
              max={max}
              step={2}
              value={settings.speed_mod}
              style={{ "--fill": fill }}
              aria-label="Movement speed"
              aria-valuetext={speedText(settings.speed_mod)}
              onChange={(e) => update({ speed_mod: Number(e.target.value) })}
            />
            <span className="val">{speedText(settings.speed_mod)}</span>
          </div>
          <div className="setting">
            <div className="what">
              <b>Resolution</b>
              <span>Windowed only for now.</span>
            </div>
            <select
              className="field"
              aria-label="Resolution"
              value={`${settings.width}x${settings.height}`}
              onChange={(e) => {
                const [width, height] = e.target.value.split("x").map(Number);
                update({ width, height });
              }}
            >
              {res.map(([w, h]) => (
                <option key={`${w}x${h}`} value={`${w}x${h}`}>
                  {w} × {h}
                </option>
              ))}
            </select>
          </div>
        </section>

        <section className="panel group">
          <h2>Folders</h2>
          {[
            ["install", "FFXI install", init?.paths.install, "Read only"],
            ["viewer", "PlayOnline", init?.paths.viewer, "Read only"],
            ["settings", "Settings file", init?.paths.settings, null],
          ].map(([target, name, path, note]) => (
            <div className="setting" key={target}>
              <div className="what">
                <b>
                  {name}
                  {note && <span className="chip tag">{note}</span>}
                </b>
                <span className="mono selectable" title={path}>
                  {path}
                </span>
              </div>
              <button type="button" className="btn" onClick={() => open(target)}>
                <Icon.Folder />
                Open
              </button>
            </div>
          ))}
        </section>
      </div>
    </>
  );
}
