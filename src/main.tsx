import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import { loadSettings } from "./lib/settings";
import { applyTheme, resolveTheme } from "./themes";
// JetBrains Mono, bundled: used only when it isn't installed (the installed copy comes first).
import "@fontsource-variable/jetbrains-mono/wght.css";
import "@fontsource-variable/jetbrains-mono/wght-italic.css";
import "./styles.css";
import "./controls.css";
import "./workspace.css";
import "./results.css";
import "./ai.css";
import "./type.css";

const appWindow = getCurrentWindow();

/**
 * The window starts hidden (Rust has sized and centered it). Maximize it, then show it after
 * the first paint so there's no white flash. "Restore Down" returns to the centered size.
 */
function showWhenPainted() {
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      void appWindow
        .maximize()
        .catch(() => {})
        .then(() => appWindow.show())
        // Windows doesn't record a restore position for a window maximized while hidden.
        .then(() => invoke("window_fit_restore"))
        .catch(() => {});
    }),
  );
}

loadSettings()
  .then((settings) => {
    // Set the theme before the first render so the window never shows the wrong colours.
    const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    applyTheme(resolveTheme(settings, prefersDark));
    document.documentElement.dataset.density = settings.density;
    createRoot(document.getElementById("root")!).render(<App />);
    showWhenPainted();
  })
  .catch((err) => {
    document.body.textContent = `Tableboat could not load its settings: ${err}`;
    void appWindow.show();
  });
