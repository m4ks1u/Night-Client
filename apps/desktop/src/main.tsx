import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import "./styles/app.css";

const root = document.getElementById("root");
if (!root) throw new Error("#root missing");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// Startup benchmark: report once the first frame after mount is painted.
requestAnimationFrame(() => setTimeout(() => void invoke("ui_ready").catch(() => {}), 0));

// Block the WebView's default context menu outside text fields (desktop feel).
window.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement;
  if (!t.closest("input, textarea, [role=log]")) e.preventDefault();
});
