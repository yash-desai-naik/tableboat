import { useEffect, useState } from "react";
import { errorText } from "../lib/connections";
import { metaApi, OBJECT_KINDS } from "../lib/meta";
import { fmtShortcut } from "../lib/platform";
import type { ObjectTab } from "../lib/workspace";
import { KindIcon } from "../shell/KindIcon";
import { onRefresh } from "../lib/refresh";

/**
 * An object's definition. For tables and views, the data grid replaces this as the
 * main view in a later step; the definition stays available.
 */
export function ObjectView({ sessionId, tab }: { sessionId: string; tab: ObjectTab }) {
  const [ddl, setDdl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [tick, setTick] = useState(0);
  useEffect(() => onRefresh(() => setTick((t) => t + 1)), []);

  useEffect(() => {
    let stale = false;
    setDdl(null);
    setError(null);
    metaApi
      .ddl(sessionId, tab.database, tab.kind, tab.name)
      .then((text) => !stale && setDdl(text))
      .catch((err) => !stale && setError(errorText(err)));
    return () => {
      stale = true;
    };
  }, [sessionId, tab.database, tab.kind, tab.name, tick]);

  const kind = OBJECT_KINDS.find((k) => k.id === tab.kind)?.label ?? tab.kind;

  return (
    <div className="object-view">
      <div className="object-head">
        <KindIcon kind={tab.kind} size={16} />
        <span className="object-name">{tab.name}</span>
        <span className="muted">
          {kind} in {tab.database}
        </span>
        <span className="spacer" />
        <button
          className="btn"
          onClick={() => setTick((t) => t + 1)}
          title={`Load the definition again (${fmtShortcut("Ctrl+R")} refreshes everything)`}
        >
          Refresh
        </button>
        <button
          className="btn"
          disabled={!ddl}
          onClick={() => {
            if (!ddl) return;
            void navigator.clipboard.writeText(ddl);
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1200);
          }}
        >
          {copied ? "Copied" : "Copy definition"}
        </button>
      </div>
      {error && <div className="notice error object-error">{error}</div>}
      {!error && ddl === null && <div className="object-loading muted">Loading definition…</div>}
      {ddl !== null && <pre className="ddl">{ddl}</pre>}
    </div>
  );
}
