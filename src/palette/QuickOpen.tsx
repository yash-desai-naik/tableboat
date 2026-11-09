import { useEffect, useRef, useState } from "react";
import { Command as Cmdk } from "cmdk";
import { errorText } from "../lib/connections";
import { metaApi, OBJECT_KINDS, objectKey, type Hit } from "../lib/meta";
import { openObject, setDatabase, useWorkspace } from "../lib/workspace";
import { Highlighted, KindIcon } from "../shell/KindIcon";

interface Props {
  sessionId: string;
  onClose: () => void;
}

/**
 * Ctrl+P: fuzzy search over every table, view, function, procedure and trigger in every
 * database of the connection. `^abc` start, `abc$` end, `'abc` exact substring.
 */
export function QuickOpen({ sessionId, onClose }: Props) {
  const ws = useWorkspace();
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<Hit[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const newTab = useRef(false);

  useEffect(() => {
    let stale = false;
    // An empty query lists the current database first; typing searches everything.
    const scope = query.trim() ? null : ws.database;
    metaApi
      .search(sessionId, query, scope, "score", 60)
      .then((result) => {
        if (stale) return;
        setHits(result);
        setError(null);
      })
      .catch((err) => !stale && setError(errorText(err)))
      .finally(() => !stale && setLoading(false));
    return () => {
      stale = true;
    };
  }, [query, sessionId, ws.database]);

  const kindLabel = (k: Hit["kind"]) => OBJECT_KINDS.find((o) => o.id === k)?.label ?? k;

  return (
    <div className="overlay" onMouseDown={onClose}>
      <div className="palette" onMouseDown={(e) => e.stopPropagation()}>
        <Cmdk label="Go to object" shouldFilter={false} loop>
          <Cmdk.Input
            autoFocus
            value={query}
            onValueChange={setQuery}
            placeholder="Go to table, view, function or trigger…"
            onKeyDown={(e) => {
              newTab.current = e.ctrlKey;
              if (e.key === "Escape") {
                e.preventDefault();
                if (query) setQuery("");
                else onClose();
              }
            }}
          />
          <Cmdk.List>
            {error && <div className="palette-note error">{error}</div>}
            {!error && !loading && hits.length === 0 && <Cmdk.Empty>No objects match.</Cmdk.Empty>}
            {hits.map((hit) => (
              <Cmdk.Item
                key={objectKey(hit)}
                value={objectKey(hit)}
                onSelect={() => {
                  onClose();
                  setDatabase(hit.database);
                  openObject(hit, newTab.current);
                  newTab.current = false;
                }}
              >
                <KindIcon kind={hit.kind} />
                <span className="palette-title mono">
                  <Highlighted text={hit.name} indices={hit.indices} />
                </span>
                <span className="palette-meta">{hit.database}</span>
                <span className="palette-kind">{kindLabel(hit.kind)}</span>
              </Cmdk.Item>
            ))}
          </Cmdk.List>
          <div className="palette-footer">
            <span>
              <kbd>↵</kbd> Open
            </span>
            <span>
              <kbd>Ctrl</kbd>
              <kbd>↵</kbd> New tab
            </span>
            <span>
              <kbd>^</kbd>start <kbd>$</kbd>end <kbd>'</kbd>exact
            </span>
            <span>
              <kbd>Esc</kbd> Close
            </span>
          </div>
        </Cmdk>
      </div>
    </div>
  );
}
