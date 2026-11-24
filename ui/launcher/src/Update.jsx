import React, { useState } from "react";
import * as Icon from "./icons.jsx";
import { Dot } from "./ui.jsx";

export const CLOSE_FIRST = "Close PlayOnline and FFXI first.";

const mb = (n) => {
  const v = (n || 0) / 1048576;
  return v >= 10 ? Math.round(v).toLocaleString("en-US") : v.toFixed(1);
};
const pct = (done, total) => (total > 0 ? Math.min(100, Math.floor((done / total) * 100)) : 0);

/** "20261124-101010" (UTC) as "Nov 24, 2026, 10:10 UTC". */
function backupLabel(name) {
  const m = /^(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})\d{2}/.exec(name || "");
  if (!m) return name || "";
  const d = new Date(Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5]));
  return `${d.toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric", timeZone: "UTC" })}, ${m[4]}:${m[5]} UTC`;
}

export const isBlocked = (upd, retail) => (upd.blocked?.length ?? 0) > 0 || retail?.pid != null;
export const isUpdating = (upd) => upd.job.phase === "planning" || upd.job.phase === "applying" || upd.restore.phase === "running";

function Bar({ value, label }) {
  return (
    <div className="bar" role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={value}>
      <i style={{ width: `${value}%` }} />
    </div>
  );
}

function Row({ label, value, note }) {
  return (
    <div className="prog-row">
      <div className="prog-text">
        <span>{label}</span>
        <span className="mono">{note}</span>
      </div>
      <Bar value={value} label={label} />
    </div>
  );
}

function Headline({ upd }) {
  const c = upd.check;
  if (c.status === "current") {
    return (
      <div className="upd-head">
        <Icon.Check className="ok" />
        <div>
          <h3>Up to date ({c.installed ?? c.server})</h3>
          <p>This install matches the update server.</p>
        </div>
      </div>
    );
  }
  if (c.status === "available") {
    return (
      <div className="upd-head">
        <Icon.Alert />
        <div>
          <h3>Update available</h3>
          <p>
            Installed <span className="mono">{c.installed ?? "unknown"}</span>. Latest <span className="mono">{c.server}</span>.
          </p>
        </div>
      </div>
    );
  }
  if (c.status === "offline") {
    return (
      <div className="upd-head">
        <Icon.Alert />
        <div>
          <h3>Couldn't reach the update server</h3>
          <p>
            Installed version <span className="mono">{c.installed ?? "unknown"}</span>.
          </p>
        </div>
      </div>
    );
  }
  return (
    <div className="upd-head">
      <span className="spinner small" aria-hidden="true" />
      <div>
        <h3>Checking for updates</h3>
        <p>
          Installed version <span className="mono">{c.installed ?? "unknown"}</span>.
        </p>
      </div>
    </div>
  );
}

function Restore({ upd, blocked, act }) {
  const [ask, setAsk] = useState(false);
  const r = upd.restore;
  const busy = upd.job.phase === "planning" || upd.job.phase === "applying";
  if (r.phase === "running") {
    return (
      <div className="upd-block">
        <Row label="Restoring previous files" value={pct(r.done, r.total)} note={`${r.done.toLocaleString("en-US")} of ${r.total.toLocaleString("en-US")}`} />
      </div>
    );
  }
  if (r.phase === "done") {
    return (
      <div className="upd-block">
        <p className="upd-line">
          <Icon.Check className="ok" />
          <span>
            Restored {r.restored.toLocaleString("en-US")} files{r.removed ? `, removed ${r.removed.toLocaleString("en-US")} added by the update` : ""}.
            {r.partial ? " The earlier update had stopped part way." : ""}
          </span>
        </p>
        <div className="btn-row">
          <button type="button" className="btn" onClick={act.dismiss}>
            Done
          </button>
        </div>
      </div>
    );
  }
  if (r.phase === "failed") {
    return (
      <div className="upd-block">
        <p className="upd-line err selectable">
          <Icon.Alert />
          {r.error}
        </p>
        <div className="btn-row">
          <button type="button" className="btn" onClick={act.dismiss}>
            Close
          </button>
        </div>
      </div>
    );
  }
  if (!upd.backup || busy) return null;
  return (
    <div className="upd-block restore">
      {ask ? (
        <>
          <p className="upd-line">Put back the files replaced on {backupLabel(upd.backup.name)}? Files the update added are removed.</p>
          <div className="btn-row">
            <button
              type="button"
              className="btn"
              disabled={blocked}
              onClick={() => {
                setAsk(false);
                act.restore();
              }}
            >
              Restore files
            </button>
            <button type="button" className="btn quiet" onClick={() => setAsk(false)}>
              Keep current files
            </button>
          </div>
        </>
      ) : (
        <div className="upd-line spread">
          <span>Backup from {backupLabel(upd.backup.name)}</span>
          <button type="button" className="btn quiet" disabled={blocked} onClick={() => setAsk(true)}>
            <Icon.Refresh />
            Restore previous files
          </button>
        </div>
      )}
    </div>
  );
}

