import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./app/app";
import { startDesktopTrace } from "./runtime-logging";
import "./styles.css";

startDesktopTrace((event) => {
  console.info(JSON.stringify(event));
});

const rootElement = document.getElementById("root");

if (rootElement === null) {
  throw new Error("Elemen root aplikasi desktop tidak ditemukan.");
}

createRoot(rootElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
