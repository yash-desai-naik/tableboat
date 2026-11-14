import { useEffect, useRef, type CSSProperties, type KeyboardEvent } from "react";
import { previewTheme, themesFor, pickTheme, type ThemeData, type ThemeMode } from "../themes";
import { updateSettings } from "../lib/settings";
import "./ThemePicker.css";

/*
 * Live preview, shared by both pickers: the card under the mouse wins, else the card focused from the
 * keyboard, else nothing (the saved theme). Only one element can be hovered or focused at a time, so
 * one module-level state is enough even with two pickers on the page.
 */
let hovered: string | null = null;
let focused: string | null = null;
let previewing: string | null = null;

function syncPreview() {
  const want = hovered ?? focused;
  if (want === previewing) return;
  previewing = want;
  previewTheme(want);
}

function setHovered(id: string | null) {
  hovered = id;
  syncPreview();
}

function setFocused(id: string | null) {
  focused = id;
  syncPreview();
}

/** A row of Settings → Appearance: every theme of one mode as a card; click (or Enter/Space) chooses. */
export function ThemePicker(props: { mode: ThemeMode; chosen: string; label: string; help?: string; usedNow: boolean }) {
  const { mode, label, help, usedNow } = props;
  const themes = themesFor(mode);
  const selected = pickTheme(mode, props.chosen).id;
  const groupRef = useRef<HTMLDivElement>(null);
  const headId = `theme-picker-${mode}`;

  // Leaving Settings while a card is previewed restores the saved theme.
  useEffect(
    () => () => {
      hovered = null;
      focused = null;
      syncPreview();
    },
    [],
  );

  const choose = (id: string) => updateSettings(mode === "light" ? { lightTheme: id } : { darkTheme: id });

  const cards = () => Array.from(groupRef.current?.querySelectorAll<HTMLButtonElement>(".theme-card") ?? []);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const all = cards();
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    if (at < 0) return;
    // Cards wrap; a row is the cards that share the first card's top edge.
    const perRow = Math.max(1, all.filter((c) => c.offsetTop === all[0].offsetTop).length);
    let next = at;
    switch (e.key) {
      case "ArrowRight":
        next = at + 1;
        break;
      case "ArrowLeft":
        next = at - 1;
        break;
      case "ArrowDown":
        next = at + perRow < all.length ? at + perRow : at;
        break;
      case "ArrowUp":
        next = at - perRow >= 0 ? at - perRow : at;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = all.length - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    all[(next + all.length) % all.length]?.focus();
  };

  return (
    <div className="theme-picker">
      <div className="theme-picker-head">
        <span id={headId}>{label}</span>
        {usedNow && <span className="theme-used-now">Used now</span>}
        {help && <div className="muted small">{help}</div>}
      </div>
      <div
        ref={groupRef}
        className="theme-grid"
        role="radiogroup"
        aria-labelledby={headId}
        onKeyDown={onKeyDown}
        onMouseLeave={() => setHovered(null)}
      >
        {themes.map((t) => {
          const isSelected = t.id === selected;
          return (
            <button
              key={t.id}
              type="button"
              role="radio"
              aria-checked={isSelected}
              tabIndex={isSelected ? 0 : -1}
              className={`theme-card${isSelected ? " selected" : ""}`}
              onClick={() => choose(t.id)}
              onMouseEnter={() => setHovered(t.id)}
              // Preview on keyboard focus only: a mouse click focuses too, and the preview
              // would otherwise stay after the pointer leaves.
              onFocus={(e) => setFocused(e.currentTarget.matches(":focus-visible") ? t.id : null)}
              // Moving between cards: the next card's focus replaces the preview (no flash back).
              onBlur={(e) => {
                if (!groupRef.current?.contains(e.relatedTarget as Node | null)) setFocused(null);
              }}
            >
              <Miniature theme={t} />
              <span className="theme-card-label">
                <span className="theme-card-name">{t.name}</span>
                {isSelected && (
                  <svg className="theme-card-check" viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                    <path d="M3 8.5l3 3 7-7" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                )}
              </span>
              {(t.id.startsWith("tableboat-") || t.highContrast) && (
                <span className="theme-card-tags">
                  {t.id.startsWith("tableboat-") && <span className="theme-tag">default</span>}
                  {t.highContrast && <span className="theme-tag">high contrast</span>}
                </span>
              )}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/**
 * A tiny Tableboat drawn in the theme's own colours (title bar, sidebar, tab, SQL, a selected row,
 * a button). The colours are passed as CSS variables on the root; ThemePicker.css lays it out.
 */
function Miniature({ theme: t }: { theme: ThemeData }) {
  const selection = t.selection ?? `color-mix(in srgb, ${t.accent.fill} 30%, ${t.surface.bg})`;
  const vars = {
    "--m-bg": t.surface.bg,
    "--m-chrome": t.surface.chrome,
    "--m-sidebar": t.surface.sidebar,
    "--m-text": t.text.primary,
    "--m-text-2": t.text.secondary,
    "--m-hint": t.text.hint,
    "--m-accent": t.accent.text,
    "--m-fill": t.accent.fill,
    "--m-on": t.accent.on,
    "--m-sel": selection,
    "--m-kw": t.syntax.keyword,
    "--m-str": t.syntax.string,
    "--m-num": t.syntax.number,
    "--m-com": t.syntax.comment,
    "--m-table": t.kind.table,
    "--m-view": t.kind.view,
  } as CSSProperties;
  return (
    <span className="mini" style={vars} aria-hidden="true">
      <span className="mini-title">
        <span className="mini-dot" />
      </span>
      <span className="mini-body">
        <span className="mini-side">
          <span className="mini-row">
            <span className="mini-icon" style={{ background: "var(--m-table)" }} />
            <span className="mini-bar" style={{ width: "60%" }} />
          </span>
          <span className="mini-row selected">
            <span className="mini-icon" style={{ background: "var(--m-table)" }} />
            <span className="mini-bar" style={{ width: "48%", background: "var(--m-text)" }} />
          </span>
          <span className="mini-row">
            <span className="mini-icon" style={{ background: "var(--m-view)" }} />
            <span className="mini-bar" style={{ width: "54%" }} />
          </span>
        </span>
        <span className="mini-main">
          <span className="mini-tabs">
            <span className="mini-tab">
              <span className="mini-bar" style={{ width: "100%", background: "var(--m-text)" }} />
            </span>
          </span>
          <span className="mini-code">
            <span className="mini-line">
              <span className="mini-tok" style={{ width: 14, background: "var(--m-kw)" }} />
              <span className="mini-tok" style={{ width: 6, background: "var(--m-text)" }} />
              <span className="mini-tok" style={{ width: 10, background: "var(--m-kw)" }} />
              <span className="mini-tok" style={{ width: 12, background: "var(--m-text)" }} />
            </span>
            <span className="mini-line">
              <span className="mini-tok" style={{ width: 12, background: "var(--m-kw)" }} />
              <span className="mini-tok" style={{ width: 18, background: "var(--m-str)" }} />
              <span className="mini-tok" style={{ width: 6, background: "var(--m-num)" }} />
            </span>
            <span className="mini-line">
              <span className="mini-tok" style={{ width: 30, background: "var(--m-com)" }} />
            </span>
          </span>
          <span className="mini-grid">
            <span className="mini-cell-row">
              <span className="mini-tok" style={{ width: 8, background: "var(--m-text-2)" }} />
              <span className="mini-tok" style={{ width: 16, background: "var(--m-text-2)" }} />
            </span>
            <span className="mini-cell-row selected">
              <span className="mini-tok" style={{ width: 8, background: "var(--m-text)" }} />
              <span className="mini-tok" style={{ width: 16, background: "var(--m-text)" }} />
            </span>
            <span className="mini-btn">
              <span className="mini-tok" style={{ width: 10, background: "var(--m-on)" }} />
            </span>
          </span>
        </span>
      </span>
    </span>
  );
}
