import React, { useEffect, useRef, useState } from "react";
import { Icon, ProfilePicker, Toggle } from "./components.jsx";
const pages = [
  ["play", "Play"],
  ["profiles", "Profiles"],
  ["library", "Addons & Mods"],
  ["settings", "Settings"],
];
async function rpc(action, fields = {}) {
  const response = await fetch("/api/v1/rpc", {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Signet-Request": "1" },
    body: JSON.stringify({ schema_version: 1, action, ...fields }),
    signal: AbortSignal.timeout(15000),
  });
  const reply = await response.json();
  if (!response.ok || !reply.ok)
    throw Error(reply.error || "Native request failed");
  return reply.value;
}
export default function NativeApp() {
  const [snapshot, setSnapshot] = useState(null),
    [selected, setSelected] = useState(""),
    [page, setPage] = useState("play"),
    [error, setError] = useState(""),
    [notice, setNotice] = useState(""),
    [busy, setBusy] = useState(false),
    [drafts, setDrafts] = useState({}),
    [connected, setConnected] = useState(false),
    [copyName, setCopyName] = useState(""),
    [packages, setPackages] = useState(null),
    [preferences, setPreferences] = useState({ textScale: 100, opaque: false });
  const pendingOperations = useRef(0);
  const profile =
    snapshot?.profiles.find((p) => p.document.name === selected) ||
    snapshot?.profiles[0];
  const session = snapshot?.session;
  const draft = profile
    ? Object.hasOwn(drafts, profile.document.name)
      ? drafts[profile.document.name]
      : profile.document
    : null;
  const enabledNames =
    profile?.document.profile.addons.map((addon) => addon.name) || [];
  const setDraft = (value) =>
    setDrafts((old) => ({ ...old, [profile.document.name]: value }));
  const forgetDraft = (name, submittedDraft) =>
    setDrafts((old) => {
      // Completion must not erase edits made while the request was pending.
      if (old[name] !== submittedDraft) return old;
      const next = { ...old };
      delete next[name];
      return next;
    });
  const invalidateConnection = () => {
    setConnected(false);
    setPackages(null);
    setSnapshot((old) => (old ? { ...old, session: null } : old));
  };
  const refresh = async (signal) => {
    const deadline = AbortSignal.timeout(15000);
    const response = await fetch("/api/v1/snapshot", {
      signal: signal ? AbortSignal.any([signal, deadline]) : deadline,
    });
    if (!response.ok)
      throw Error("Native workspace is unavailable. Reopen its owned server.");
    const value = await response.json();
    if (value.schema_version !== 1) throw Error("Unsupported native contract");
    if (signal?.aborted) return;
    setSnapshot(value);
    setConnected(true);
  };
  useEffect(() => {
    refresh().catch((e) => setError(e.message));
  }, []);
  useEffect(() => {
    if (profile) setSelected(profile.document.name);
  }, [profile]);
  useEffect(() => {
    document.documentElement.style.fontSize = `${(16 * preferences.textScale) / 100}px`;
    document.documentElement.dataset.opaque = String(preferences.opaque);
  }, [preferences]);
  useEffect(() => {
    if (!session || busy) return;
    const controller = new AbortController();
    let refreshing = false;
    const timer = setInterval(() => {
      if (refreshing) return;
      refreshing = true;
      refresh(controller.signal)
        .catch((e) => {
          if (!controller.signal.aborted) {
            invalidateConnection();
            setError(e.message);
          }
        })
        .finally(() => {
          refreshing = false;
        });
    }, 3000);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  }, [session?.process_id, busy]);
  const run = async (operation) => {
    pendingOperations.current += 1;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await operation();
    } catch (e) {
      setError(e.message);
      // A mutation may have succeeded before its response was lost.
      // Read authoritative state before offering another action.
      try {
        await refresh();
      } catch {
        invalidateConnection();
      }
    } finally {
      pendingOperations.current -= 1;
      setBusy(pendingOperations.current > 0);
    }
  };
  const save = async (enabled = enabledNames, addonOnly = false) => {
    const display = addonOnly ? profile.document : draft;
    const value = await rpc("profile", {
      request: {
        schema_version: 1,
        action: "save_profile",
        id: profile.id,
        name: profile.document.name,
        width: Number(display.width),
        height: Number(display.height),
        ui_scale: Number(display.ui_scale),
        enabled,
      },
    });
    setSnapshot((old) => ({
      ...value,
      session: old.session,
      offline_host_available: old.offline_host_available,
    }));
    if (!addonOnly) forgetDraft(profile.document.name, draft);
    setNotice("Saved to native profile");
  };
  const controls = async (operation, name) => {
    const value = await rpc("runtime", {
      operation,
      ...(name ? { name } : {}),
    });
    setSnapshot((old) => ({ ...old, session: value }));
  };
  const navigate = (next) => {
    setPage(next);
    setNotice("");
    setError("");
    if (next === "library") {
      setPackages(null);
      run(async () => setPackages(await rpc("packages")));
    }
  };
  if (!snapshot)
    return (
      <div className="app-shell">
        <header className="app-header">
          <h1>Signet</h1>
        </header>
        <main>
          <p role={error ? "alert" : "status"}>
            {error || "Opening native workspace…"}
          </p>
          <button className="secondary" onClick={() => run(refresh)}>
            Retry connection
          </button>
        </main>
      </div>
    );
  const loaded = session?.reply?.status?.loaded_sources || {};
  return (
    <div className="app-shell native-player" data-page={page}>
      <a className="skip-link" href="#main-content">
        Skip to main content
      </a>
      <header className="app-header">
        <h1>Signet</h1>
        <span className="preview-label">Native workspace</span>
      </header>
      <div
        className="launch-toolbar"
        role="region"
        aria-label="Profile and launch"
      >
        <ProfilePicker
          profiles={snapshot.profiles.map((p) => ({
            id: p.document.name,
            name: p.document.name,
          }))}
          selected={profile?.document.name || ""}
          onChange={(id) => {
            setSelected(id);
            setNotice("");
          }}
        />
        <button
          className="primary play-button"
          disabled
          title={snapshot.game_launch.reason}
        >
          <Icon name="play" size={14} />
          Play FFXI
        </button>
      </div>
      <nav className="main-tabs" aria-label="Main navigation">
        {pages.map(([id, title]) => (
          <button
            key={id}
            className={`nav-item ${page === id ? "active" : ""}`}
            aria-current={page === id ? "page" : undefined}
            onClick={() => navigate(id)}
          >
            {title}
          </button>
        ))}
      </nav>
      <main id="main-content" aria-busy={busy}>
        {error && (
          <p className="quiet-note" role="alert">
            {error}
            <button className="text-button" onClick={() => run(refresh)}>
              Refresh
            </button>
          </p>
        )}
        {!connected && <p role="status">Runtime status unavailable</p>}
        {snapshot.warnings?.map((warning) => (
          <p className="quiet-note" role="alert" key={warning}>
            {warning}
          </p>
        ))}
        {notice && (
          <p className="quiet-note" role="status">
            {notice}
          </p>
        )}
        {!profile && (
          <p>
            No native profiles found. Import an existing launcher document into
            this workspace’s profiles folder.
          </p>
        )}
        {profile && page === "play" && (
          <>
            <h2>{profile.document.name}</h2>
            <div className="summary-row">
              <span>Saved display</span>
              <span>
                {profile.document.width} × {profile.document.height} ·{" "}
                {profile.document.ui_scale}x
              </span>
            </div>
            <div className="summary-row">
              <span>Selected addons</span>
              <span>{profile.document.profile.addons.length}</span>
            </div>
            <p className="quiet-note">
              Game launch needs a verified isolated client.
            </p>
            <details className="advanced">
              <summary>Advanced</summary>
              <p className="quiet-note">{snapshot.game_launch.reason}</p>
              <p className="quiet-note">
                Test selected addons in the real Lua runtime. Commands are
                captured; FFXI does not start.
              </p>
              <div className="button-row">
                <button
                  className="secondary"
                  disabled={
                    busy ||
                    !connected ||
                    !!session ||
                    !snapshot.offline_host_available ||
                    profile.requires_restore
                  }
                  onClick={() =>
                    run(async () => {
                      const value = await rpc("start_offline", {
                        id: profile.id,
                      });
                      setSnapshot((old) => ({ ...old, session: value }));
                    })
                  }
                >
                  Test addons
                </button>
                {session && (
                  <>
                    <button
                      className="secondary"
                      disabled={busy || !connected}
                      onClick={() => run(() => controls("status"))}
                    >
                      Refresh runtime
                    </button>
                    <button
                      className="secondary"
                      disabled={busy || !connected}
                      onClick={() =>
                        run(async () => {
                          await rpc("stop_runtime");
                          setSnapshot((old) => ({ ...old, session: null }));
                          setNotice("Offline test stopped");
                        })
                      }
                    >
                      Stop test
                    </button>
                  </>
                )}
              </div>
              {session && (
                <>
                  <p role="status">Running · offline addon test</p>
                  {session.profile_id !== profile.id && (
                    <p className="quiet-note">
                      Saved changes apply to the next session.
                    </p>
                  )}
                  {session.document.profile.addons.map((addon) => (
                    <div className="setting-row" key={addon.name}>
                      <div>
                        <h3>{addon.name}</h3>
                        <p>
                          {Object.hasOwn(loaded, addon.name.toLowerCase())
                            ? "Loaded in offline runtime"
                            : "Not loaded"}
                        </p>
                      </div>
                      <div className="button-row">
                        {Object.hasOwn(loaded, addon.name.toLowerCase()) ? (
                          <>
                            <button
                              className="secondary"
                              disabled={busy || !connected}
                              onClick={() =>
                                run(() => controls("reload", addon.name))
                              }
                            >
                              Reload {addon.name}
                            </button>
                            <button
                              className="secondary"
                              disabled={busy || !connected}
                              onClick={() =>
                                run(() => controls("unload", addon.name))
                              }
                            >
                              Unload {addon.name}
                            </button>
                          </>
                        ) : (
                          <button
                            className="secondary"
                            disabled={busy || !connected}
                            onClick={() =>
                              run(() => controls("load", addon.name))
                            }
                          >
                            Load {addon.name}
                          </button>
                        )}
                      </div>
                    </div>
                  ))}
                  {session.reply.autoload_errors?.length > 0 && (
                    <p role="alert">
                      {JSON.stringify(session.reply.autoload_errors)}
                    </p>
                  )}
                  {session.reply.error && (
                    <p role="alert">{session.reply.error}</p>
                  )}
                </>
              )}
            </details>
          </>
        )}
        {profile && draft && page === "profiles" && (
          <>
            <h2>{profile.document.name}</h2>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                run(() => save());
              }}
            >
              <div className="paired-fields">
                <label>
                  Width
                  <input
                    required
                    type="number"
                    min="640"
                    max="16384"
                    value={draft.width}
                    onChange={(e) =>
                      setDraft({ ...draft, width: e.target.value })
                    }
                  />
                </label>
                <label>
                  Height
                  <input
                    required
                    type="number"
                    min="480"
                    max="16384"
                    value={draft.height}
                    onChange={(e) =>
                      setDraft({ ...draft, height: e.target.value })
                    }
                  />
                </label>
                <label>
                  Game UI scale
                  <input
                    required
                    type="number"
                    min="0.5"
                    max="4"
                    step="0.05"
                    value={draft.ui_scale}
                    onChange={(e) =>
                      setDraft({ ...draft, ui_scale: e.target.value })
                    }
                  />
                </label>
              </div>
              <button className="primary" disabled={busy || !connected}>
                Save profile
              </button>
            </form>
            <p className="quiet-note">
              Saved display preferences are requested values. Active game
              settings are not verified.
            </p>
            <div className="button-row">
              <label>
                Copy name
                <input
                  value={copyName}
                  maxLength={64}
                  pattern="[A-Za-z0-9_-]+"
                  onChange={(e) => setCopyName(e.target.value)}
                />
              </label>
              <button
                className="secondary"
                disabled={busy || !connected || !copyName.trim()}
                onClick={() =>
                  run(async () => {
                    const value = await rpc("profile", {
                      request: {
                        schema_version: 1,
                        action: "copy_profile",
                        id: profile.id,
                        name: copyName,
                      },
                    });
                    setSnapshot((old) => ({
                      ...value,
                      session: old.session,
                      offline_host_available: old.offline_host_available,
                    }));
                    setSelected((current) =>
                      current === profile.document.name ? copyName : current,
                    );
                    setNotice("Profile copied");
                    setCopyName((current) =>
                      current === copyName ? "" : current,
                    );
                  })
                }
              >
                Copy profile
              </button>
            </div>
            <details className="advanced">
              <summary>Advanced</summary>
              <p className="quiet-note">
                Names use letters, numbers, hyphens and underscores. Restore
                creates a new native version.
              </p>
              {profile.requires_restore && (
                <p role="alert">
                  A newer saved version is unreadable. Restore a version below
                  before testing addons.
                </p>
              )}
              {profile.history
                .filter(
                  (_, index) =>
                    profile.requires_restore ||
                    index < profile.history.length - 1,
                )
                .reverse()
                .map((version) => (
                  <div className="setting-row" key={version}>
                    <span>{version}</span>
                    <button
                      className="secondary"
                      disabled={busy || !connected}
                      onClick={() =>
                        run(async () => {
                          const value = await rpc("profile", {
                            request: {
                              schema_version: 1,
                              action: "restore_profile",
                              id: profile.id,
                              version,
                            },
                          });
                          setSnapshot((old) => ({
                            ...value,
                            session: old.session,
                            offline_host_available: old.offline_host_available,
                          }));
                          forgetDraft(profile.document.name, draft);
                          setNotice("Profile restored");
                        })
                      }
                    >
                      Restore {version}
                    </button>
                  </div>
                ))}
            </details>
          </>
        )}
        {profile && page === "library" && (
          <>
            <h2>Addons & Mods</h2>
            {profile.available_addons.map((addon) => (
              <div className="setting-row" key={addon.name}>
                <div>
                  <h3>{addon.name}</h3>
                  <p className="quiet-note">
                    {addon.ecosystem} · partial compatibility · {addon.engine}
                  </p>
                </div>
                <Toggle
                  label={`Enable ${addon.name}`}
                  checked={profile.document.profile.addons.some(
                    (a) => a.name === addon.name,
                  )}
                  disabled={busy || !connected}
                  onChange={(enabled) =>
                    run(() =>
                      save(
                        enabled
                          ? [...enabledNames, addon.name]
                          : enabledNames.filter((n) => n !== addon.name),
                        true,
                      ),
                    )
                  }
                />
              </div>
            ))}
            <p className="quiet-note">
              Selection is saved for the next session. Native plugins, visual
              folders and generated addon settings are unavailable.
            </p>
            <details className="advanced">
              <summary>Managed packages</summary>
              {packages ? (
                <>
                  <p>Active generation: {packages.current?.number ?? "none"}</p>
                  {Object.entries(packages.current?.slots || {}).map(
                    ([name, slot]) => (
                      <div className="setting-row" key={name}>
                        <span>{name}</span>
                        <span>{slot}</span>
                      </div>
                    ),
                  )}
                  {!packages.current && (
                    <p>No active packages in this workspace.</p>
                  )}
                </>
              ) : (
                <p>Package store is unavailable.</p>
              )}
            </details>
          </>
        )}
        {page === "settings" && (
          <>
            <h2>Settings</h2>
            <div className="setting-row">
              <label htmlFor="native-text-size">Text size</label>
              <select
                id="native-text-size"
                value={preferences.textScale}
                onChange={(e) =>
                  setPreferences({
                    ...preferences,
                    textScale: Number(e.target.value),
                  })
                }
              >
                {[100, 125, 150].map((v) => (
                  <option key={v} value={v}>
                    {v}%
                  </option>
                ))}
              </select>
            </div>
            <div className="setting-row">
              <span>Opaque background</span>
              <Toggle
                label="Opaque background"
                checked={preferences.opaque}
                onChange={(opaque) =>
                  setPreferences({ ...preferences, opaque })
                }
              />
            </div>
            <p className="quiet-note">
              Graphics-provider setup and game pacing measurements are
              unavailable.
            </p>
          </>
        )}
        <footer>
          Native profiles · immutable versions ·{" "}
          {!connected
            ? "runtime status unavailable"
            : session
              ? "offline runtime active"
              : "no owned runtime"}
        </footer>
      </main>
    </div>
  );
}
