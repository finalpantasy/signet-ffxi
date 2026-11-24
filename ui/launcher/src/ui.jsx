import React from "react";

export function Seg({ value, options, onChange, label }) {
  return (
    <div className="seg" role="group" aria-label={label}>
      {options.map(([v, text]) => (
        <button key={v} type="button" aria-pressed={value === v} onClick={() => onChange(v)}>
          {text}
        </button>
      ))}
    </div>
  );
}

export const speedText = (v) => (v > 0 ? `+${v}%` : `${v}%`);

export function Stepper({ value, min, max, onChange, label }) {
  const step = (d, e) => onChange(Math.max(min, Math.min(max, value + d * (e.shiftKey ? 10 : 2))));
  return (
    <div className="stepper" role="group" aria-label={label}>
      <button type="button" aria-label="Slower" onClick={(e) => step(-1, e)} disabled={value <= min}>
        −
      </button>
      <output aria-live="polite">{speedText(value)}</output>
      <button type="button" aria-label="Faster" onClick={(e) => step(1, e)} disabled={value >= max}>
        +
      </button>
    </div>
  );
}

export function ZoneArt({ art, className, alt = "" }) {
  if (!art) return <div className={`fill ${className || ""}`} aria-hidden="true" />;
  return <img className={className} src={`./zones/${art.file}`} alt={alt} draggable="false" />;
}

export function Dot({ state }) {
  return <span className={`dot ${state || ""}`} aria-hidden="true" />;
}

/** Retail client state for chips and the sidebar: dot style plus short text. The update
 * state comes from the server version check, not from file dates. */
export function clientSummary(r, opening, upd) {
  if (opening) return { dot: "starting", short: "Opening PlayOnline", state: "opening" };
  if (!r) return { dot: "", short: "Checking", state: "checking" };
  if (r.pid != null) return { dot: "ready", short: "PlayOnline running", state: "running" };
  if (upd?.job?.phase === "applying" || upd?.restore?.phase === "running") return { dot: "starting", short: "Updating", state: "updating" };
  const c = upd?.check?.status;
  if (c === "available") return { dot: "warn", short: "Update available", state: "outdated", server: upd.check.server };
  if (c === "current") return { dot: "ok", short: "Up to date", state: "current" };
  if (c === "offline") return { dot: "", short: "Update server unreachable", state: "offline" };
  if (!r.viewer?.found) return { dot: "exited", short: "PlayOnline not found", state: "missing" };
  return { dot: "", short: "Checking for updates", state: "checking" };
}

// No-break spaces keep the game title on one line.
export const UPDATE_STEPS = "Update through PlayOnline: Play, then FINAL FANTASY XI, then Play.";

export function fmtBytes(n) {
  if (!n) return "0 B";
  return n >= 1048576 ? `${(n / 1048576).toFixed(2)} MB` : `${(n / 1024).toFixed(1)} KB`;
}

export function fmtDate(secs) {
  if (!secs) return "Unknown";
  return new Date(secs * 1000).toLocaleString("en-US", { month: "short", day: "numeric", year: "numeric", hour: "numeric", minute: "2-digit" });
}

/** "2019-04-30" as "Apr 30, 2019". */
export function fmtDay(iso) {
  if (!iso) return "Unknown";
  const [y, m, d] = iso.split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" });
}