export function UpdatePanel({ upd, retail, act, open }) {
  const blocked = isBlocked(upd, retail);
  const { check: c, job: j } = upd;
  const p = j.progress;
  const busyCheck = c.status === "checking";

  let body = null;
  if (j.phase === "planning") {
    body = (
      <div className="upd-block">
        {p.stage === "scan" ? (
          <Row label="Checking files" value={pct(p.scanned, p.scan_total)} note={`${p.scanned.toLocaleString("en-US")} of ${p.scan_total.toLocaleString("en-US")}`} />
        ) : (
          <p className="upd-line">
            <span className="spinner small" aria-hidden="true" />
            Getting the file list
          </p>
        )}
        <div className="btn-row">
          <button type="button" className="btn" onClick={act.cancel} disabled={j.cancelling}>
            {j.cancelling ? "Canceling" : "Cancel"}
          </button>
        </div>
      </div>
    );
  } else if (j.phase === "planned") {
    const plan = j.plan;
    body = plan.current ? (
      <div className="upd-block">
        <p className="upd-line">Nothing to download. Every file matches the server.</p>
        <div className="btn-row">
          <button type="button" className="btn" onClick={act.dismiss}>
            Done
          </button>
        </div>
      </div>
    ) : (
      <div className="upd-block">
        <p className="upd-line">
          <span>
            <b>{plan.files.toLocaleString("en-US")} files</b>, {mb(plan.bytes)} MB to download. Replaced files are backed up first.
          </span>
        </p>
        <div className="btn-row">
          <button type="button" className="btn gold" onClick={act.apply} disabled={blocked}>
            Download and install ({mb(plan.bytes)} MB)
          </button>
          <button type="button" className="btn quiet" onClick={act.dismiss}>
            Cancel
          </button>
        </div>
        {blocked && <p className="upd-note">{CLOSE_FIRST}</p>}
      </div>
    );
  } else if (j.phase === "applying") {
    const installing = p.stage === "install";
    body = (
      <div className="upd-block">
        {(p.stage === "connect" || p.stage === "scan") && (
          <Row
            label={p.stage === "scan" ? "Checking files" : "Connecting"}
            value={p.stage === "scan" ? pct(p.scanned, p.scan_total) : 0}
            note={p.stage === "scan" ? `${p.scanned.toLocaleString("en-US")} of ${p.scan_total.toLocaleString("en-US")}` : ""}
          />
        )}
        {(p.stage === "download" || installing) && (
          <>
            <Row label="Downloaded" value={pct(p.done_bytes, p.total_bytes)} note={`${mb(p.done_bytes)} of ${mb(p.total_bytes)} MB`} />
            <Row label="Verified" value={pct(p.verified, p.verify_total)} note={`${p.verified.toLocaleString("en-US")} of ${p.verify_total.toLocaleString("en-US")} files`} />
            <Row label="Installed" value={pct(p.installed, p.install_total)} note={installing ? `${p.installed.toLocaleString("en-US")} of ${p.install_total.toLocaleString("en-US")} files` : "Waiting"} />
          </>
        )}
        <div className="btn-row">
          <button type="button" className="btn" onClick={act.cancel} disabled={installing || j.cancelling}>
            {j.cancelling ? "Canceling" : "Cancel"}
          </button>
          {installing && <span className="upd-note inline">Installing. This step can't be canceled.</span>}
        </div>
      </div>
    );
  } else if (j.phase === "done") {
    const r = j.result;
    body = (
      <div className="upd-block">
        <p className="upd-line">
          <Icon.Check className="ok" />
          {r.installed === 0 ? "Already up to date. Nothing was changed." : `Updated to ${r.version}. ${r.installed.toLocaleString("en-US")} files installed, ${mb(r.downloaded_bytes)} MB downloaded.`}
        </p>
        {r.backup && (
          <p className="upd-note">
            Backup of the replaced files: <span className="mono selectable">{r.backup}</span>
          </p>
        )}
        <div className="btn-row">
          <button type="button" className="btn" onClick={act.dismiss}>
            Done
          </button>
          {r.backup && (
            <button type="button" className="btn quiet" onClick={() => open("backups")}>
              <Icon.Folder />
              Open backups
            </button>
          )}
        </div>
      </div>
    );
  } else if (j.phase === "failed") {
    body = (
      <div className="upd-block">
        <p className="upd-line err selectable">
          <Icon.Alert />
          {j.error}
        </p>
        <div className="btn-row">
          <button
            type="button"
            className="btn"
            onClick={() => {
              act.dismiss();
              act.check();
            }}
          >
            Close
          </button>
        </div>
      </div>
    );
  } else if (j.phase === "cancelled") {
    body = (
      <div className="upd-block">
        <p className="upd-line">Canceled. Nothing in the install changed.</p>
        <div className="btn-row">
          <button type="button" className="btn" onClick={act.dismiss}>
            Done
          </button>
        </div>
      </div>
    );
  } else if (upd.restore.phase === "idle") {
    body = (
      <div className="upd-block">
        <div className="btn-row">
          {c.status === "available" && (
            <button type="button" className="btn gold" onClick={act.plan} disabled={blocked}>
              <Icon.Refresh />
              Update client
            </button>
          )}
          <button type="button" className="btn quiet" onClick={act.check} disabled={busyCheck}>
            {c.status === "offline" ? "Try again" : "Check again"}
          </button>
          {blocked && c.status === "available" && <span className="upd-note inline">{CLOSE_FIRST}</span>}
        </div>
      </div>
    );
  }

  return (
    <section className="panel update" aria-label="Client updates" aria-busy={isUpdating(upd)}>
      <Headline upd={upd} />
      {body}
      <Restore upd={upd} blocked={blocked} act={act} />
    </section>
  );
}

/** Compact prompt on Home, shown only while an update is waiting or running. */
export function UpdatePrompt({ upd, retail, act, go }) {
  const blocked = isBlocked(upd, retail);
  const { check: c, job: j } = upd;
  if (isUpdating(upd)) {
    const p = j.progress;
    const note = j.phase === "applying" && p.stage === "download" ? ` ${pct(p.done_bytes, p.total_bytes)}%` : "";
    return (
      <div className="play-note">
        <Dot state="starting" />
        <span>
          Updating the client{note}.{" "}
          <button type="button" className="link" onClick={() => go("retail")}>
            View
          </button>
        </span>
      </div>
    );
  }
  if (c.status !== "available") return null;
  return (
    <div className="upd-prompt">
      <p className="play-note">
        <Icon.Alert />
        Update available ({c.server}). {blocked ? CLOSE_FIRST : ""}
      </p>
      <button
        type="button"
        className="btn glass-dark"
        disabled={blocked}
        onClick={() => {
          go("retail");
          act.plan();
        }}
      >
        Update client
      </button>
    </div>
  );
}
