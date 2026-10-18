import React, { useState } from "react";
import { Icon, Modal } from "./components.jsx";

function Editor({ profile, save, duplicate, remove, canRemove }) {
  const [draft, setDraft] = useState({
    name: profile.name,
    character: profile.character,
    server: profile.server,
    description: profile.description,
  });
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  const dirty = Object.keys(draft).some((key) => draft[key] !== profile[key]);
  const update = (key, value) => {
    setDraft({ ...draft, [key]: value });
    setSaved(false);
  };
  return (
    <section className="glass profile-editor">
      <h2>Profile details</h2>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const message = save(draft);
          setError(message);
          setSaved(!message);
        }}
      >
        <div className="field">
          <label htmlFor="profile-name">Profile name</label>
          <input
            id="profile-name"
            maxLength={60}
            required
            value={draft.name}
            onChange={(e) => update("name", e.target.value)}
          />
        </div>
        <div className="two-fields">
          <div className="field">
            <label htmlFor="character">Character</label>
            <input
              id="character"
              maxLength={60}
              required
              value={draft.character}
              onChange={(e) => update("character", e.target.value)}
            />
          </div>
          <div className="field">
            <label htmlFor="server">Server</label>
            <input
              id="server"
              maxLength={60}
              required
              value={draft.server}
              onChange={(e) => update("server", e.target.value)}
            />
          </div>
        </div>
        <div className="field">
          <label htmlFor="profile-description">Notes</label>
          <textarea
            id="profile-description"
            maxLength={180}
            rows={2}
            value={draft.description}
            onChange={(e) => update("description", e.target.value)}
          />
        </div>
        {error && (
          <p className="field-error" role="alert">
            {error}
          </p>
        )}
        <div className="button-row">
          <button className="primary" type="submit">
            Save profile
          </button>
          <button className="secondary" type="button" onClick={duplicate}>
            Copy profile
          </button>
        </div>
        <p className="helper" aria-live="polite">
          {dirty ? "Unsaved changes" : saved ? "Saved" : ""}
        </p>
      </form>
      <div className="profile-bottom">
        <span>{profile.enabled.length} items enabled</span>
        <button
          className="text-button danger"
          disabled={!canRemove}
          onClick={remove}
        >
          Remove profile
        </button>
      </div>
    </section>
  );
}

export default function Profiles({
  state,
  select,
  update,
  create,
  duplicate,
  remove,
}) {
  const [dialog, setDialog] = useState(null);
  const [name, setName] = useState("");
  const [error, setError] = useState("");
  const profile = state.profiles.find((p) => p.id === state.selected);
  const save = (draft) => {
    if (!draft.name.trim() || !draft.character.trim() || !draft.server.trim())
      return "Add a profile name, character, and server.";
    if (
      state.profiles.some(
        (p) =>
          p.id !== profile.id &&
          p.name.toLowerCase() === draft.name.trim().toLowerCase(),
      )
    )
      return "That name is already in use. Try another one.";
    update({
      ...draft,
      name: draft.name.trim(),
      character: draft.character.trim(),
      server: draft.server.trim(),
    });
    return "";
  };
  return (
    <>
      <div className="page-title">
        <div>
          <h1>Profiles</h1>
        </div>
        <button
          className="primary small"
          disabled={state.profiles.length >= 30}
          onClick={() => {
            setDialog("new");
            setName("");
            setError("");
          }}
        >
          <Icon name="plus" />
          New profile
        </button>
      </div>
      <div className="profiles-layout">
        <div className="profile-list" aria-label="Choose a profile">
          {state.profiles.map((p) => (
            <button
              key={p.id}
              aria-pressed={p.id === profile.id}
              className={
                "profile-choice " + (p.id === profile.id ? "selected" : "")
              }
              onClick={() => select(p.id)}
            >
              <span className="choice-heading">
                {p.name}
                {p.id === profile.id && <Icon name="check" size={18} />}
              </span>
              <span>
                {p.character} · {p.server}
              </span>
            </button>
          ))}
        </div>
        <Editor
          key={profile.id}
          profile={profile}
          save={save}
          duplicate={duplicate}
          remove={() => setDialog("remove")}
          canRemove={state.profiles.length > 1}
        />
      </div>
      {dialog === "new" && (
        <Modal title="New profile" close={() => setDialog(null)}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const message = create(name);
              if (message) setError(message);
              else setDialog(null);
            }}
          >
            <p>Copies the current profile’s settings.</p>
            <div className="field">
              <label htmlFor="new-profile-name">Profile name</label>
              <input
                id="new-profile-name"
                autoFocus
                maxLength={60}
                required
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </div>
            {error && (
              <p className="field-error" role="alert">
                {error}
              </p>
            )}
            <div className="button-row">
              <button className="primary">Create profile</button>
              <button
                type="button"
                className="secondary"
                onClick={() => setDialog(null)}
              >
                Cancel
              </button>
            </div>
          </form>
        </Modal>
      )}
      {dialog === "remove" && (
        <Modal title="Remove profile?" close={() => setDialog(null)}>
          <p>Remove “{profile.name}” from this preview?</p>
          <div className="button-row">
            <button
              className="primary"
              onClick={() => {
                remove();
                setDialog(null);
              }}
            >
              Remove profile
            </button>
            <button className="secondary" onClick={() => setDialog(null)}>
              Keep profile
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}
