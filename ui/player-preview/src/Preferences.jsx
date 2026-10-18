import React from "react";
import { RangeField, Toggle } from "./components.jsx";

export function Graphics({ profile, update }) {
  const set = (key, value) =>
    update({ graphics: { ...profile.graphics, [key]: value } });
  const choices = (id, label, options) => (
    <div className="field">
      <label htmlFor={id}>{label}</label>
      <select
        id={id}
        value={profile.graphics[id]}
        onChange={(e) => set(id, e.target.value)}
      >
        {options.map((v) => (
          <option key={v}>{v}</option>
        ))}
      </select>
    </div>
  );
  return (
    <section className="settings-surface" aria-labelledby="display-heading">
      <h2 id="display-heading">Game display</h2>
      <div className="two-fields">
        {choices("resolution", "Game resolution", [
          "3840 × 2160",
          "2560 × 1440",
          "1920 × 1080",
        ])}
        {choices("windowMode", "Window style", [
          "Borderless window",
          "Windowed",
        ])}
      </div>
      <RangeField
        field={{
          id: "game-ui-scale",
          label: "In-game interface size",
          min: 1,
          max: 2,
          step: 0.1,
          unit: "×",
        }}
        value={profile.graphics.uiScale}
        onChange={(value) => set("uiScale", value)}
      />
      <details className="advanced">
        <summary>Advanced graphics</summary>
        <p className="helper">
          Preferences only. Graphics providers are not connected yet.
        </p>
        <div className="two-fields">
          {choices("renderer", "Graphics mode", [
            "Recommended",
            "Original graphics",
            "Modern DirectX",
            "Vulkan",
          ])}
          {choices("filtering", "Picture preference", [
            "Balanced",
            "Sharper textures",
            "Softer edges",
          ])}
        </div>
        <button
          className="secondary"
          onClick={() =>
            update({
              graphics: {
                resolution: "3840 × 2160",
                uiScale: 1.5,
                renderer: "Recommended",
                filtering: "Balanced",
                windowMode: "Borderless window",
              },
            })
          }
        >
          Restore display defaults
        </button>
      </details>
    </section>
  );
}

export function Settings({ preferences, change, reset }) {
  return (
    <section className="settings-surface" aria-labelledby="appearance-heading">
      <h2 id="appearance-heading">App appearance</h2>
      <RangeField
        field={{
          id: "launcher-text-size",
          label: "Launcher text size",
          min: 90,
          max: 125,
          step: 5,
          unit: "%",
        }}
        value={preferences.textScale}
        onChange={(textScale) => change({ textScale })}
      />
      <div className="setting-row">
        <h3>Reduce transparency</h3>
        <Toggle
          label="Reduce transparency"
          checked={preferences.opaque}
          onChange={(opaque) => change({ opaque })}
        />
      </div>
      <details className="advanced">
        <summary>Advanced app settings</summary>
        <div className="setting-row">
          <div>
            <h3>Check for updates</h3>
            <p className="helper">
              Preference only; no updates are downloaded.
            </p>
          </div>
          <Toggle
            label="Check for updates"
            checked={preferences.checkUpdates}
            onChange={(checkUpdates) => change({ checkUpdates })}
          />
        </div>
        <button className="secondary" onClick={reset}>
          Reset preview
        </button>
      </details>
    </section>
  );
}
