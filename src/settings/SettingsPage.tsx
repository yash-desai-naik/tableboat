import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { OBJECT_KINDS } from "../lib/meta";
import { updateSettings, useSettings, type Settings } from "../lib/settings";
import { Dialog } from "../shell/Dialog";
import { useAppVersion, versionLine } from "../shell/AboutDialog";
import { uiActions } from "../lib/ui";

type Def =
  | { key: keyof Settings; type: "select"; options: [string | number, string][] }
  | { key: keyof Settings; type: "number"; min: number; max: number; step?: number; suffix?: string }
  | { key: keyof Settings; type: "toggle" }
  | { key: "sidebarKinds"; type: "kinds" };

type Entry = Def & { section: string; label: string; help?: string };

/** Every setting, in the order shown. */
const ENTRIES: Entry[] = [
  {
    section: "Appearance",
    key: "theme",
    type: "select",
    label: "Theme",
    options: [
      ["system", "Match Windows"],
      ["light", "Light"],
      ["dark", "Dark"],
    ],
  },
  {
    section: "Appearance",
    key: "density",
    type: "select",
    label: "Density",
    help: "Row height in the grid, sidebar and tabs.",
    options: [
      ["compact", "Compact"],
      ["default", "Default"],
      ["comfortable", "Comfortable"],
    ],
  },
  {
    section: "Appearance",
    key: "uiZoom",
    type: "select",
    label: "Zoom",
    help: "Ctrl+= / Ctrl+- / Ctrl+0",
    options: [0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2].map((z) => [z, `${Math.round(z * 100)}%`]),
  },
  { section: "Appearance", key: "editorFontSize", type: "number", label: "Editor font size", min: 8, max: 40, suffix: "px" },
  { section: "Appearance", key: "gridFontSize", type: "number", label: "Results font size", min: 8, max: 40, suffix: "px" },

  { section: "SQL editor", key: "wordWrap", type: "toggle", label: "Wrap long lines", help: "Alt+Z" },
  {
    section: "SQL editor",
    key: "stopOnError",
    type: "toggle",
    label: "Stop at the first error",
    help: "When running several statements, don't run the rest after one fails.",
  },
  {
    section: "SQL editor",
    key: "maxRows",
    type: "number",
    label: "Row limit for queries",
    help: "The rest of a large result isn't downloaded. Keeps memory low.",
    min: 100,
    max: 5_000_000,
    step: 1000,
    suffix: "rows",
  },

  { section: "Tables", key: "pageSize", type: "number", label: "Rows per page", min: 10, max: 100_000, step: 50, suffix: "rows" },
  {
    section: "Tables",
    key: "sortDescendingFirst",
    type: "toggle",
    label: "Sort descending on first header click",
    help: "As TablePlus does. Off: ascending first.",
  },

  { section: "Sidebar", key: "sidebarKinds", type: "kinds", label: "Object types shown" },

  {
    section: "Start-up",
    key: "restoreTabs",
    type: "toggle",
    label: "Reopen tabs when connecting",
    help: "Each connection remembers its tabs, including SQL you typed.",
  },
  {
    section: "Start-up",
    key: "reopenLastConnection",
    type: "toggle",
    label: "Reconnect to the last connection at start-up",
    help: "Skipped after you disconnect, and for connections that ask for their password.",
  },

  {
    section: "Claude",
    key: "claudeModel",
    type: "select",
    label: "Model",
    help: "Used from the next message. Cheaper models are much less expensive per question.",
    options: [
      ["", "Claude Code's default"],
      ["sonnet", "Sonnet (balanced)"],
      ["haiku", "Haiku (fastest, cheapest)"],
      ["opus", "Opus (most capable)"],
    ],
  },
];

