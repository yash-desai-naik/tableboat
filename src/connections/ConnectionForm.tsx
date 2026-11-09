import { useState, type ReactNode } from "react";
import { open as openFile, save as saveFile } from "@tauri-apps/plugin-dialog";
import {
  api,
  COLOR_PRESETS,
  ENGINES,
  engineInfo,
  errorText,
  TAGS,
  type Connection,
  type ConnectionSummary,
  type Engine,
  type Secrets,
  type ServerInfo,
  type SshAuth,
  type SslMode,
} from "../lib/connections";
import { Dialog } from "../shell/Dialog";

interface StoredFlags {
  hasPassword: boolean;
  hasSshPassword: boolean;
  hasSshPassphrase: boolean;
}

interface Props {
  title: string;
  initial: Connection;
  /** Secrets that came with the form (e.g. from an imported URL); they count as typed. */
  initialSecrets?: Secrets;
  stored?: StoredFlags;
  onClose: () => void;
  /** `connectPassword` is set when the user pressed Connect (password for "ask every time"). */
  onSaved: (saved: ConnectionSummary, connect: { password?: string } | null) => void;
}

/** A secret field: untouched (keep stored value), typed, or cleared. */
interface SecretField {
  value: string;
  dirty: boolean;
}

const SSL_MODES: { id: SslMode; label: string }[] = [
  { id: "preferred", label: "Preferred – use TLS if the server offers it" },
  { id: "disabled", label: "Disabled" },
  { id: "required", label: "Required – TLS, don't check the certificate" },
  { id: "verify-ca", label: "Verify CA – check the certificate" },
  { id: "verify-identity", label: "Verify identity – check certificate and host name" },
];

