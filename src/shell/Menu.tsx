import { useEffect, useLayoutEffect, useRef, useState } from "react";

export type MenuItem =
  | { label: string; onSelect: () => void; hint?: string; danger?: boolean; disabled?: boolean }
  | "separator";

interface MenuProps {
  x: number;
  y: number;
  items: MenuItem[];
  onClose: () => void;
}

/** A small context menu at a screen position; closes on outside click, Esc or selection. */
export function Menu({ x, y, items, onClose }: MenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });
  const [active, setActive] = useState(-1);

  // Keep the menu inside the window.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      x: Math.min(x, window.innerWidth - r.width - 4),
      y: Math.min(y, window.innerHeight - r.height - 4),
    });
    el.focus();
  }, [x, y]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  const selectable = items
    .map((item, i) => ({ item, i }))
    .filter(({ item }) => item !== "separator" && !item.disabled);

  return (
    <div
      ref={ref}
      className="menu"
      role="menu"
      tabIndex={-1}
      style={{ left: pos.x, top: pos.y }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          onClose();
        } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
          e.preventDefault();
          const idx = selectable.findIndex((s) => s.i === active);
          const next = e.key === "ArrowDown" ? idx + 1 : idx - 1;
          const wrapped = (next + selectable.length) % selectable.length;
          setActive(selectable[wrapped]?.i ?? -1);
        } else if (e.key === "Enter") {
          const item = items[active];
          if (item && item !== "separator" && !item.disabled) {
            e.preventDefault();
            onClose();
            item.onSelect();
          }
        }
      }}
    >
      {items.map((item, i) =>
        item === "separator" ? (
          <div key={i} className="menu-separator" role="separator" />
        ) : (
          <button
            key={i}
            role="menuitem"
            className={`menu-item${item.danger ? " danger" : ""}${active === i ? " active" : ""}`}
            disabled={item.disabled}
            onMouseEnter={() => setActive(i)}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
          >
            <span>{item.label}</span>
            {item.hint && <span className="menu-hint">{item.hint}</span>}
          </button>
        ),
      )}
    </div>
  );
}
