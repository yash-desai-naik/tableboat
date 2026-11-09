import { useEffect, useState } from "react";
import { Command as Cmdk } from "cmdk";
import { errorText } from "../lib/connections";
import { metaApi } from "../lib/meta";
import { setDatabase, useWorkspace } from "../lib/workspace";
import { DatabaseIcon } from "../shell/Icons";

const SYSTEM = new Set(["information_schema", "mysql", "performance_schema", "sys"]);

/** Ctrl+K: switch the database shown in the sidebar. */
export function DatabasePicker({ sessionId, onClose }: { sessionId: string; onClose: () => void }) {
  const ws = useWorkspace();
  const [databases, setDatabases] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    metaApi
      // Always fresh, so databases created elsewhere appear.
      .databases(sessionId, true)
      .then(setDatabases)
      .catch((err) => setError(errorText(err)));
  }, [sessionId]);

  return (
    <div className="overlay" onMouseDown={onClose}>
      <div className="palette narrow" onMouseDown={(e) => e.stopPropagation()}>
        <Cmdk label="Switch database" loop defaultValue={ws.database ?? undefined}>
          <Cmdk.Input
            autoFocus
            placeholder="Switch database…"
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                onClose();
              }
            }}
          />
          <Cmdk.List>
            {error && <div className="palette-note error">{error}</div>}
            {databases === null && !error && <div className="palette-note">Loading…</div>}
            <Cmdk.Empty>No database matches.</Cmdk.Empty>
            {databases?.map((db) => (
              <Cmdk.Item
                key={db}
                value={db}
                onSelect={() => {
                  setDatabase(db);
                  onClose();
                }}
              >
                <span className="palette-check" aria-hidden="true">
                  {db === ws.database ? "✓" : ""}
                </span>
                <DatabaseIcon size={14} />
                <span className="palette-title mono">{db}</span>
                {SYSTEM.has(db.toLowerCase()) && <span className="palette-kind">system</span>}
              </Cmdk.Item>
            ))}
          </Cmdk.List>
          <div className="palette-footer">
            <span>
              <kbd>↵</kbd> Switch
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
