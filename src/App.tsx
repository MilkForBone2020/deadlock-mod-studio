import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, errorText, isTauri } from "./api";
import type { Action, Overview } from "./types";
import { LibraryView } from "./LibraryView";
import { ActivityView, ProfilesView, SettingsView } from "./OtherViews";
import { AUDIO_EXTS, SoundBuilderView } from "./SoundBuilder";
import { Icon, Modal, Toasts, type Toast } from "./ui";
import { useUpdater, type Updater } from "./updater";

type View = "library" | "sounds" | "profiles" | "activity" | "settings";

const NAV: { id: View; label: string; icon: string }[] = [
  { id: "library", label: "Library", icon: "library" },
  { id: "sounds", label: "Sound Builder", icon: "wave" },
  { id: "profiles", label: "Profiles", icon: "profiles" },
  { id: "activity", label: "Activity", icon: "activity" },
  { id: "settings", label: "Settings", icon: "settings" },
];

export default function App() {
  const [ov, setOv] = useState<Overview | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [view, setView] = useState<View>("library");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [planOpen, setPlanOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [dropping, setDropping] = useState(false);
  const [logKey, setLogKey] = useState(0);
  const toastId = useRef(0);
  const updater = useUpdater();
  const soundDrop = useRef<((paths: string[]) => void) | null>(null);
  const viewRef = useRef<View>("library");
  viewRef.current = view;

  const toast = useCallback((kind: Toast["kind"], text: string) => {
    setToasts((t) => [...t.slice(-3), { id: ++toastId.current, kind, text }]);
  }, []);
  const dismiss = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);

  const refresh = useCallback(async () => {
    try {
      setOv(await api.overview());
      setLoadError(null);
    } catch (e) {
      setLoadError(errorText(e));
    }
  }, []);

  /** Run a backend call, show its result, then reload state. */
  const act = useCallback(
    async (fn: () => Promise<unknown>, ok?: string) => {
      setBusy(true);
      try {
        await fn();
        if (ok) toast("ok", ok);
        return true;
      } catch (e) {
        toast("error", errorText(e));
        return false;
      } finally {
        setBusy(false);
        setLogKey((k) => k + 1);
        await refresh();
      }
    },
    [refresh, toast],
  );

  const importPaths = useCallback(
    async (paths: string[]) => {
      const vpks = paths.filter((p) => p.toLowerCase().endsWith(".vpk"));
      const audio = paths.filter((p) => AUDIO_EXTS.includes(p.split(".").pop()?.toLowerCase() ?? ""));
      if (audio.length && viewRef.current === "sounds") {
        if (soundDrop.current) soundDrop.current(audio);
        else toast("info", "Create a sound mod on the left first, then add the game sound your file replaces.");
        if (!vpks.length) return;
      }
      if (vpks.length === 0) {
        toast(
          "info",
          audio.length
            ? "Audio files go in Sound Builder: open it, pick the game sound to replace, then drop your file."
            : "Only .vpk and audio files can be dropped here",
        );
        return;
      }
      await act(async () => {
        const problems = await api.importMods(vpks);
        const okCount = vpks.length - problems.length;
        if (okCount > 0) toast("ok", `Imported ${okCount} mod${okCount === 1 ? "" : "s"}. They start switched off.`);
        if (problems.length) throw problems.join("\n");
      });
    },
    [act, toast],
  );

  const pickImport = useCallback(async () => {
    const picked = await open({ multiple: true, filters: [{ name: "VPK", extensions: ["vpk"] }] });
    if (!picked) return;
    await importPaths(Array.isArray(picked) ? picked : [picked]);
  }, [importPaths]);

  // Startup: check for a new Deadlock build (safe mode), then poll.
  useEffect(() => {
    const check = async () => {
      try {
        const ev = await api.checkForUpdate();
        if (ev) toast("info", `Deadlock updated to build ${ev.toBuild}. Safe mode switched every mod off.`);
      } catch {
        /* shown via overview */
      }
      await refresh();
    };
    void check();
    const fast = setInterval(refresh, 10_000);
    const slow = setInterval(check, 60_000);
    const onFocus = () => void check();
    window.addEventListener("focus", onFocus);
    return () => {
      clearInterval(fast);
      clearInterval(slow);
      window.removeEventListener("focus", onFocus);
    };
  }, [refresh, toast]);

  // Drag VPKs from Explorer onto the window to import them.
  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent((e) => {
        const p = e.payload;
        if (p.type === "enter" || p.type === "over") setDropping(true);
        else if (p.type === "leave") setDropping(false);
        else if (p.type === "drop") {
          setDropping(false);
          void importPaths(p.paths);
        }
      })
      .then((u) => (unlisten = u))
      .catch(() => undefined);
    return () => unlisten?.();
  }, [importPaths]);

  if (!ov) {
    return (
      <div className="boot">
        {loadError ? (
          <>
            <h1>Couldn't start</h1>
            <p>{loadError}</p>
            <button className="btn" onClick={refresh}>Try again</button>
          </>
        ) : (
          <p>Loading…</p>
        )}
      </div>
    );
  }

  const pending = ov.plan.actions.length;
  // Only when my files are already in place but the line has gone (Steam
  // update, verify, or another manager rewriting gameinfo.gi); not before
  // the first apply.
  const gameinfoReset =
    ov.game.found &&
    ov.game.addonsLine === false &&
    !ov.safeMode &&
    ov.plan.actions.length > 0 &&
    ov.plan.actions.every((a) => a.kind === "addSearchPath");

  return (
    <div className={`shell ${busy ? "busy" : ""}`}>
      <aside className="sidebar">
        <div className="brand">
          <img src="/icon.png" alt="" width={34} height={34} />
          <div>
            <div className="brand-name">Mod Studio</div>
            <div className="brand-sub">for Deadlock</div>
          </div>
        </div>
        <nav aria-label="Main">
          {NAV.map((n) => (
            <button
              key={n.id}
              className={`nav-item ${view === n.id ? "active" : ""}`}
              aria-current={view === n.id ? "page" : undefined}
              onClick={() => setView(n.id)}
            >
              <Icon name={n.icon} size={18} />
              {n.label}
              {n.id === "library" && pending > 0 && <span className="nav-badge">{pending}</span>}
            </button>
          ))}
        </nav>

        <div className="game-card">
          <div className="game-card-title">Deadlock</div>
          <StatusLine ok={ov.game.found} label={ov.game.found ? "Install found" : "Install not found"} />
          <StatusLine
            ok={!ov.game.running}
            warn
            label={ov.game.running ? "Running: changes wait" : "Not running"}
          />
          <StatusLine
            ok={ov.game.addonsLine === true}
            neutral={!ov.mods.some((m) => m.enabled) && ov.game.addonsLine !== true}
            label={ov.game.addonsLine ? "Loading my mods" : "My mods not loaded"}
          />
          <div className="build">Build {ov.game.buildId ?? "unknown"}</div>
          <button className="btn small launch" disabled={!ov.game.found || ov.game.running || pending > 0} onClick={() => act(() => api.launchGame())} title={pending > 0 ? "Apply your changes first" : "Launch through Steam"}>
            <Icon name="play" size={13} /> Launch
          </button>
        </div>
      </aside>

      <main className="content">
        <UpdateBanner u={updater} />
        {ov.safeMode && (
          <div className="banner safe" role="status">
            <Icon name="shield" size={20} />
            <div>
              <strong>Safe mode: Deadlock updated to build {ov.safeMode.toBuild}</strong>
              <p>
                {ov.safeMode.disabled.length > 0
                  ? `Switched off ${ov.safeMode.disabled.length} mod(s): ${ov.safeMode.disabled.join(", ")}. Turn them back on one at a time and test each in game.`
                  : "No mods were on, so nothing changed."}
              </p>
            </div>
            <button className="btn ghost small" onClick={() => act(() => api.dismissSafeMode())}>Got it</button>
          </div>
        )}
        {gameinfoReset && (
          <div className="banner warn" role="status">
            <Icon name="warn" size={20} />
            <div>
              <strong>Your mods aren't loading</strong>
              <p>
                gameinfo.gi lost the addons_studio line. That happens after a Steam update or "Verify integrity", or when
                Deadlock Mod Manager rewrites the file as it launches. Apply to add it back, then start the game from
                Steam or this app.
              </p>
            </div>
          </div>
        )}
        {!ov.game.found && view !== "settings" && (
          <div className="banner warn" role="status">
            <Icon name="warn" size={20} />
            <div>
              <strong>Deadlock folder not found</strong>
              <p>{ov.game.dir ? `Nothing at ${ov.game.dir}.` : "No folder set yet."} Set it in Settings.</p>
            </div>
            <button className="btn small" onClick={() => setView("settings")}>Open Settings</button>
          </div>
        )}

        <div className="view">
          {view === "library" && (
            <LibraryView ov={ov} act={act} selectedId={selectedId} select={setSelectedId} onImport={pickImport} />
          )}
          {view === "sounds" && (
            <SoundBuilderView
              ov={ov}
              act={act}
              dropRef={soundDrop}
              goTo={(v, modId) => {
                setView(v);
                if (modId) setSelectedId(modId);
              }}
            />
          )}
          {view === "profiles" && <ProfilesView ov={ov} act={act} />}
          {view === "activity" && <ActivityView act={act} refreshKey={logKey} />}
          {view === "settings" && <SettingsView ov={ov} act={act} updater={updater} />}
        </div>

        {(pending > 0 || ov.plan.blockers.length > 0) && ov.game.found && (
          <div className={`apply-bar ${ov.plan.blockers.length ? "blocked" : ""}`}>
            <div>
              <strong>
                {ov.plan.blockers.length
                  ? "Can't apply yet"
                  : `${pending} change${pending === 1 ? "" : "s"} not applied to the game`}
              </strong>
              <span>
                {ov.plan.blockers.length
                  ? ov.plan.blockers[0]
                  : ov.game.running
                    ? "Close Deadlock to apply."
                    : "Nothing in the game folder changes until you apply."}
              </span>
            </div>
            <button className="btn primary" onClick={() => setPlanOpen(true)}>
              Review & apply
            </button>
          </div>
        )}
      </main>

      {planOpen && (
        <PlanModal
          ov={ov}
          close={() => setPlanOpen(false)}
          apply={async () => {
            const ok = await act(() => api.applyChanges(), "Applied. The game folder matches your library.");
            if (ok) setPlanOpen(false);
          }}
          busy={busy}
        />
      )}

      {dropping && (
        <div className="drop-overlay">
          <div>
            <Icon name="plus" size={36} />
            <p>Drop VPKs to import</p>
          </div>
        </div>
      )}
      <Toasts items={toasts} dismiss={dismiss} />
    </div>
  );
}

