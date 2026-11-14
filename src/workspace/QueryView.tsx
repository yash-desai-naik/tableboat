import { useEffect, useRef, useState } from "react";
import { cancelActiveRun, FORMAT_RESULT_EVENT, formatActive, runAll, runCurrent, type FormatResult } from "../editor/actions";
import { SqlEditor } from "../editor/SqlEditor";
import type { SessionInfo } from "../lib/connections";
import { fmtShortcut } from "../lib/platform";
import { dialectOf, getRunState, isReadOnlyRun, rerunTab, useRunState } from "../lib/query";
import { onRefresh } from "../lib/refresh";
import { updateSettings, useSettings } from "../lib/settings";
import type { QueryTab } from "../lib/workspace";
import { useWorkspace } from "../lib/workspace";
import { ResultsPane } from "../results/ResultsPane";

/** A SQL tab: editor on top, results below, with a draggable divider. */
export function QueryView({ info, tab }: { info: SessionInfo; tab: QueryTab }) {
  // Refresh (Ctrl+R, or after Claude changed data): run the last query again, only if it just reads.
  useEffect(
    () =>
      onRefresh(() => {
        const last = getRunState(tab.id).lastRun;
        if (last && isReadOnlyRun(last.statements)) void rerunTab(tab.id);
      }),
    [tab.id],
  );
  const settings = useSettings();
  const ws = useWorkspace();
  const run = useRunState(tab.id);
  const engine = info.server.engine ?? info.connection.engine;
  const dark = document.documentElement.dataset.theme === "dark";
  const [formatError, setFormatError] = useState<string | null>(null);
  // Format runs from the button, Shift+Alt+F or the palette: show its error here either way.
  useEffect(() => {
    const onResult = (e: Event) => {
      const { tabId, error } = (e as CustomEvent<FormatResult>).detail;
      if (tabId === tab.id) setFormatError(error);
    };
    window.addEventListener(FORMAT_RESULT_EVENT, onResult);
    return () => window.removeEventListener(FORMAT_RESULT_EVENT, onResult);
  }, [tab.id]);
  const container = useRef<HTMLDivElement>(null);
  const drag = useRef<{ y: number; h: number } | null>(null);

  return (
    <div className="query-view" ref={container}>
      <div className="editor-bar">
        <button
          className="btn primary small-btn"
          onClick={runCurrent}
          disabled={run.running}
          title={`Run the statement under the cursor, or the selection (${fmtShortcut("Ctrl+Enter")})`}
        >
          Run current
        </button>
        <button
          className="btn small-btn"
          onClick={runAll}
          disabled={run.running}
          title={`Run every statement (${fmtShortcut("Ctrl+Shift+Enter")})`}
        >
          Run all
        </button>
        {run.running && (
          <button className="btn small-btn" onClick={cancelActiveRun} title={`Cancel (${fmtShortcut("Ctrl+.")})`}>
            Cancel
          </button>
        )}
        <button
          className="btn small-btn"
          title={`Format the selection or the statement under the cursor (${fmtShortcut("Shift+Alt+F")}). ${fmtShortcut("Ctrl+Shift+Alt+F")} formats everything.`}
          onClick={() => void formatActive()}
        >
          Format
        </button>
        {formatError && (
          <span className="editor-error" title={formatError}>
            {formatError.split("\n")[0]}
          </span>
        )}
        <span className="spacer" />
        <span className="muted small mono">
          {engine === "sqlite" ? "main" : (ws.database ?? "no database selected")}
        </span>
      </div>

      <div className="editor-area">
        <SqlEditor
          tabId={tab.id}
          initialText={tab.initialText}
          engine={engine}
          sessionId={info.sessionId}
          database={engine === "sqlite" ? "main" : ws.database}
          dark={dark}
          wordWrap={settings.wordWrap}
        />
      </div>

      <div
        className="hsplitter"
        role="separator"
        aria-orientation="horizontal"
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          drag.current = { y: e.clientY, h: settings.resultsHeight };
          document.body.classList.add("resizing-v");
        }}
        onPointerMove={(e) => {
          if (!drag.current || !container.current) return;
          const max = container.current.clientHeight - 120;
          const h = Math.round(Math.max(80, Math.min(max, drag.current.h - (e.clientY - drag.current.y))));
          updateSettings({ resultsHeight: h }, { debounceMs: 400 });
        }}
        onPointerUp={(e) => {
          e.currentTarget.releasePointerCapture(e.pointerId);
          drag.current = null;
          document.body.classList.remove("resizing-v");
        }}
        onDoubleClick={() => updateSettings({ resultsHeight: 320 })}
      />

      <div className="results-area" style={{ height: settings.resultsHeight }}>
        <ResultsPane
          tabId={tab.id}
          dialect={dialectOf(engine)}
          table={null}
          onCancel={cancelActiveRun}
          emptyText={`${fmtShortcut("Ctrl+Enter")} runs the statement under the cursor · ${fmtShortcut("Ctrl+Shift+Enter")} runs everything`}
        />
      </div>
    </div>
  );
}
