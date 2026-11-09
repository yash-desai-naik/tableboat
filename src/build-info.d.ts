/** Set by vite.config.ts at build time. */
declare const __APP_BUILD__: {
  /** Short git commit the app was built from ("unknown" outside a git checkout). */
  commit: string;
  /** The working tree had uncommitted changes at build time. */
  dirty: boolean;
  /** ISO time of the build. */
  date: string;
  /** A development run (`tauri dev`), not a release build. */
  dev: boolean;
};
