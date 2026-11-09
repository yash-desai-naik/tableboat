import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import { loadSettings } from "./lib/settings";
import "./styles.css";
import "./controls.css";
import "./workspace.css";
import "./results.css";
import "./ai.css";

const appWindow = getCurrentWindow();

/** The window starts hidden; show it after the first paint so there's no white flash. */
function showWhenPainted() {
  requestAnimationFrame(() => requestAnimationFrame(() => void appWindow.show()));
}

loadSettings()
  .then((settings) => {
    // Set the theme before the first render so the window never shows the wrong colours.
    const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    document.documentElement.dataset.theme =
      settings.theme === "system" ? (prefersDark ? "dark" : "light") : settings.theme;
    document.documentElement.dataset.density = settings.density;
    createRoot(document.getElementById("root")!).render(<App />);
    showWhenPainted();
  })
  .catch((err) => {
    document.body.textContent = `Tableboat could not load its settings: ${err}`;
    void appWindow.show();
  });