export function ConnectionForm({ title, initial, initialSecrets, stored, onClose, onSaved }: Props) {
  const [conn, setConn] = useState<Connection>(initial);
  const [password, setPassword] = useState<SecretField>(secretFrom(initialSecrets?.password));
  const [sshPassword, setSshPassword] = useState<SecretField>(secretFrom(initialSecrets?.sshPassword));
  const [passphrase, setPassphrase] = useState<SecretField>(secretFrom(initialSecrets?.sshPassphrase));
  const [busy, setBusy] = useState<null | "test" | "save" | "connect">(null);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [urlInput, setUrlInput] = useState("");

  const isSqlite = conn.engine === "sqlite";
  const engine = engineInfo(conn.engine);

  const update = (patch: Partial<Connection>) => {
    setConn((c) => ({ ...c, ...patch }));
    setResult(null);
  };
  const updateSsl = (patch: Partial<Connection["ssl"]>) => update({ ssl: { ...conn.ssl, ...patch } });
  const updateSsh = (patch: Partial<Connection["ssh"]>) => update({ ssh: { ...conn.ssh, ...patch } });

  const secrets = (): Secrets => ({
    password: password.dirty ? password.value : undefined,
    sshPassword: sshPassword.dirty ? sshPassword.value : undefined,
    sshPassphrase: passphrase.dirty ? passphrase.value : undefined,
  });

  async function importUrl() {
    if (!urlInput.trim()) return;
    try {
      const parsed = await api.parseUrl(urlInput);
      setConn({ ...parsed.connection, id: conn.id, createdAt: conn.createdAt });
      setPassword(secretFrom(parsed.secrets.password));
      setSshPassword(secretFrom(parsed.secrets.sshPassword));
      setPassphrase(secretFrom(parsed.secrets.sshPassphrase));
      setUrlInput("");
      setResult({ ok: true, text: "Filled in from the URL. Check the details, then Test or Save." });
    } catch (err) {
      setResult({ ok: false, text: errorText(err) });
    }
  }

  async function test() {
    setBusy("test");
    setResult(null);
    try {
      const info = await api.test(conn, secrets());
      setResult({ ok: true, text: describeServer(info) });
    } catch (err) {
      setResult({ ok: false, text: errorText(err) });
    } finally {
      setBusy(null);
    }
  }

  async function save(connect: boolean) {
    setBusy(connect ? "connect" : "save");
    setResult(null);
    try {
      const saved = await api.save(conn, secrets());
      const typedPassword = password.dirty && password.value ? password.value : undefined;
      onSaved(saved, connect ? { password: conn.savePassword ? undefined : typedPassword } : null);
    } catch (err) {
      setResult({ ok: false, text: errorText(err) });
      setBusy(null);
    }
  }

  async function browseSqlite() {
    const picked = await openFile({
      title: "Open SQLite database",
      multiple: false,
      directory: false,
      filters: [
        { name: "SQLite database", extensions: ["sqlite", "sqlite3", "db", "db3", "s3db", "sl3"] },
        { name: "All files", extensions: ["*"] },
      ],
    });
    if (typeof picked === "string") update({ file: picked, name: conn.name || fileName(picked) });
  }

  async function createSqlite() {
    const target = await saveFile({
      title: "Create SQLite database",
      defaultPath: "database.sqlite",
      filters: [{ name: "SQLite database", extensions: ["sqlite", "db"] }],
    });
    if (!target) return;
    try {
      await api.createSqliteFile(target);
      update({ file: target, name: conn.name || fileName(target) });
      setResult({ ok: true, text: `Created ${target}` });
    } catch (err) {
      setResult({ ok: false, text: errorText(err) });
    }
  }

  async function pickFile(titleText: string, apply: (path: string) => void) {
    const picked = await openFile({ title: titleText, multiple: false, directory: false });
    if (typeof picked === "string") apply(picked);
  }

  const footer = (
    <>
      <button className="btn" onClick={test} disabled={busy !== null || !engine.ready}>
        {busy === "test" ? "Testing…" : "Test"}
      </button>
      <span className="spacer" />
      <button className="btn" onClick={onClose} disabled={busy === "save" || busy === "connect"}>
        Cancel
      </button>
      <button className="btn" onClick={() => save(false)} disabled={busy !== null}>
        {busy === "save" ? "Saving…" : "Save"}
      </button>
      <button className="btn primary" onClick={() => save(true)} disabled={busy !== null || !engine.ready}>
        {busy === "connect" ? "Connecting…" : "Connect"}
      </button>
    </>
  );

  return (
    <Dialog title={title} onClose={onClose} footer={footer} width={600}>
      <div
        className="conn-form"
        onKeyDown={(e) => {
          if (e.key === "Enter" && e.ctrlKey) {
            e.preventDefault();
            void save(true);
          }
        }}
      >
        <div className="url-import">
          <input
            className="input mono"
            placeholder="Paste a connection URL, e.g. mysql://user:pass@host:3306/db"
            value={urlInput}
            onChange={(e) => setUrlInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.ctrlKey) {
                e.preventDefault();
                void importUrl();
              }
            }}
            spellCheck={false}
          />
          <button className="btn" onClick={importUrl} disabled={!urlInput.trim()}>
            Fill in
          </button>
        </div>

        <div className="segmented" role="radiogroup" aria-label="Database type">
          {ENGINES.map((e) => (
            <button
              key={e.id}
              role="radio"
              aria-checked={conn.engine === e.id}
              className={conn.engine === e.id ? "seg active" : "seg"}
              title={e.ready ? e.label : `${e.label} support arrives in a later step`}
              onClick={() =>
                update({
                  engine: e.id as Engine,
                  // "root" is a MySQL habit; MongoDB usually has no default user.
                  user: e.id === "mongodb" && conn.user === "root" ? "" : conn.user,
                })
              }
            >
              {e.label}
              {!e.ready && <span className="soon">soon</span>}
            </button>
          ))}
        </div>

        <Section title="Identity">
          <Row label="Nickname">
            <input
              className="input"
              value={conn.name}
              placeholder={isSqlite ? "e.g. Local app data" : "e.g. Shop production"}
              onChange={(e) => update({ name: e.target.value })}
            />
          </Row>
          <Row label="Colour">
            <div className="swatches">
              {COLOR_PRESETS.map((c) => (
                <button
                  key={c}
                  className={conn.color.toLowerCase() === c ? "swatch selected" : "swatch"}
                  style={{ background: c }}
                  title={c}
                  aria-label={`Colour ${c}`}
                  onClick={() => update({ color: c })}
                />
              ))}
              <label className="swatch custom" title="Custom colour">
                <input type="color" value={conn.color} onChange={(e) => update({ color: e.target.value })} />
              </label>
            </div>
          </Row>
          <Row label="Tag">
            <select
              className="input"
              value={conn.tag ?? ""}
              onChange={(e) => update({ tag: e.target.value || null })}
            >
              <option value="">No tag</option>
              {TAGS.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.label}
                </option>
              ))}
              {conn.tag && !TAGS.some((t) => t.id === conn.tag) && <option value={conn.tag}>{conn.tag}</option>}
            </select>
          </Row>
        </Section>

        {isSqlite ? (
          <Section title="Database file">
            <Row label="File">
              <div className="inline">
                <input
                  className="input mono"
                  value={conn.file ?? ""}
                  placeholder="C:\path\to\database.sqlite"
                  onChange={(e) => update({ file: e.target.value })}
                  spellCheck={false}
                />
                <button className="btn" onClick={browseSqlite}>
                  Browse…
                </button>
              </div>
            </Row>
            <Row label="">
              <button className="link-btn" onClick={createSqlite}>
                Create a new database file…
              </button>
            </Row>
          </Section>
        ) : (
          <>
            <Section title="Server">
              <Row label="Host">
                <div className="inline">
                  <input
                    className="input mono"
                    value={conn.host}
                    onChange={(e) => update({ host: e.target.value })}
                    spellCheck={false}
                  />
                  <input
                    className="input mono port"
                    value={conn.port ?? ""}
                    placeholder={String(engine.defaultPort ?? "")}
                    inputMode="numeric"
                    aria-label="Port"
                    onChange={(e) => {
                      const digits = e.target.value.replace(/\D/g, "").slice(0, 5);
                      update({ port: digits ? Number(digits) : null });
                    }}
                  />
                </div>
              </Row>
              <Row label="User">
                <input
                  className="input mono"
                  value={conn.user}
                  onChange={(e) => update({ user: e.target.value })}
                  spellCheck={false}
                  autoComplete="off"
                />
              </Row>
              <Row label="Password">
                <SecretInput
                  field={password}
                  storedLabel={stored?.hasPassword ? "Saved in Windows Credential Manager" : undefined}
                  onChange={(f) => {
                    setPassword(f);
                    setResult(null);
                  }}
                />
              </Row>
              <Row label="">
                <label className="check">
                  <input
                    type="checkbox"
                    checked={!conn.savePassword}
                    onChange={(e) => update({ savePassword: !e.target.checked })}
                  />
                  Ask for the password every time
                </label>
              </Row>
              <Row label="Database">
                <input
                  className="input mono"
                  value={conn.database}
                  placeholder="optional"
                  onChange={(e) => update({ database: e.target.value })}
                  spellCheck={false}
                />
              </Row>
              {conn.engine === "mongodb" && (
                <>
                  <Row label="Auth database">
                    <input
                      className="input mono"
                      value={conn.options.authSource ?? ""}
                      placeholder="admin"
                      onChange={(e) => {
                        const options = { ...conn.options };
                        if (e.target.value) options.authSource = e.target.value;
                        else delete options.authSource;
                        update({ options });
                      }}
                      spellCheck={false}
                    />
                  </Row>
                  <Row label="">
                    <label className="check">
                      <input
                        type="checkbox"
                        checked={conn.options.srv === "true"}
                        onChange={(e) => {
                          const options = { ...conn.options };
                          if (e.target.checked) options.srv = "true";
                          else delete options.srv;
                          update({ options });
                        }}
                      />
                      DNS seed list (mongodb+srv, e.g. Atlas) – no port needed
                    </label>
                  </Row>
                </>
              )}
            </Section>

            <Section title="TLS / SSL">
              <Row label="Mode">
                <select
                  className="input"
                  value={conn.ssl.mode}
                  onChange={(e) => updateSsl({ mode: e.target.value as SslMode })}
                >
                  {SSL_MODES.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.label}
                    </option>
                  ))}
                </select>
              </Row>
              {conn.ssl.mode !== "disabled" && (
                <>
                  <FileRow
                    label="CA certificate"
                    value={conn.ssl.caFile}
                    onPick={() => pickFile("CA certificate", (p) => updateSsl({ caFile: p }))}
                    onClear={() => updateSsl({ caFile: null })}
                  />
                  <FileRow
                    label="Client cert"
                    value={conn.ssl.certFile}
                    onPick={() => pickFile("Client certificate", (p) => updateSsl({ certFile: p }))}
                    onClear={() => updateSsl({ certFile: null })}
                  />
                  <FileRow
                    label="Client key"
                    value={conn.ssl.keyFile}
                    onPick={() => pickFile("Client key", (p) => updateSsl({ keyFile: p }))}
                    onClear={() => updateSsl({ keyFile: null })}
                  />
                </>
              )}
            </Section>

            <Section
              title="SSH tunnel"
              aside={
                <label className="check">
                  <input
                    type="checkbox"
                    checked={conn.ssh.enabled}
                    onChange={(e) => updateSsh({ enabled: e.target.checked })}
                  />
                  Connect over SSH
                </label>
              }
            >
              {conn.ssh.enabled && (
                <>
                  <Row label="SSH server">
                    <div className="inline">
                      <input
                        className="input mono"
                        value={conn.ssh.host}
                        placeholder="bastion.example.com"
                        onChange={(e) => updateSsh({ host: e.target.value })}
                        spellCheck={false}
                      />
                      <input
                        className="input mono port"
                        value={conn.ssh.port}
                        inputMode="numeric"
                        aria-label="SSH port"
                        onChange={(e) => {
                          const digits = e.target.value.replace(/\D/g, "").slice(0, 5);
                          updateSsh({ port: digits ? Number(digits) : 22 });
                        }}
                      />
                    </div>
                  </Row>
                  <Row label="SSH user">
                    <input
                      className="input mono"
                      value={conn.ssh.user}
                      onChange={(e) => updateSsh({ user: e.target.value })}
                      spellCheck={false}
                    />
                  </Row>
                  <Row label="Sign in with">
                    <select
                      className="input"
                      value={conn.ssh.auth}
                      onChange={(e) => updateSsh({ auth: e.target.value as SshAuth })}
                    >
                      <option value="password">Password</option>
                      <option value="key">Private key</option>
                      <option value="agent">SSH agent (OpenSSH, 1Password, Pageant)</option>
                    </select>
                  </Row>
                  {conn.ssh.auth === "password" && (
                    <Row label="SSH password">
                      <SecretInput
                        field={sshPassword}
                        storedLabel={stored?.hasSshPassword ? "Saved in Windows Credential Manager" : undefined}
                        onChange={setSshPassword}
                      />
                    </Row>
                  )}
                  {conn.ssh.auth === "key" && (
                    <>
                      <FileRow
                        label="Private key"
                        value={conn.ssh.keyFile}
                        placeholder="Default: ~/.ssh/id_ed25519, id_ecdsa or id_rsa"
                        onPick={() => pickFile("SSH private key", (p) => updateSsh({ keyFile: p }))}
                        onClear={() => updateSsh({ keyFile: null })}
                      />
                      <Row label="Passphrase">
                        <SecretInput
                          field={passphrase}
                          placeholder="only if the key has one"
                          storedLabel={stored?.hasSshPassphrase ? "Saved in Windows Credential Manager" : undefined}
                          onChange={setPassphrase}
                        />
                      </Row>
                    </>
                  )}
                </>
              )}
            </Section>
          </>
        )}

        {result && (
          <div className={result.ok ? "notice ok" : "notice error"} role="status">
            {result.text}
          </div>
        )}
      </div>
    </Dialog>
  );
}

