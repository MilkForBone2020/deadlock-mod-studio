import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import type { Overview } from "./types";
import { Icon } from "./ui";

type Act = (fn: () => Promise<unknown>, ok?: string) => Promise<boolean>;

export function ProfilesView({ ov, act }: { ov: Overview; act: Act }) {
  const [name, setName] = useState("");
  const enabled = ov.mods.filter((m) => m.enabled);
  const nameOf = (id: string) => ov.mods.find((m) => m.id === id)?.name;
  const matches = (mods: string[]) =>
    mods.length === enabled.length && mods.every((id, i) => enabled[i]?.id === id);

  return (
    <div className="page">
      <div className="view-head">
        <div>
          <h1>Profiles</h1>
          <p className="sub">Saved sets of mods, in load order. Switching profile stages the change; apply to update the game.</p>
        </div>
      </div>

      <form
        className="save-profile"
        onSubmit={async (e) => {
          e.preventDefault();
          if (await act(() => api.saveProfile(name), `Saved profile "${name.trim()}"`)) setName("");
        }}
      >
        <div>
          <strong>Save what's on now</strong>
          <span className="hint">
            {enabled.length === 0 ? "No mods are on, so this saves an empty (vanilla) profile." : `${enabled.length} mod(s): ${enabled.map((m) => m.name).join(", ")}`}
          </span>
        </div>
        <input value={name} placeholder="Profile name, e.g. Full" onChange={(e) => setName(e.target.value)} aria-label="Profile name" />
        <button className="btn primary" disabled={!name.trim()}>
          Save
        </button>
      </form>

      {ov.profiles.length === 0 ? (
        <p className="empty-line">No profiles yet. Turn on the mods you want together, then save them here.</p>
      ) : (
        <ul className="profile-list">
          {ov.profiles.map((p) => {
            const active = matches(p.mods);
            const missing = p.mods.filter((id) => !nameOf(id)).length;
            return (
              <li key={p.id} className={`profile ${active ? "active" : ""}`}>
                <div className="profile-main">
                  <div className="profile-name">
                    {p.name}
                    {active && <span className="pill">Current</span>}
                  </div>
                  <ol className="profile-mods">
                    {p.mods.length === 0 && <li className="hint">Vanilla: no mods</li>}
                    {p.mods.map((id) => (nameOf(id) ? <li key={id}>{nameOf(id)}</li> : null))}
                  </ol>
                  {missing > 0 && <span className="hint">{missing} mod(s) in this profile were removed from the library.</span>}
                </div>
                <div className="profile-actions">
                  <button className="btn" disabled={active} onClick={() => act(() => api.applyProfile(p.id), `Switched to "${p.name}". Apply to update the game.`)}>
                    Use
                  </button>
                  <button className="icon-btn" aria-label={`Delete profile ${p.name}`} onClick={() => act(() => api.deleteProfile(p.id), `Deleted "${p.name}"`)}>
                    <Icon name="trash" />
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

export function ActivityView({ act, refreshKey }: { act: Act; refreshKey: number }) {
  const [lines, setLines] = useState<string[]>([]);
  const load = useCallback(() => {
    api.readLog(200).then(setLines).catch(() => setLines([]));
  }, []);
  useEffect(load, [load, refreshKey]);

  return (
    <div className="page">
      <div className="view-head">
        <div>
          <h1>Activity</h1>
          <p className="sub">Every file the app moved, renamed or edited, newest first.</p>
        </div>
        <div className="head-actions">
          <button className="btn ghost" onClick={() => act(() => api.openFolder("backups"))}>
            <Icon name="folder" /> gameinfo backups
          </button>
          <button className="btn ghost" onClick={() => act(() => api.openFolder("logs"))}>
            <Icon name="folder" /> Log folder
          </button>
        </div>
      </div>
      {lines.length === 0 ? (
        <p className="empty-line">Nothing logged yet.</p>
      ) : (
        <ul className="log">
          {lines.map((l, i) => {
            const at = l.slice(0, 25);
            const msg = l.slice(25).trim();
            const d = new Date(at);
            return (
              <li key={i}>
                <time>{Number.isNaN(d.getTime()) ? at : d.toLocaleString()}</time>
                <span>{msg}</span>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

export function SettingsView({ ov, act }: { ov: Overview; act: Act }) {
  const [dir, setDir] = useState(ov.settings.gameDir ?? "");
  const [confirmVanilla, setConfirmVanilla] = useState(false);
  useEffect(() => setDir(ov.settings.gameDir ?? ""), [ov.settings.gameDir]);

  const browse = async () => {
    const picked = await open({ directory: true, multiple: false, defaultPath: dir || undefined });
    if (typeof picked === "string") setDir(picked);
  };
  const detect = async () => {
    const found = await api.detectGameDir();
    if (found) setDir(found);
    else await act(() => Promise.reject("Couldn't find Deadlock automatically. Browse to the folder instead."));
  };

  return (
    <div className="page">
      <div className="view-head">
        <div>
          <h1>Settings</h1>
          <p className="sub">Where Deadlock lives, and the escape hatch.</p>
        </div>
      </div>

      <section className="card">
        <h2>Deadlock folder</h2>
        <p className="hint">The folder that contains <code>game\citadel\gameinfo.gi</code>.</p>
        <div className="path-row">
          <input value={dir} onChange={(e) => setDir(e.target.value)} aria-label="Deadlock folder" spellCheck={false} />
          <button className="btn ghost" onClick={browse}>Browse…</button>
          <button className="btn ghost" onClick={detect}>Detect</button>
          <button className="btn primary" disabled={!dir.trim() || dir === ov.settings.gameDir} onClick={() => act(() => api.setGameDir(dir), "Deadlock folder saved")}>
            Save
          </button>
        </div>
        <dl className="facts-grid">
          <dt>Status</dt>
          <dd>{ov.game.found ? "Found" : "Not found"}</dd>
          <dt>Steam build</dt>
          <dd>{ov.game.buildId ?? "Unknown"}</dd>
          <dt>gameinfo.gi</dt>
          <dd>{ov.game.addonsLine == null ? "—" : ov.game.addonsLine ? "Loads the addons folder" : "Vanilla (addons not loaded)"}</dd>
        </dl>
        <div className="row-actions">
          <button className="btn ghost" disabled={!ov.game.found} onClick={() => act(() => api.openFolder("game"))}>
            <Icon name="folder" /> Open citadel folder
          </button>
          <button className="btn ghost" onClick={() => act(() => api.openFolder("library"))}>
            <Icon name="folder" /> Open mod library
          </button>
        </div>
      </section>

      <section className="card">
        <h2>Safe mode</h2>
        <p className="hint">
          Always on. When Steam installs a new Deadlock build, every mod is switched off and removed from the addons
          folder. Turn mods back on one at a time and mark each one verified once it works.
        </p>
      </section>

      <section className="card danger-card">
        <h2>Restore vanilla</h2>
        <p className="hint">
          Switches every mod off, removes the files this app placed in addons, and takes the addons line out of
          gameinfo.gi. Your library and profiles are kept. Files you put in addons yourself are never touched.
        </p>
        {confirmVanilla ? (
          <div className="row-actions">
            <button
              className="btn danger"
              disabled={ov.game.running}
              onClick={async () => {
                await act(() => api.restoreVanilla(), "Game restored to vanilla");
                setConfirmVanilla(false);
              }}
            >
              Yes, restore vanilla
            </button>
            <button className="btn ghost" onClick={() => setConfirmVanilla(false)}>Cancel</button>
          </div>
        ) : (
          <button className="btn danger-outline" disabled={!ov.game.found} onClick={() => setConfirmVanilla(true)}>
            Restore vanilla…
          </button>
        )}
      </section>

      <p className="foot-note">
        App data: <code>{ov.dataDir}</code>
      </p>
    </div>
  );
}
