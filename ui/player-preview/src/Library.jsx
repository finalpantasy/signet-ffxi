import React, { useState } from "react";
import { conflicts, packs } from "./data.js";
import { Icon, Modal, SchemaFields, Toggle } from "./components.jsx";

export default function Library({ state, profile, select, toggle, update }) {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("All");
  const [enabledOnly, setEnabledOnly] = useState(false);
  const [editing, setEditing] = useState(null);
  const ordered = [
    ...profile.order.map((id) => packs.find((p) => p.id === id)),
    ...packs.filter((p) => p.type !== "Visual packs"),
  ];
  const visible = ordered.filter(
    (p) =>
      (category === "All" || p.type === category) &&
      (!enabledOnly || profile.enabled.includes(p.id)) &&
      `${p.name} ${p.description} ${p.source}`
        .toLowerCase()
        .includes(query.toLowerCase()),
  );
  const overlaps = conflicts(profile);
  const move = (id, direction) => {
    const order = [...profile.order];
    const index = order.indexOf(id);
    const target = index + direction;
    if (target < 0 || target >= order.length) return;
    [order[index], order[target]] = [order[target], order[index]];
    update({ order });
  };
  const values = editing ? profile.addonSettings[editing.id] || {} : {};
  return (
    <>
      <div className="page-title">
        <div>
          <h1>Addons &amp; Mods</h1>
        </div>
      </div>
      <div className="library-toolbar">
        <label className="search-field">
          <span className="sr-only">Search addons and mods</span>
          <input
            type="search"
            placeholder="Search addons & mods"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </label>
        <label className="check-label">
          <input
            type="checkbox"
            checked={enabledOnly}
            onChange={(e) => setEnabledOnly(e.target.checked)}
          />
          Only enabled
        </label>
      </div>
      <div className="category-nav" aria-label="Filter item type">
        {["All", "Visual packs", "Addons", "Plugins"].map((c) => (
          <button
            key={c}
            aria-pressed={category === c}
            className={category === c ? "selected" : ""}
            onClick={() => setCategory(c)}
          >
            {c}
          </button>
        ))}
      </div>
      <p className="library-hint">
        Sample library · folders load from top to bottom.
      </p>
      <div className="library-list">
        {visible.map((pack) => (
          <article
            className={"library-row " + (pack.unavailable ? "unavailable" : "")}
            key={pack.id}
          >
            <div className="library-item">
              <div className="item-heading">
                <h2>{pack.name}</h2>
                <span className="kind">
                  {pack.type === "Visual packs"
                    ? "Folder"
                    : pack.type === "Addons"
                      ? "Addon"
                      : "Plugin"}
                </span>
              </div>
              <span className="item-origin">{pack.source}</span>
            </div>
            <div className="row-actions">
              {pack.type === "Visual packs" && (
                <div className="order-actions">
                  <button
                    className="icon-button"
                    aria-label={"Move " + pack.name + " up"}
                    disabled={profile.order.indexOf(pack.id) === 0}
                    onClick={() => move(pack.id, -1)}
                  >
                    <Icon name="up" size={18} />
                  </button>
                  <button
                    className="icon-button"
                    aria-label={"Move " + pack.name + " down"}
                    disabled={
                      profile.order.indexOf(pack.id) ===
                      profile.order.length - 1
                    }
                    onClick={() => move(pack.id, 1)}
                  >
                    <Icon name="down" size={18} />
                  </button>
                </div>
              )}
              {!pack.unavailable && (
                <button
                  className="text-button"
                  onClick={() => setEditing(pack)}
                >
                  Details
                </button>
              )}
              <Toggle
                label={"Enable " + pack.name}
                checked={profile.enabled.includes(pack.id)}
                onChange={() => toggle(pack.id)}
                disabled={pack.unavailable}
              />
            </div>
          </article>
        ))}
        {!visible.length && (
          <div className="empty-state">
            <h2>No matches</h2>
            <p>Try another name, or clear your filters.</p>
            <button
              className="secondary"
              onClick={() => {
                setQuery("");
                setCategory("All");
                setEnabledOnly(false);
              }}
            >
              Clear filters
            </button>
          </div>
        )}
      </div>
      <details className="conflict-section advanced">
        <summary>
          File conflicts{overlaps.length ? ` (${overlaps.length})` : ""}
        </summary>
        {overlaps.length ? (
          <>
            <p>
              First enabled folder wins. Reorder folders to change priority.
            </p>
            {overlaps.map((c) => (
              <div className="conflict-row" key={c.path}>
                <div>
                  <strong>{c.winner} takes priority</strong>
                  <p>{c.names.join(" and ")}</p>
                </div>
                <code>{c.path}</code>
              </div>
            ))}
          </>
        ) : (
          <p>No overlapping files in your enabled sample folders.</p>
        )}
      </details>
      {editing && (
        <Modal title={editing.name} close={() => setEditing(null)}>
          <p>{editing.description}</p>
          {editing.fields.length ? (
            <SchemaFields
              fields={editing.fields}
              values={values}
              onChange={(field, value) =>
                update({
                  addonSettings: {
                    ...profile.addonSettings,
                    [editing.id]: { ...values, [field]: value },
                  },
                })
              }
            />
          ) : (
            <p>No additional settings.</p>
          )}
          {editing.files && (
            <details className="file-details">
              <summary>Sample files in this folder</summary>
              {editing.files.map((file) => (
                <code key={file}>{file}</code>
              ))}
            </details>
          )}
          <div className="button-row">
            <button className="primary" onClick={() => setEditing(null)}>
              Done
            </button>
            {editing.fields.length > 0 && (
              <button
                className="secondary"
                onClick={() =>
                  update({
                    addonSettings: {
                      ...profile.addonSettings,
                      [editing.id]: {},
                    },
                  })
                }
              >
                Restore defaults
              </button>
            )}
          </div>
        </Modal>
      )}
    </>
  );
}
