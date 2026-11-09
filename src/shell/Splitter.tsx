import { useRef, useState } from "react";

interface SplitterProps {
  /** Which side of the splitter the resized pane is on. */
  pane: "left" | "right";
  width: number;
  min: number;
  max: number;
  onResize: (width: number) => void;
  onReset: () => void;
  label: string;
}

/** A 1 px divider with a wider invisible grab area. Double-click resets the width. */
export function Splitter({ pane, width, min, max, onResize, onReset, label }: SplitterProps) {
  const start = useRef<{ x: number; width: number } | null>(null);
  const [dragging, setDragging] = useState(false);

  return (
    <div
      className={dragging ? "splitter dragging" : "splitter"}
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = { x: e.clientX, width };
        setDragging(true);
        document.body.classList.add("resizing");
      }}
      onPointerMove={(e) => {
        if (!start.current) return;
        const dx = e.clientX - start.current.x;
        const next = start.current.width + (pane === "left" ? dx : -dx);
        onResize(Math.round(Math.min(max, Math.max(min, next))));
      }}
      onPointerUp={(e) => {
        e.currentTarget.releasePointerCapture(e.pointerId);
        start.current = null;
        setDragging(false);
        document.body.classList.remove("resizing");
      }}
      onDoubleClick={onReset}
    />
  );
}
