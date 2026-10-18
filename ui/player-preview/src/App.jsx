import React, { useEffect, useState } from "react";
import { cleanState, defaults, packs, STORE_KEY } from "./data.js";
import { Icon, Modal, ProfilePicker } from "./components.jsx";
import Home from "./Home.jsx";
import Profiles from "./Profiles.jsx";
import Library from "./Library.jsx";
import { Graphics, Settings } from "./Preferences.jsx";

const pages = [
  ["play", "Play"],
  ["profiles", "Profiles"],
  ["library", "Addons & Mods"],
  ["settings", "Settings"],
];

export default function App() {
  const [state, setState] = useState(() => {
    try {
      return cleanState(JSON.parse(localStorage.getItem(STORE_KEY)));
    } catch {
      return defaults();
    }
  });
  const [page, setPage] = useState("play");
  const [modal, setModal] = useState(null);
  const [stored, setStored] = useState(true);
  const [notice, setNotice] = useState("");
  const profile = state.profiles.find((p) => p.id === state.selected);
  useEffect(() => {
    try {
      localStorage.setItem(STORE_KEY, JSON.stringify(state));
      setStored(true);
    } catch {
      setStored(false);
    }
  }, [state]);
  useEffect(() => {
    document.documentElement.style.fontSize = `${(16 * state.preferences.textScale) / 100}px`;
    document.documentElement.dataset.opaque = String(state.preferences.opaque);
  }, [state.preferences]);
  const select = (selected) =>
    setState((previous) => ({ ...previous, selected }));
  const update = (changes) =>
    setState((previous) => ({
      ...previous,
      profiles: previous.profiles.map((p) =>
        p.id === previous.selected ? { ...p, ...changes } : p,
      ),
    }));
  const toggle = (id) =>
    update({
      enabled: profile.enabled.includes(id)
        ? profile.enabled.filter((v) => v !== id)
        : [...profile.enabled, id],
    });
  const navigate = (next) => {
    setPage(next);
    setNotice("");
    window.scrollTo({ top: 0, behavior: "instant" });
  };
  const create = (name) => {
    const trimmed = name.trim();
    if (state.profiles.length >= 30)
      return "This preview supports up to 30 profiles.";
    if (!trimmed || trimmed.length > 60)
      return "Choose a name between 1 and 60 characters.";
    if (
      state.profiles.some((p) => p.name.toLowerCase() === trimmed.toLowerCase())
    )
      return "That profile name is already in use.";
    const p = {
      ...structuredClone(profile),
      id: crypto.randomUUID(),
      name: trimmed,
    };
    setState((previous) => ({
      ...previous,
      profiles: [...previous.profiles, p],
      selected: p.id,
    }));
    return "";
  };
  const duplicate = () => {
    const base = profile.name.slice(0, 48);
    let name = base + " copy";
    let i = 2;
    while (
      state.profiles.some((p) => p.name.toLowerCase() === name.toLowerCase())
    )
      name = `${base} copy ${i++}`;
    const error = create(name);
    if (error) setNotice(error);
  };
  const remove = () =>
    setState((previous) => {
      if (previous.profiles.length <= 1) return previous;
      const profiles = previous.profiles.filter(
        (p) => p.id !== previous.selected,
      );
      return { ...previous, profiles, selected: profiles[0].id };
    });
  return (
    <div className="app-shell" data-page={page}>
      <a className="skip-link" href="#main-content">
        Skip to main content
      </a>
      <header className="app-header">
        <h1>Signet</h1>
        <button
          className="preview-label"
          onClick={() => setModal("about")}
          aria-label="About this preview"
        >
          Preview
        </button>
      </header>
      <div className="launch-toolbar">
        <ProfilePicker
          profiles={state.profiles}
          selected={state.selected}
          onChange={select}
        />
        <button
          className="primary play-button"
          onClick={() => setModal("launch")}
        >
          <Icon name="play" size={14} />
          Play FFXI
        </button>
      </div>
      <nav className="main-tabs" aria-label="Main navigation">
        {pages.map(([id, title]) => (
          <button
            className={"nav-item " + (page === id ? "active" : "")}
            key={id}
            aria-current={page === id ? "page" : undefined}
            onClick={() => navigate(id)}
          >
            {title}
          </button>
        ))}
      </nav>
      <main id="main-content">
        {notice && (
          <p className="quiet-note" role="status">
            {notice}
          </p>
        )}
        {page === "play" && (
          <Home
            profile={profile}
            navigate={(next) => {
              navigate(next === "graphics" ? "settings" : next);
            }}
            toggle={toggle}
            openLaunch={() => setModal("launch")}
          />
        )}
        {page === "profiles" && (
          <Profiles
            state={state}
            select={select}
            update={update}
            create={create}
            duplicate={duplicate}
            remove={remove}
          />
        )}
        {page === "library" && (
          <Library
            state={state}
            profile={profile}
            select={select}
            toggle={toggle}
            update={update}
          />
        )}
        {page === "settings" && (
          <Graphics
            state={state}
            profile={profile}
            select={select}
            update={update}
          />
        )}
        {page === "settings" && (
          <Settings
            preferences={state.preferences}
            change={(changes) =>
              setState((previous) => ({
                ...previous,
                preferences: { ...previous.preferences, ...changes },
              }))
            }
            reset={() => setModal("reset")}
          />
        )}
        <footer aria-live="polite">
          {stored
            ? "Preview · saved in this browser"
            : "Storage is unavailable. Changes last for this visit only."}
        </footer>
      </main>
      {modal === "about" && (
        <Modal title="Interface preview" close={() => setModal(null)}>
          <p>Sample profiles and settings. Changes stay in this browser.</p>
          <p>
            Game launch, mod installation, and graphics setup are not connected
            yet.
          </p>
          <div className="button-row">
            <button className="secondary" onClick={() => setModal("reset")}>
              Reset preview
            </button>
            <button className="primary" onClick={() => setModal(null)}>
              Done
            </button>
          </div>
        </Modal>
      )}
      {modal === "launch" && (
        <Modal title="Launch preview" close={() => setModal(null)}>
          <h3>{profile.name}</h3>
          <p>
            {profile.character} · {profile.server}
          </p>
          <dl className="launch-summary">
            <div>
              <dt>Display</dt>
              <dd>{profile.graphics.resolution}</dd>
            </div>
            <div>
              <dt>Interface size</dt>
              <dd>{profile.graphics.uiScale.toFixed(1)}×</dd>
            </div>
            <div>
              <dt>Enabled items</dt>
              <dd>{profile.enabled.length}</dd>
            </div>
          </dl>
          <p>
            {packs
              .filter((p) => profile.enabled.includes(p.id))
              .map((p) => p.name)
              .join(", ") || "No sample items enabled."}
          </p>
          <p className="quiet-note">
            FFXI will not start. Game launch is not connected yet.
          </p>
          <div className="button-row">
            <button className="primary" onClick={() => setModal(null)}>
              Close
            </button>
            <button
              className="secondary"
              onClick={() => {
                setModal(null);
                navigate("profiles");
              }}
            >
              Edit profile
            </button>
          </div>
        </Modal>
      )}
      {modal === "reset" && (
        <Modal title="Start fresh?" close={() => setModal(null)}>
          <p>Reset this preview’s profiles and settings?</p>
          <div className="button-row">
            <button
              className="primary"
              onClick={() => {
                setState(defaults());
                setPage("play");
                setModal(null);
                setNotice("");
              }}
            >
              Restore sample setup
            </button>
            <button className="secondary" onClick={() => setModal(null)}>
              Keep my changes
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
}