function UpdateBanner({ u }: { u: Updater }) {
  if (u.status !== "available" && u.status !== "downloading" && !(u.status === "error" && u.latest)) return null;
  return (
    <div className="banner update" role="status">
      <Icon name="down" size={20} />
      <div>
        <strong>
          {u.status === "downloading" ? `Installing v${u.latest}…` : `Update available: v${u.latest}`}
        </strong>
        <p>
          {u.status === "error"
            ? `The update didn't install: ${u.error}`
            : u.status === "downloading"
              ? "The app will reopen by itself when it's done. Your library and settings are kept."
              : `You're on v${u.current}. Installing takes a few seconds and the app reopens itself.`}
        </p>
        {u.status === "downloading" && (
          <div className="progress" aria-label="Download progress">
            <span style={{ width: `${Math.round((u.progress ?? 0.05) * 100)}%` }} />
          </div>
        )}
      </div>
      {u.status !== "downloading" && (
        <button className="btn primary small" onClick={() => void u.install()}>
          Install and restart
        </button>
      )}
    </div>
  );
}

function StatusLine({ ok, label, warn, neutral }: { ok: boolean; label: string; warn?: boolean; neutral?: boolean }) {
  const cls = neutral ? "neutral" : ok ? "ok" : warn ? "warn" : "bad";
  return (
    <div className={`status-line ${cls}`}>
      <span className="dot" />
      {label}
    </div>
  );
}