function secretFrom(value: string | undefined): SecretField {
  return value === undefined ? { value: "", dirty: false } : { value, dirty: true };
}

function fileName(path: string) {
  return path.split(/[\\/]/).pop()?.replace(/\.(sqlite3?|db3?|s3db|sl3)$/i, "") ?? "";
}

function describeServer(info: ServerInfo) {
  const parts = [`${info.product} ${info.version}`];
  if (info.database) parts.push(`database ${info.database}`);
  if (info.tls) parts.push("TLS");
  if (info.ssh) parts.push("through SSH");
  return `Connected: ${parts.join(" · ")}`;
}

function Section({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="form-section">
      <div className="form-section-head">
        <span>{title}</span>
        {aside}
      </div>
      {children}
    </section>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="form-row">
      <span className="form-label">{label}</span>
      <div className="form-field">{children}</div>
    </div>
  );
}

function FileRow(props: {
  label: string;
  value: string | null;
  placeholder?: string;
  onPick: () => void;
  onClear: () => void;
}) {
  return (
    <Row label={props.label}>
      <div className="inline">
        <input
          className="input mono"
          readOnly
          value={props.value ?? ""}
          placeholder={props.placeholder ?? "none"}
          title={props.value ?? ""}
        />
        <button className="btn" onClick={props.onPick}>
          Choose…
        </button>
        {props.value && (
          <button className="btn" onClick={props.onClear} title="Remove">
            ✕
          </button>
        )}
      </div>
    </Row>
  );
}

function SecretInput(props: {
  field: SecretField;
  storedLabel?: string;
  placeholder?: string;
  onChange: (f: SecretField) => void;
}) {
  const { field, storedLabel } = props;
  const showStored = storedLabel && !field.dirty;
  const cleared = field.dirty && field.value === "" && storedLabel;
  return (
    <div className="inline">
      <input
        className="input mono"
        type="password"
        autoComplete="new-password"
        value={field.value}
        placeholder={showStored ? `•••••••• (${storedLabel})` : cleared ? "will be removed" : props.placeholder ?? ""}
        onChange={(e) => props.onChange({ value: e.target.value, dirty: true })}
      />
      {storedLabel && !cleared && (
        <button className="btn" title="Remove the saved value" onClick={() => props.onChange({ value: "", dirty: true })}>
          Clear
        </button>
      )}
    </div>
  );
}
