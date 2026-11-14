import { useState, type ReactNode } from "react";
import { closeNotice, installUpdate, openReleasesPage, skipUpdate, useUpdate, type UpdateInfo } from "../lib/update";
import { Dialog } from "./Dialog";

/** A small card in the corner: a new version, its download, or the result of "Check for updates". */
export function UpdateNotice() {
  const u = useUpdate();
  const [notes, setNotes] = useState(false);
  const mb = (n: number) => (n / 1048576).toFixed(1);

  let body: ReactNode = null;
  if (u.phase === "checking") {
    body = <div className="update-title">Checking for updates…</div>;
  } else if (u.phase === "uptodate") {
    body = (
      <>
        <div className="update-title">You're up to date</div>
        <div className="update-sub">This is the newest version of Tableboat.</div>
      </>
    );
  } else if (u.phase === "downloading" && u.info) {
    const pct = u.total ? Math.min(100, (u.downloaded / u.total) * 100) : null;
    body = (
      <>
        <div className="update-title">Downloading Tableboat {u.info.version}…</div>
        <div className="update-bar" role="progressbar" aria-valuenow={pct ?? undefined} aria-valuemin={0} aria-valuemax={100}>
          <span style={{ width: `${pct ?? 30}%` }} className={pct === null ? "indeterminate" : undefined} />
        </div>
        <div className="update-sub">{u.total ? `${mb(u.downloaded)} of ${mb(u.total)} MB` : `${mb(u.downloaded)} MB`}</div>
      </>
    );
  } else if (u.phase === "installing" && u.info) {
    body = (
      <>
        <div className="update-title">Installing Tableboat {u.info.version}…</div>
        <div className="update-sub">Tableboat closes and opens again on the new version. Your tabs are saved.</div>
      </>
    );
  } else if (u.phase === "error") {
    body = (
      <>
        <div className="update-title">Update failed</div>
        <div className="update-sub update-error">{u.error}</div>
        <div className="update-actions">
          <button className="btn small-btn" onClick={openReleasesPage}>
            Download from GitHub
          </button>
          <button className="btn small-btn" onClick={closeNotice}>
            Close
          </button>
        </div>
      </>
    );
  } else if (u.info) {
    body = (
      <>
        <div className="update-title">Tableboat {u.info.version} is available</div>
        <div className="update-sub">You have {u.info.current}.</div>
        <div className="update-actions">
          <button className="btn primary small-btn" onClick={() => void installUpdate()}>
            Install and restart
          </button>
          <button className="btn small-btn" onClick={() => setNotes(true)}>
            What's new
          </button>
          <button className="btn small-btn" onClick={closeNotice} title="Remind me next time">
            Later
          </button>
        </div>
        <button className="link-btn update-skip" onClick={skipUpdate}>
          Skip this version
        </button>
      </>
    );
  }

  const busy = u.phase === "downloading" || u.phase === "installing";
  return (
    <>
      {body && (u.noticeOpen || busy) && (
        <div className="update-notice" role="status" aria-live="polite">
          {!busy && (
            <button className="icon-btn update-close" onClick={closeNotice} title="Close" aria-label="Close">
              ×
            </button>
          )}
          {body}
        </div>
      )}
      {notes && u.info && <WhatsNew info={u.info} onClose={() => setNotes(false)} />}
    </>
  );
}

/** The release notes of an update, with Install. */
export function WhatsNew({ info, onClose }: { info: UpdateInfo; onClose: () => void }) {
  return (
    <Dialog
      title={`What's new in Tableboat ${info.version}`}
      onClose={onClose}
      width={640}
      footer={
        <>
          <button className="link-btn" onClick={openReleasesPage}>
            Release page on GitHub
          </button>
          <span className="spacer" />
          <button className="btn" onClick={onClose}>
            Close
          </button>
          <button
            className="btn primary"
            onClick={() => {
              onClose();
              void installUpdate();
            }}
          >
            Install and restart
          </button>
        </>
      }
    >
      <div className="release-notes">
        <ReleaseNotes text={info.notes} />
      </div>
    </Dialog>
  );
}

/**
 * Release notes Markdown (CHANGELOG.md's section plus the download footer, which isn't
 * needed here): headings, lists (wrapped lines joined), bold, code and links (as text).
 */
function ReleaseNotes({ text }: { text: string }) {
  // The footer (download table, checksum…) is for the web page, not the app.
  const notes = text.split(/\n---\s*\n/)[0].replace(/\r\n/g, "\n");
  type Block = { kind: "h" | "p" | "li"; depth: number; text: string };
  const blocks: Block[] = [];
  for (const raw of notes.split("\n")) {
    const line = raw.trimEnd();
    const h = /^#{1,6}\s+(.*)$/.exec(line);
    const li = /^(\s*)[-*]\s+(.*)$/.exec(line);
    const last = blocks[blocks.length - 1];
    if (!line.trim()) blocks.push({ kind: "p", depth: 0, text: "" });
    else if (h) blocks.push({ kind: "h", depth: 0, text: h[1] });
    else if (li) blocks.push({ kind: "li", depth: Math.floor(li[1].length / 2), text: li[2] });
    else if (last && last.text && last.kind !== "h") last.text += ` ${line.trim()}`;
    else blocks.push({ kind: "p", depth: 0, text: line.trim() });
  }
  return (
    <>
      {blocks.map((b, i) =>
        b.kind === "h" ? (
          <div key={i} className="rn-h">
            {inline(b.text)}
          </div>
        ) : b.kind === "li" ? (
          <div key={i} className="rn-li" style={{ paddingLeft: 14 + b.depth * 16 }}>
            <span className="rn-mark">•</span>
            <span>{inline(b.text)}</span>
          </div>
        ) : b.text ? (
          <p key={i}>{inline(b.text)}</p>
        ) : null,
      )}
    </>
  );
}

function inline(s: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\([^)]+\))/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let k = 0;
  while ((m = re.exec(s))) {
    if (m.index > last) out.push(s.slice(last, m.index));
    const t = m[0];
    if (t.startsWith("`")) out.push(<code key={k++}>{t.slice(1, -1)}</code>);
    else if (t.startsWith("**")) out.push(<strong key={k++}>{inline(t.slice(2, -2))}</strong>);
    else out.push(<span key={k++}>{/^\[([^\]]+)\]/.exec(t)?.[1]}</span>);
    last = re.lastIndex;
  }
  if (last < s.length) out.push(s.slice(last));
  return out;
}
