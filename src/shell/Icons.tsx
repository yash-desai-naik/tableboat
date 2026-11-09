import { useId, type ReactNode } from "react";

// Tableboat's own 16 px outline icons (1.5 px stroke). Window caption glyphs are 10 px
// with 1 px strokes, drawn on whole pixels so they stay crisp at any scaling.

type IconProps = { size?: number };

function Svg({ size = 16, children }: IconProps & { children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

export function PanelLeftIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <rect x="2" y="3" width="12" height="10" rx="1.5" />
      <path d="M6 3v10" />
    </Svg>
  );
}

export function PanelRightIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <rect x="2" y="3" width="12" height="10" rx="1.5" />
      <path d="M10 3v10" />
    </Svg>
  );
}

export function CommandIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <rect x="2" y="3" width="12" height="10" rx="1.5" />
      <path d="M5 7l2 1.5L5 10M8.5 10.5H11" />
    </Svg>
  );
}

export function PlugIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M6 2v3M10 2v3M4.5 5h7v2.5a3.5 3.5 0 0 1-7 0zM8 11v3" />
    </Svg>
  );
}

export function PencilIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M10.5 2.5 13.5 5.5 5.5 13.5H2.5v-3z M9 4l3 3" />
    </Svg>
  );
}

/** A window with a "+": open something in a new window. */
export function NewWindowIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <rect x="1.5" y="3" width="10" height="9.5" rx="1.5" />
      <path d="M1.5 5.5h10M13.5 1.5v4M11.5 3.5h4" />
    </Svg>
  );
}

export function DatabaseIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <ellipse cx="8" cy="4" rx="5" ry="2" />
      <path d="M3 4v8c0 1.1 2.2 2 5 2s5-.9 5-2V4M3 8c0 1.1 2.2 2 5 2s5-.9 5-2" />
    </Svg>
  );
}

export function ChatIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M3 4.5A1.5 1.5 0 0 1 4.5 3h7A1.5 1.5 0 0 1 13 4.5v5A1.5 1.5 0 0 1 11.5 11H7l-3 2.5V11h.5" />
    </Svg>
  );
}

export function CloseSmallIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M4.5 4.5l7 7M11.5 4.5l-7 7" />
    </Svg>
  );
}

function Caption({ children }: { children: ReactNode }) {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth={1} aria-hidden="true">
      {children}
    </svg>
  );
}

export const MinimizeGlyph = () => (
  <Caption>
    <path d="M0 5.5h10" />
  </Caption>
);

export const MaximizeGlyph = () => (
  <Caption>
    <rect x="0.5" y="0.5" width="9" height="9" rx="1" />
  </Caption>
);

export const RestoreGlyph = () => (
  <Caption>
    <rect x="0.5" y="2.5" width="7" height="7" rx="1" />
    <path d="M2.5 2.5V1.5a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-1" />
  </Caption>
);

export const CloseGlyph = () => (
  <Caption>
    <path d="M0.5 0.5l9 9M9.5 0.5l-9 9" />
  </Caption>
);

/** The app mark (same drawing as src-tauri/app-icon.svg): a boat whose sail is a table. */
export function AppMark({ size = 16 }: IconProps) {
  const id = useId();
  return (
    <svg width={size} height={size} viewBox="0 0 1024 1024" aria-hidden="true">
      <defs>
        <linearGradient id={`${id}rows`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#f29a72" />
          <stop offset="1" stopColor="#cc5f36" />
        </linearGradient>
        <linearGradient id={`${id}hull`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#b4492a" />
          <stop offset="1" stopColor="#7f3015" />
        </linearGradient>
        <mask id={`${id}cut`}>
          <rect width="1024" height="1024" fill="#fff" />
          <rect x="372" y="160" width="34" height="540" fill="#000" />
        </mask>
      </defs>
      <rect x="176" y="24" width="48" height="712" rx="24" fill="#b04722" />
      <rect x="250" y="34" width="260" height="104" rx="26" fill="#b04722" />
      <g fill={`url(#${id}rows)`} mask={`url(#${id}cut)`}>
        <rect x="250" y="164" width="400" height="104" rx="26" />
        <rect x="250" y="294" width="530" height="104" rx="26" />
        <rect x="250" y="424" width="650" height="104" rx="26" />
        <rect x="250" y="554" width="752" height="104" rx="26" />
      </g>
      <path d="M22 700 H1002 L886 946 Q866 990 818 990 H206 Q158 990 138 946 Z" fill={`url(#${id}hull)`} />
    </svg>
  );
}

/* ---------- object kinds (sidebar, quick open, tabs) ---------- */

export function TableIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <rect x="2" y="3" width="12" height="10" rx="1.5" />
      <path d="M2 6.5h12M6.5 6.5V13" />
    </Svg>
  );
}

export function ViewIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <rect x="2" y="3" width="12" height="10" rx="1.5" strokeDasharray="2 1.6" />
      <path d="M2 6.5h12" />
    </Svg>
  );
}

export function FunctionIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M10.5 2.8c-1.6-.4-2.7.3-3 1.9L6 12c-.3 1.5-1.3 2-2.6 1.6M4.5 6.5h5" />
    </Svg>
  );
}

export function ProcedureIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <rect x="2" y="3" width="12" height="10" rx="1.5" />
      <path d="M6.5 6l3 2-3 2z" />
    </Svg>
  );
}

export function TriggerIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M9 2L4 9h4l-1 5 5-7H8z" />
    </Svg>
  );
}

export function ChevronIcon({ open, size = 12 }: { open: boolean; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      style={{ transform: open ? "rotate(90deg)" : undefined }}
    >
      <path d="M4.5 3l3 3-3 3" />
    </svg>
  );
}

export function RefreshIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M13 8a5 5 0 1 1-1.5-3.6M13 2.5v2.5h-2.5" />
    </Svg>
  );
}

/* ---------- toolbar actions ---------- */

export function PlusIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M8 3v10M3 8h10" />
    </Svg>
  );
}

export function DuplicateIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <rect x="5" y="5" width="8.5" height="8.5" rx="1.5" />
      <path d="M11 5V3.5A1.5 1.5 0 0 0 9.5 2h-6A1.5 1.5 0 0 0 2 3.5v6A1.5 1.5 0 0 0 3.5 11H5" />
    </Svg>
  );
}

export function TrashIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M2.5 4.5h11M6.5 4.5V3h3v1.5M4 4.5l.7 8.6a1 1 0 0 0 1 .9h4.6a1 1 0 0 0 1-.9l.7-8.6M6.7 7v4.5M9.3 7v4.5" />
    </Svg>
  );
}

export function CodeIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M5.5 4.5 2 8l3.5 3.5M10.5 4.5 14 8l-3.5 3.5M9 3 7 13" />
    </Svg>
  );
}

export function DownloadIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M8 2.5v8M4.5 7 8 10.5 11.5 7M3 13.5h10" />
    </Svg>
  );
}

export function SearchIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <circle cx="7" cy="7" r="4.5" />
      <path d="m10.5 10.5 3 3" />
    </Svg>
  );
}

export function ChevronLeftIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M10 3.5 5.5 8l4.5 4.5" />
    </Svg>
  );
}

export function ChevronRightIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M6 3.5 10.5 8 6 12.5" />
    </Svg>
  );
}

export function FilterIcon(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M2.5 3.5h11L9.5 8.5v4l-3 1.5V8.5z" />
    </Svg>
  );
}
