import { describeTarget, displayName, type SessionInfo } from "../lib/connections";
import { disconnect } from "../lib/session";
import { uiActions } from "../lib/ui";
import { newQueryTab } from "../lib/workspace";
import { Kbd } from "../shell/Kbd";

/** What the main area shows when no tab is open: connection details and shortcuts. */
export function ConnectedView({ info }: { info: SessionInfo }) {
  const { connection: c, server } = info;
  const rows: [string, string][] = [
    ["Server", `${server.product} ${server.fullVersion}`],
    [c.engine === "sqlite" ? "File" : "Address", describeTarget(c)],
    ["Database", server.database ?? "none selected"],
    ["Encryption", c.engine === "sqlite" ? "local file" : server.tls ? "TLS" : "not encrypted"],
  ];
  if (server.ssh) rows.push(["Tunnel", `SSH via ${c.ssh.user}@${c.ssh.host}:${c.ssh.port}`]);

  return (
    <div className="connected">
      <div className="connected-card">
        <div className="connected-head">
          <span className="conn-dot" style={{ background: c.color }} />
          <h2>{displayName(c)}</h2>
        </div>
        <table className="kv">
          <tbody>
            {rows.map(([k, v]) => (
              <tr key={k}>
                <th>{k}</th>
                <td>{v}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <table className="kv shortcuts">
          <tbody>
            {(
              [
                ["New SQL tab", "Ctrl+T"],
                ["Go to table, view or routine", "Ctrl+P"],
                ["Switch database", "Ctrl+K"],
                ["Open or create a connection", "Ctrl+Shift+K"],
                ["All commands", "Ctrl+Shift+P"],
              ] as const
            ).map(([label, combo]) => (
              <tr key={label}>
                <th>{label}</th>
                <td>
                  <Kbd combo={combo} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="connected-actions">
          <button className="btn" onClick={() => void disconnect()}>
            Disconnect
          </button>
          <button className="btn" onClick={() => uiActions.openConnections("list")}>
            Switch connection…
          </button>
          <button className="btn primary" onClick={() => newQueryTab()}>
            New SQL tab
          </button>
        </div>
      </div>
    </div>
  );
}
