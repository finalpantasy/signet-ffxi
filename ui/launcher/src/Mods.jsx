import React from "react";
import * as Icon from "./icons.jsx";

export default function Mods() {
  return (
    <>
      <div className="head">
        <h1>Mods</h1>
        <span className="chip">Coming soon</span>
      </div>
      <div className="panel soon">
        <div>
          <Icon.Blocks />
          <h2>Mods live in the 32-bit runtime</h2>
          <p>They move here once the 64-bit client can load them.</p>
        </div>
      </div>
    </>
  );
}
