import { comboParts } from "../lib/keys";

export function Kbd({ combo }: { combo: string }) {
  return (
    <span className="kbd-group">
      {comboParts(combo).map((part, i) => (
        <kbd key={i}>{part}</kbd>
      ))}
    </span>
  );
}
