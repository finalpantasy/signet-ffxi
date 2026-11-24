import React from "react";
import { call } from "./bridge.js";
import * as Icon from "./icons.jsx";
import { Dot, fmtBytes, fmtDate, fmtDay } from "./ui.jsx";
import { UpdatePanel, isUpdating } from "./Update.jsx";

function Banner({ info, client }) {
  if (client.state === "running") {
    return (
      <div className="panel banner" role="status">
        <Dot state="ready" />
        <div>
          <h2>PlayOnline is running</h2>
          <p>Process {info.pid}. Play opens again once it closes.</p>
        </div>
      </div>
    );
  }
  if (!info.viewer.found) {
    return (
      <div className="panel banner warn" role="status">
        <Icon.Alert />
        <div>
          <h2>PlayOnline not found</h2>
          <p className="selectable">No pol.exe in {info.viewer.path || "the PlayOnline folder"}. Reinstall PlayOnline or FINAL FANTASY XI.</p>
        </div>
      </div>
    );
  }
  return null;
}

export default function Retail({ retail: info, client, busy, playRetail, heroArt, notify, upd, act }) {
  if (!info) {
    return (
      <>
        <div className="head">
          <h1>Retail client</h1>
        </div>
        <div className="panel soon" aria-busy="true">
          <div>
            <span className="spinner" aria-hidden="true" />
            <p>Reading the install</p>
          </div>
        </div>
      </>
    );
  }

  const running = client.state === "running";
  const noPol = !info.viewer.found;
  const copy = (text) =>
    navigator.clipboard
      ?.writeText(text)
      .then(() => notify("SHA-256 copied"))
      .catch(() => notify("Copy failed", true));
  const open = (target) => call("open", { target }).catch((e) => notify(e.message, true));

  return (
    <>
      <div className="head">
        <h1>Retail client</h1>
      </div>

      <div className="rhero">
        {heroArt ? <img src={`./zones/${heroArt.file}`} alt="" draggable="false" /> : null}
        <div className="rhero-body">
          <div>
            <div className="eyebrow">Retail · PlayOnline</div>
            <h2>FINAL FANTASY XI</h2>
            <p>{running ? `PlayOnline running · pid ${info.pid}` : "Opens PlayOnline. Windows asks for approval, then you sign in as usual."}</p>
          </div>
          <button type="button" className="btn gold big" onClick={playRetail} disabled={running || busy === "retail" || noPol || isUpdating(upd)}>
            {running ? <Dot state="ready" /> : <Icon.Play />}
            {busy === "retail" ? "Opening PlayOnline" : running ? "PlayOnline running" : "Play FINAL FANTASY XI"}
          </button>
        </div>
      </div>

      <Banner info={info} client={client} />
      <UpdatePanel upd={upd} retail={info} act={act} open={open} />

      <div className="cols">
        <section className="panel">
          <div className="panel-head">
            <h2>FINAL FANTASY XI</h2>
            <span className="grow" />
            <button type="button" className="btn quiet" onClick={() => open("install")}>
              <Icon.Folder />
              Open folder
            </button>
          </div>
          <dl className="kv">
            <dt>Version</dt>
            <dd className="mono selectable">{info.installed ?? "Unknown"}</dd>
            <dt>Build date</dt>
            <dd>{fmtDay(info.installedDate)}</dd>
            <dt>FFXiMain.dll</dt>
            <dd>{info.dll.found ? `${fmtBytes(info.dll.size)} · ${fmtDate(info.dll.modified)}` : "Missing"}</dd>
            <dt>SHA-256</dt>
            <dd className="hash">
              <span className="mono selectable" title={info.dll.sha256}>
                {info.dll.sha256 ? `${info.dll.sha256.slice(0, 16)}…${info.dll.sha256.slice(-8)}` : "Unknown"}
              </span>
              {info.dll.sha256 && (
                <button type="button" className="btn quiet icon" aria-label="Copy SHA-256" title="Copy SHA-256" onClick={() => copy(info.dll.sha256)}>
                  <Icon.Copy />
                </button>
              )}
            </dd>
          </dl>
        </section>
        <section className="panel">
          <div className="panel-head">
            <h2>PlayOnline</h2>
            <span className="grow" />
            <button type="button" className="btn quiet" onClick={() => open("viewer")} disabled={!info.viewer.found}>
              <Icon.Folder />
              Open folder
            </button>
          </div>
          <dl className="kv">
            <dt>Version</dt>
            <dd className="mono selectable">{info.viewer.version ?? "Unknown"}</dd>
            <dt>Last update</dt>
            <dd>{fmtDay(info.viewer.date)}</dd>
            <dt>pol.exe</dt>
            <dd>{info.viewer.found ? "Found" : "Missing"}</dd>
            <dt>State</dt>
            <dd className="state-line">
              <Dot state={running ? "ready" : ""} />
              {running ? `Running · pid ${info.pid}` : "Closed"}
            </dd>
          </dl>
        </section>
      </div>
    </>
  );
}
