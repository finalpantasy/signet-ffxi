import React from "react";
import { Toggle } from "./components.jsx";
import { packs } from "./data.js";

export default function Home({ profile, navigate, toggle }) {
  return (
    <>
      <div className="setup-line">
        <h2>{profile.character}</h2>
        <span>{profile.server}</span>
      </div>
      <button className="summary-row" onClick={() => navigate("graphics")}>
        <span>Display</span>
        <span>
          {profile.graphics.resolution} · {profile.graphics.uiScale.toFixed(1)}×
          UI
          <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true">
            <path
              d="m6 3 5 5-5 5"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
            />
          </svg>
        </span>
      </button>
      <div className="section-heading">
        <h2>Quick mods</h2>
        <button className="text-button" onClick={() => navigate("library")}>
          Manage all
        </button>
      </div>
      <div className="quick-mods">
        {packs.slice(0, 3).map((pack) => (
          <div className="mod-row" key={pack.id}>
            <span>{pack.name}</span>
            <Toggle
              label={"Enable " + pack.name}
              checked={profile.enabled.includes(pack.id)}
              onChange={() => toggle(pack.id)}
            />
          </div>
        ))}
      </div>
    </>
  );
}
