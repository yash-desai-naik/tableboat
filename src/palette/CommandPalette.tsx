import { Command as Cmdk } from "cmdk";
import type { Command } from "../lib/commands";
import { Kbd } from "../shell/Kbd";

interface Props {
  commands: Command[];
  onClose: () => void;
}

/** Ctrl+Shift+P: fuzzy list of every command, grouped by category. */
export function CommandPalette({ commands, onClose }: Props) {
  const groups = new Map<string, Command[]>();
  for (const cmd of commands) {
    if (cmd.hidden) continue;
    const list = groups.get(cmd.category) ?? [];
    list.push(cmd);
    groups.set(cmd.category, list);
  }

  return (
    <div className="overlay" onMouseDown={onClose}>
      <div className="palette" onMouseDown={(e) => e.stopPropagation()}>
        <Cmdk label="Command palette" loop>
          <Cmdk.Input
            autoFocus
            placeholder="Type a command…"
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                onClose();
              }
            }}
          />
          <Cmdk.List>
            <Cmdk.Empty>No matching commands</Cmdk.Empty>
            {[...groups].map(([category, items]) => (
              <Cmdk.Group heading={category} key={category}>
                {items.map((cmd) => (
                  <Cmdk.Item
                    key={cmd.id}
                    value={`${cmd.category}: ${cmd.title}`}
                    onSelect={() => {
                      onClose();
                      cmd.run();
                    }}
                  >
                    <span className="palette-check" aria-hidden="true">
                      {cmd.checked ? "✓" : ""}
                    </span>
                    <span className="palette-title">
                      <span className="palette-category">{cmd.category}: </span>
                      {cmd.title}
                    </span>
                    {cmd.keys?.[0] && <Kbd combo={cmd.keys[0]} />}
                  </Cmdk.Item>
                ))}
              </Cmdk.Group>
            ))}
          </Cmdk.List>
          <div className="palette-footer">
            <span>
              <kbd>↵</kbd> Run
            </span>
            <span>
              <kbd>↑</kbd>
              <kbd>↓</kbd> Move
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