/** Ctrl+, : every setting, searchable. Changes apply and save immediately. */
export function SettingsPage({ onClose }: { onClose: () => void }) {
  const settings = useSettings();
  const [query, setQuery] = useState("");
  const [fileNote, setFileNote] = useState<string | null>(null);
  const version = useAppVersion();

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? ENTRIES.filter((e) => `${e.section} ${e.label} ${e.help ?? ""}`.toLowerCase().includes(q)) : ENTRIES;
  }, [query]);

  const sections = [...new Set(visible.map((e) => e.section))];

  return (
    <Dialog
      title="Settings"
      onClose={onClose}
      width={680}
      footer={
        <>
          <button
            className="link-btn"
            onClick={() =>
              void invoke<string>("open_settings_file")
                .then((p) => setFileNote(`Opened ${p}`))
                .catch((e) => setFileNote(String(e)))
            }
          >
            Open settings.json
          </button>
          {fileNote && <span className="muted small">{fileNote}</span>}
          <span className="spacer" />
          <button
            className="link-btn settings-version"
            title="About Tableboat: version, build, folders"
            onClick={() => {
              onClose();
              uiActions.openAbout();
            }}
          >
            {version ? versionLine(version) : "Tableboat"}
          </button>
          <button className="btn primary" onClick={onClose}>
            Done
          </button>
        </>
      }
    >
      <input
        className="input"
        placeholder="Search settings"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        spellCheck={false}
      />
      {visible.length === 0 && <p className="muted">No setting matches “{query}”.</p>}
      {sections.map((section) => (
        <section key={section} className="form-section">
          <div className="form-section-head">
            <span>{section}</span>
          </div>
          {visible
            .filter((e) => e.section === section)
            .map((e) => (
              <div key={e.key} className="setting-row">
                <div className="setting-text">
                  <div>{e.label}</div>
                  {e.help && <div className="muted small">{e.help}</div>}
                </div>
                <div className="setting-control">
                  <Control entry={e} settings={settings} />
                </div>
              </div>
            ))}
        </section>
      ))}
    </Dialog>
  );
}

function Control({ entry, settings }: { entry: Entry; settings: Settings }) {
  const value = settings[entry.key];
  switch (entry.type) {
    case "select":
      return (
        <select
          className="input"
          value={String(value)}
          onChange={(e) => {
            const raw = entry.options.find(([v]) => String(v) === e.target.value)?.[0];
            updateSettings({ [entry.key]: raw } as Partial<Settings>);
          }}
        >
          {entry.options.map(([v, label]) => (
            <option key={String(v)} value={String(v)}>
              {label}
            </option>
          ))}
        </select>
      );
    case "number":
      return (
        <NumberInput
          value={Number(value)}
          min={entry.min}
          max={entry.max}
          step={entry.step ?? 1}
          suffix={entry.suffix}
          onChange={(n) => updateSettings({ [entry.key]: n } as Partial<Settings>, { debounceMs: 300 })}
        />
      );
    case "toggle":
      return (
        <label className="switch">
          <input
            type="checkbox"
            checked={Boolean(value)}
            onChange={(e) => updateSettings({ [entry.key]: e.target.checked } as Partial<Settings>)}
          />
          <span>{value ? "On" : "Off"}</span>
        </label>
      );
    case "kinds":
      return (
        <div className="kind-checks">
          {OBJECT_KINDS.map((k) => (
            <label key={k.id} className="check">
              <input
                type="checkbox"
                checked={settings.sidebarKinds.includes(k.id)}
                onChange={(e) => {
                  const next = e.target.checked
                    ? [...settings.sidebarKinds, k.id]
                    : settings.sidebarKinds.filter((x) => x !== k.id);
                  updateSettings({ sidebarKinds: OBJECT_KINDS.map((o) => o.id).filter((id) => next.includes(id)) });
                }}
              />
              {k.plural}
            </label>
          ))}
        </div>
      );
  }
}

/** A number field that only saves valid values within range (on blur or Enter for typed values). */
function NumberInput(props: { value: number; min: number; max: number; step: number; suffix?: string; onChange: (n: number) => void }) {
  const [text, setText] = useState(String(props.value));
  const [focused, setFocused] = useState(false);
  const shown = focused ? text : String(props.value);
  const commit = (raw: string) => {
    const n = Math.round(Number(raw.replace(/[,_\s]/g, "")));
    if (Number.isFinite(n)) props.onChange(Math.min(props.max, Math.max(props.min, n)));
  };
  return (
    <span className="inline">
      <input
        className="input mono number-input"
        inputMode="numeric"
        value={shown}
        onFocus={() => {
          setText(String(props.value));
          setFocused(true);
        }}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => {
          setFocused(false);
          commit(text);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit(text);
          if (e.key === "ArrowUp" || e.key === "ArrowDown") {
            e.preventDefault();
            const next = Math.min(props.max, Math.max(props.min, props.value + (e.key === "ArrowUp" ? props.step : -props.step)));
            setText(String(next));
            props.onChange(next);
          }
        }}
      />
      {props.suffix && <span className="muted small">{props.suffix}</span>}
    </span>
  );
}
