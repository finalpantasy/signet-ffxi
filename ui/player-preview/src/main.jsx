import React from "react";
import { createRoot } from "react-dom/client";
import App from "./App.jsx";
import NativeApp from "./NativeApp.jsx";
import "./styles.css";
// Native mode is explicit. A disconnected native bridge never falls back to samples.
const native =
  document.querySelector('meta[name="signet-native"]')?.content === "1" ||
  new URLSearchParams(location.search).get("native") === "1";
createRoot(document.getElementById("root")).render(
  native ? <NativeApp /> : <App />,
);