function describe(a: Action): { icon: string; text: string; cls: string } {
  switch (a.kind) {
    case "createAddonsDir":
      return { icon: "folder", text: "Create the addons_studio folder", cls: "add" };
    case "copy":
      return { icon: "plus", text: `Place "${a.modName}" as addons_studio\\${a.file}`, cls: "add" };
    case "remove":
      return { icon: "trash", text: `Remove addons_studio\\${a.file}${a.modName ? ` ("${a.modName}")` : ""}`, cls: "remove" };
    case "addSearchPath":
      return { icon: "check", text: "Add Game citadel/addons_studio to gameinfo.gi (backed up first)", cls: "add" };
    case "removeSearchPath":
      return { icon: "close", text: "Take the addons_studio line out of gameinfo.gi (backed up first)", cls: "remove" };
  }
}

function PlanModal({ ov, close, apply, busy }: { ov: Overview; close: () => void; apply: () => void; busy: boolean }) {
  const p = ov.plan;
  const blocked = p.blockers.length > 0 || ov.game.running;
  return (
    <Modal
      title="Review changes"
      onClose={close}
      footer={
        <>
          <span className="hint">{ov.game.running ? "Close Deadlock first." : "Only files this app placed are removed."}</span>
          <button className="btn ghost" onClick={close}>Cancel</button>
          <button className="btn primary" disabled={blocked || busy || p.actions.length === 0} onClick={apply}>
            Apply {p.actions.length} change{p.actions.length === 1 ? "" : "s"}
          </button>
        </>
      }
    >
      {p.blockers.length > 0 && (
        <div className="plan-group">
          <h3 className="bad">Fix first</h3>
          <ul className="plan-list">
            {p.blockers.map((b) => (
              <li key={b} className="remove">
                <Icon name="warn" size={14} /> {b}
              </li>
            ))}
          </ul>
        </div>
      )}
      {p.actions.length > 0 && (
        <div className="plan-group">
          <h3>What will change</h3>
          <ul className="plan-list">
            {p.actions.map((a, i) => {
              const d = describe(a);
              return (
                <li key={i} className={d.cls}>
                  <Icon name={d.icon} size={14} /> {d.text}
                </li>
              );
            })}
          </ul>
        </div>
      )}
      {p.warnings.length > 0 && (
        <div className="plan-group">
          <h3 className="warn">Worth knowing</h3>
          <ul className="plan-list">
            {p.warnings.map((w) => (
              <li key={w} className="note">
                <Icon name="warn" size={14} /> {w}
              </li>
            ))}
          </ul>
        </div>
      )}
      <div className="plan-group">
        <h3>addons_studio folder afterwards</h3>
        {p.layout.length === 0 ? (
          <p className="hint">Empty: none of your mods load.</p>
        ) : (
          <ol className="layout-list">
            {p.layout.map(([file, name]) => (
              <li key={file}>
                <code>{file}</code> {name}
              </li>
            ))}
          </ol>
        )}
      </div>
    </Modal>
  );
}
