// Original line icons, 20px grid, 1.5 stroke. No third-party icon set.
import React from "react";

const I = ({ children, ...p }) => (
  <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" {...p}>
    {children}
  </svg>
);

export const Mark = (p) => (
  <svg viewBox="0 0 24 24" aria-hidden="true" {...p}>
    <defs>
      <linearGradient id="mk" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stopColor="#ecd394" />
        <stop offset="1" stopColor="#b38a3e" />
      </linearGradient>
    </defs>
    <path d="M12 1.8 21 12l-9 10.2L3 12z" fill="url(#mk)" />
    <path d="M12 1.8 21 12h-9z" fill="#fff" opacity=".28" />
    <path d="M12 22.2 3 12h9z" fill="#000" opacity=".14" />
    <path d="M12 7.2 16.2 12 12 16.8 7.8 12z" fill="#fbf1d6" opacity=".92" />
  </svg>
);

export const Home = (p) => (
  <I {...p}>
    <path d="M3.5 9 10 3.5 16.5 9v7a1 1 0 0 1-1 1h-3.5v-4.5h-4V17H4.5a1 1 0 0 1-1-1z" />
  </I>
);
export const Globe = (p) => (
  <I {...p}>
    <circle cx="10" cy="10" r="7" />
    <path d="M3 10h14M10 3c2 2.2 2.8 4.5 2.8 7s-.8 4.8-2.8 7c-2-2.2-2.8-4.5-2.8-7S8 5.2 10 3z" />
  </I>
);
export const Server = (p) => (
  <I {...p}>
    <rect x="3.5" y="3.5" width="13" height="5.5" rx="1.5" />
    <rect x="3.5" y="11" width="13" height="5.5" rx="1.5" />
    <path d="M6.5 6.25h.01M6.5 13.75h.01M10 6.25h3.5M10 13.75h3.5" />
  </I>
);
export const Crystal = (p) => (
  <I {...p}>
    <path d="M10 2.5 15.5 8 10 17.5 4.5 8z" />
    <path d="M4.5 8h11M10 2.5 8 8l2 9.5L12 8z" />
  </I>
);
export const Gear = (p) => (
  <I {...p}>
    <circle cx="10" cy="10" r="2.5" />
    <path d="M10 2.75v2M10 15.25v2M17.25 10h-2M4.75 10h-2M15.1 4.9l-1.4 1.4M6.3 13.7l-1.4 1.4M15.1 15.1l-1.4-1.4M6.3 6.3 4.9 4.9" />
    <circle cx="10" cy="10" r="5.25" />
  </I>
);
export const Blocks = (p) => (
  <I {...p}>
    <rect x="3.5" y="3.5" width="5.5" height="5.5" rx="1.25" />
    <rect x="11" y="3.5" width="5.5" height="5.5" rx="1.25" />
    <rect x="3.5" y="11" width="5.5" height="5.5" rx="1.25" />
    <path d="M13.75 11v5.5M11 13.75h5.5" />
  </I>
);
export const Play = (p) => (
  <svg viewBox="0 0 20 20" aria-hidden="true" {...p}>
    <path d="M6.5 4.2v11.6a.8.8 0 0 0 1.2.7l9-5.8a.8.8 0 0 0 0-1.4l-9-5.8a.8.8 0 0 0-1.2.7z" fill="currentColor" />
  </svg>
);
export const Search = (p) => (
  <I {...p}>
    <circle cx="9" cy="9" r="5.5" />
    <path d="m13 13 3.5 3.5" />
  </I>
);
export const Star = ({ filled, ...p }) => (
  <I {...p}>
    <path d="m10 3 2.1 4.4 4.8.6-3.5 3.3.9 4.7L10 13.7 5.7 16l.9-4.7L3.1 8l4.8-.6z" fill={filled ? "currentColor" : "none"} />
  </I>
);
export const Folder = (p) => (
  <I {...p}>
    <path d="M3 6a1.5 1.5 0 0 1 1.5-1.5h3.2l1.6 1.8h6.2A1.5 1.5 0 0 1 17 7.8v6.7a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 3 14.5z" />
  </I>
);
export const Copy = (p) => (
  <I {...p}>
    <rect x="7" y="7" width="9.5" height="9.5" rx="1.5" />
    <path d="M13 4.5A1.5 1.5 0 0 0 11.5 3h-7A1.5 1.5 0 0 0 3 4.5v7A1.5 1.5 0 0 0 4.5 13" />
  </I>
);
export const Alert = (p) => (
  <I {...p}>
    <path d="M10 3.2 17.4 16H2.6z" />
    <path d="M10 8.2v3.6M10 14h.01" />
  </I>
);
export const Check = (p) => (
  <I {...p}>
    <circle cx="10" cy="10" r="7" />
    <path d="m6.8 10.2 2.2 2.2 4.2-4.6" />
  </I>
);
export const Stop = (p) => (
  <I {...p}>
    <rect x="5.5" y="5.5" width="9" height="9" rx="1.5" />
  </I>
);
export const Swap = (p) => (
  <I {...p}>
    <path d="M4 7h11l-3-3M16 13H5l3 3" />
  </I>
);
export const Compass = (p) => (
  <I {...p}>
    <circle cx="10" cy="10" r="7" />
    <path d="m12.8 7.2-1.6 4-4 1.6 1.6-4z" />
  </I>
);
export const Refresh = (p) => (
  <I {...p}>
    <path d="M15.5 9.5A5.5 5.5 0 1 1 13.9 6" />
    <path d="M14.2 2.8v3.4h-3.4" />
  </I>
);
