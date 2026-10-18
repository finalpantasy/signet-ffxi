import React, { useEffect, useRef, useState } from "react";

export function Icon({ name, size = 22 }) {
  const paths = {
    play: <path d="m7 4 14 8-14 8z" fill="currentColor" stroke="none" />,
    profiles: (
      <>
        <circle cx="12" cy="7" r="4" />
        <path d="M4 21v-3a8 8 0 0 1 16 0v3Z" />
      </>
    ),
    library: (
      <>
        <rect x="3" y="3" width="7" height="7" rx="1" />
        <rect x="14" y="3" width="7" height="7" rx="1" />
        <rect x="3" y="14" width="7" height="7" rx="1" />
        <rect x="14" y="14" width="7" height="7" rx="1" />
      </>
    ),
    graphics: (
      <>
        <rect x="2" y="3" width="20" height="14" rx="1" />
        <path d="M8 22h8m-4-5v5" />
      </>
    ),
    settings: (
      <>
        <path d="M4 6h16M4 12h16M4 18h16" />
        <circle cx="8" cy="6" r="2" fill="var(--canvas)" />
        <circle cx="16" cy="12" r="2" fill="var(--canvas)" />
        <circle cx="10" cy="18" r="2" fill="var(--canvas)" />
      </>
    ),
    close: <path d="m6 6 12 12M6 18 18 6" />,
    up: <path d="m5 15 7-7 7 7" />,
    down: <path d="m5 9 7 7 7-7" />,
    check: <path d="m4 12 5 5L20 6" />,
    plus: <path d="M12 4v16M4 12h16" />,
  };
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {paths[name] || paths.library}
    </svg>
  );
}

export function Toggle({ label, checked, onChange, disabled = false }) {
  return (
    <button
      type="button"
      role="switch"
      aria-label={label}
      aria-checked={checked}
      disabled={disabled}
      className={"toggle " + (checked ? "on" : "")}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  );
}

export function Modal({ title, children, close }) {
  const ref = useRef(null);
  useEffect(() => {
    const dialog = ref.current;
    const opener = document.activeElement;
    dialog.showModal();
    return () => {
      dialog.close();
      if (opener instanceof HTMLElement && opener.isConnected) {
        opener.focus({ preventScroll: true });
      }
    };
  }, []);
  return (
    <dialog
      ref={ref}
      className="modal"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      onClick={(e) => {
        if (e.target === ref.current) close();
      }}
      onKeyDown={(e) => {
        if (e.key !== "Tab") return;
        const controls = [
          ...e.currentTarget.querySelectorAll(
            "button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [href], [tabindex]:not([tabindex='-1'])",
          ),
        ].filter((control) => control.getClientRects().length > 0);
        const first = controls[0];
        const last = controls[controls.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last?.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first?.focus();
        }
      }}
      aria-labelledby="dialog-title"
    >
      <div className="modal-body">
        <div className="modal-heading">
          <h2 id="dialog-title">{title}</h2>
          <button
            type="button"
            className="icon-button"
            aria-label="Close dialog"
            onClick={close}
          >
            <Icon name="close" />
          </button>
        </div>
        {children}
      </div>
    </dialog>
  );
}

export function RangeField({ field, value, onChange }) {
  const [draft, setDraft] = useState(String(value));
  const [invalid, setInvalid] = useState(false);
  useEffect(() => {
    setDraft(String(value));
    setInvalid(false);
  }, [value]);
  const commit = () => {
    const number = Number(draft);
    if (
      draft.trim() === "" ||
      !Number.isFinite(number) ||
      number < field.min ||
      number > field.max
    ) {
      setInvalid(true);
      return;
    }
    const snapped = Math.min(
      field.max,
      Math.max(
        field.min,
        Math.round((number - field.min) / field.step) * field.step + field.min,
      ),
    );
    setInvalid(false);
    onChange(Number(snapped.toFixed(2)));
    setDraft(String(Number(snapped.toFixed(2))));
  };
  return (
    <div className="field">
      <label htmlFor={field.id + "-number"}>{field.label}</label>
      <div className="range-line">
        <input
          aria-label={field.label + " slider"}
          type="range"
          min={field.min}
          max={field.max}
          step={field.step}
          value={value}
          onChange={(e) => onChange(Number(e.target.value))}
        />
        <div className="number-unit">
          <input
            id={field.id + "-number"}
            type="number"
            aria-invalid={invalid}
            min={field.min}
            max={field.max}
            step={field.step}
            value={draft}
            onChange={(e) => {
              setDraft(e.target.value);
              setInvalid(false);
            }}
            onBlur={commit}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                commit();
              }
            }}
          />
          <span>{field.unit}</span>
        </div>
      </div>
      <p className={invalid ? "field-error" : "helper"}>
        {invalid
          ? `Choose a number from ${field.min} to ${field.max}.`
          : field.help}
      </p>
    </div>
  );
}

export function SchemaFields({ fields, values, onChange }) {
  return fields.map((field) => {
    const value = values[field.id] ?? field.default;
    if (field.type === "range")
      return (
        <RangeField
          key={field.id}
          field={field}
          value={value}
          onChange={(v) => onChange(field.id, v)}
        />
      );
    if (field.type === "toggle")
      return (
        <div className="setting-row" key={field.id}>
          <div>
            <h3>{field.label}</h3>
            <p>{field.help}</p>
          </div>
          <Toggle
            label={field.label}
            checked={value}
            onChange={(v) => onChange(field.id, v)}
          />
        </div>
      );
    return (
      <div className="field" key={field.id}>
        <label htmlFor={field.id}>{field.label}</label>
        <select
          id={field.id}
          value={value}
          onChange={(e) => onChange(field.id, e.target.value)}
        >
          {field.choices.map((v) => (
            <option key={v}>{v}</option>
          ))}
        </select>
        <p className="helper">{field.help}</p>
      </div>
    );
  });
}

export function ProfilePicker({ profiles, selected, onChange }) {
  return (
    <div className="compact-picker">
      <label htmlFor="active-profile">Current profile</label>
      <select
        id="active-profile"
        value={selected}
        onChange={(e) => onChange(e.target.value)}
      >
        {profiles.map((p) => (
          <option value={p.id} key={p.id}>
            {p.name}
          </option>
        ))}
      </select>
    </div>
  );
}
