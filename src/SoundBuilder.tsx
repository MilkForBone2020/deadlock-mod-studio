import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { api, errorText, isTauri } from "./api";
import type { BuildResult, GameSound, Level, Overview, SoundProject } from "./types";
import { Icon, Modal } from "./ui";

type Act = (fn: () => Promise<unknown>, ok?: string) => Promise<boolean>;

export const AUDIO_EXTS = ["mp3", "wav", "mp4", "m4a", "ogg", "flac", "aac"];

// ---------- helpers ----------

function shortName(path: string) {
  const f = path.split("/").pop() ?? path;
  return f.replace(/\.vsnd_c$/, "");
}
function folder(path: string) {
  return path.split("/").slice(0, -1).join("/");
}
function fileName(p: string) {
  return p.split(/[\\/]/).pop() ?? p;
}
function fmtDb(v: number) {
  return `${v > 0 ? "+" : ""}${v.toFixed(1)}`;
}
function fmtLen(s: number) {
  return s >= 60 ? `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, "0")}` : `${s.toFixed(1)} s`;
}
function mimeFor(bytes: Uint8Array) {
  const s = String.fromCharCode(...bytes.slice(0, 12));
  if (s.startsWith("RIFF")) return "audio/wav";
  if (s.startsWith("OggS")) return "audio/ogg";
  if (s.startsWith("fLaC")) return "audio/flac";
  if (s.slice(4, 8) === "ftyp") return "audio/mp4";
  return "audio/mpeg";
}

/** One shared player: play a game sound or one of my files, click again to stop. */
function usePlayer() {
  const audio = useRef<HTMLAudioElement | null>(null);
  const url = useRef<string | null>(null);
  const [playing, setPlaying] = useState<string | null>(null);

  const stop = useCallback(() => {
    audio.current?.pause();
    if (url.current) URL.revokeObjectURL(url.current);
    url.current = null;
    setPlaying(null);
  }, []);

  const play = useCallback(
    async (kind: "game" | "file", path: string) => {
      const key = `${kind}:${path}`;
      if (playing === key) return stop();
      stop();
      if (!isTauri) return;
      const buf = new Uint8Array(await api.audioBytes(kind, path));
      url.current = URL.createObjectURL(new Blob([buf], { type: mimeFor(buf) }));
      const a = audio.current ?? new Audio();
      audio.current = a;
      a.src = url.current;
      a.onended = () => setPlaying(null);
      setPlaying(key);
      await a.play();
    },
    [playing, stop],
  );

  useEffect(() => stop, [stop]);
  return { play, stop, playing };
}

/** Lazily measured loudness for a game sound or a file. */
function useLevel(kind: "game" | "file", path: string | null) {
  const [level, setLevel] = useState<Level | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setLevel(null);
    setError(null);
    if (!path) return;
    let live = true;
    api
      .audioLevel(kind, path)
      .then((l) => live && setLevel(l))
      .catch((e) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [kind, path]);
  return { level, error };
}

function LevelText({ level, error }: { level: Level | null; error: string | null }) {
  if (error) return <span className="lvl bad" title={error}>can't read</span>;
  if (!level) return <span className="lvl muted">measuring…</span>;
  return (
    <span className="lvl" title={`${level.rate} Hz, ${level.channels === 1 ? "mono" : "stereo"}, peak ${fmtDb(level.peakDb)} dBFS`}>
      {fmtLen(level.durationSecs)} · {level.loudnessDb.toFixed(1)} LUFS
    </span>
  );
}

function PlayBtn({ active, onClick, label }: { active: boolean; onClick: () => void; label: string }) {
  return (
    <button className={`icon-btn play ${active ? "on" : ""}`} onClick={onClick} aria-label={label} title={label}>
      <Icon name={active ? "close" : "play"} size={13} />
    </button>
  );
}

// ---------- main view ----------

export function SoundBuilderView({
  ov,
  act,
  goTo,
  dropRef,
}: {
  ov: Overview;
  act: Act;
  goTo: (view: "library" | "settings", modId?: string) => void;
  dropRef: React.MutableRefObject<((paths: string[]) => void) | null>;
}) {
  const [selectedId, setSelectedId] = useState<string | null>(ov.soundProjects[0]?.id ?? null);
  const [newName, setNewName] = useState("");
  const [newHero, setNewHero] = useState("");
  const player = usePlayer();
  const project = ov.soundProjects.find((p) => p.id === selectedId) ?? null;

  useEffect(() => {
    if (!project && ov.soundProjects.length) setSelectedId(ov.soundProjects[0].id);
  }, [project, ov.soundProjects]);

  return (
    <div className="builder">
      <aside className="builder-list">
        <div className="builder-list-head">
          <h1>Sound Builder</h1>
          <p className="sub">Replace game sounds with your own files.</p>
        </div>
        <ul className="project-list">
          {ov.soundProjects.map((p) => (
            <li key={p.id}>
              <button className={`project-item ${p.id === selectedId ? "active" : ""}`} onClick={() => setSelectedId(p.id)}>
                <span className="pi-name">{p.name}</span>
                <span className="pi-meta">
                  {p.hero && `${p.hero} · `}
                  {p.items.length} sound{p.items.length === 1 ? "" : "s"}
                  {p.lastBuilt ? " · built" : ""}
                </span>
              </button>
            </li>
          ))}
        </ul>
        <form
          className="new-project"
          onSubmit={async (e) => {
            e.preventDefault();
            let created: SoundProject | null = null;
            const ok = await act(async () => {
              created = await api.createSoundProject(newName, newHero);
            });
            if (ok && created) {
              setSelectedId((created as SoundProject).id);
              setNewName("");
              setNewHero("");
            }
          }}
        >
          <strong>New sound mod</strong>
          <input value={newName} placeholder="Name, e.g. Storm Cloud remix" onChange={(e) => setNewName(e.target.value)} aria-label="New sound mod name" />
          <input value={newHero} placeholder="Hero (optional), e.g. Seven" onChange={(e) => setNewHero(e.target.value)} aria-label="Hero" />
          <button className="btn primary small" disabled={!newName.trim()}>
            <Icon name="plus" size={14} /> Create
          </button>
        </form>
      </aside>

      <section className="builder-main">
        {!ov.csdkFound && (
          <div className="banner warn inline" role="status">
            <Icon name="warn" size={20} />
            <div>
              <strong>CSDK isn't set up</strong>
              <p>Building needs CSDK 13's compiler. Set its folder in Settings.</p>
            </div>
            <button className="btn small" onClick={() => goTo("settings")}>Open Settings</button>
          </div>
        )}
        {project ? (
          <ProjectEditor key={project.id} p={project} ov={ov} act={act} player={player} goTo={goTo} dropRef={dropRef} />
        ) : (
          <div className="empty">
            <div className="empty-mark">
              <Icon name="activity" size={30} />
            </div>
            <h2>Make your first sound mod</h2>
            <ol className="steps">
              <li>Name it in <strong>New sound mod</strong> on the left and click <strong>Create</strong>.</li>
              <li>Click <strong>Add game sound</strong> and pick the sounds to replace (Seven is <code>gigawatt</code>).</li>
              <li>Choose your file for each one: MP3, WAV, MP4, M4A, OGG or FLAC. You can also drop files on the window.</li>
              <li>Click <strong>Build mod</strong>. Volume is matched to the originals and the mod lands in your library.</li>
            </ol>
          </div>
        )}
      </section>
    </div>
  );
}

function ProjectEditor({
  p,
  ov,
  act,
  player,
  goTo,
  dropRef,
}: {
  p: SoundProject;
  ov: Overview;
  act: Act;
  player: ReturnType<typeof usePlayer>;
  goTo: (view: "library" | "settings", modId?: string) => void;
  dropRef: React.MutableRefObject<((paths: string[]) => void) | null>;
}) {
  const [name, setName] = useState(p.name);
  const [hero, setHero] = useState(p.hero);
  const [picker, setPicker] = useState(false);
  const [building, setBuilding] = useState<string | null>(null);
  const [result, setResult] = useState<BuildResult | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const missing = p.items.filter((i) => !i.source).length;
  const builtMod = ov.mods.find((m) => m.id === p.modId) ?? null;

  // Audio files dropped on the window fill the first sound without a file.
  useEffect(() => {
    dropRef.current = (paths: string[]) => {
      const audio = paths.filter((x) => AUDIO_EXTS.includes(x.split(".").pop()?.toLowerCase() ?? ""));
      const empty = p.items.filter((i) => !i.source);
      if (!audio.length) return;
      if (!empty.length) {
        void act(() =>
          Promise.reject(
            p.items.length
              ? "Every sound already has a file. Use Change on a row to swap it, or Add game sound for a new one."
              : "Add the game sound your file replaces first (Add game sound), then drop it again.",
          ),
        );
        return;
      }
      void act(async () => {
        for (let k = 0; k < Math.min(audio.length, empty.length); k++) {
          await api.setProjectItem(p.id, empty[k].target, audio[k]);
        }
      }, `Added ${Math.min(audio.length, empty.length)} file(s)`);
    };
    return () => {
      dropRef.current = null;
    };
  }, [p, act, dropRef]);

  const choose = async (target: string) => {
    const picked = await open({ multiple: false, filters: [{ name: "Audio", extensions: AUDIO_EXTS }] });
    if (typeof picked === "string") await act(() => api.setProjectItem(p.id, target, picked));
  };

  const build = async () => {
    setBuilding("Starting…");
    const un = isTauri ? await listen<string>("build-progress", (e) => setBuilding(e.payload)) : () => undefined;
    try {
      let r: BuildResult | null = null;
      const ok = await act(async () => {
        r = await api.buildSoundProject(p.id);
      });
      if (ok && r) setResult(r);
    } finally {
      un();
      setBuilding(null);
    }
  };

  return (
    <div className="editor">
      <div className="editor-head">
        <div className="form-row">
          <label>
            Name
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              onBlur={() => name !== p.name && act(() => api.updateSoundProject(p.id, name, undefined))}
            />
          </label>
          <label>
            Hero
            <input
              value={hero}
              placeholder="e.g. Seven"
              onChange={(e) => setHero(e.target.value)}
              onBlur={() => hero !== p.hero && act(() => api.updateSoundProject(p.id, undefined, hero))}
            />
          </label>
        </div>
        <div className="head-actions">
          {confirmDelete ? (
            <span className="confirm">
              Delete project?
              <button className="btn danger small" onClick={() => act(() => api.deleteSoundProject(p.id), `Deleted "${p.name}"`)}>Delete</button>
              <button className="btn ghost small" onClick={() => setConfirmDelete(false)}>Cancel</button>
            </span>
          ) : (
            <button className="icon-btn" aria-label="Delete project" title="Delete project (the built mod stays in your library)" onClick={() => setConfirmDelete(true)}>
              <Icon name="trash" />
            </button>
          )}
        </div>
      </div>

      <div className="items-head">
        <h3>Sounds replaced ({p.items.length})</h3>
        <button className="btn small" onClick={() => setPicker(true)}>
          <Icon name="plus" size={14} /> Add game sound
        </button>
      </div>

      {p.items.length === 0 ? (
        <div className="empty small">
          <p>
            Pick the game sounds this mod replaces. Search by hero file name: Seven's sounds are under <code>gigawatt</code>,
            and Storm Cloud is <code>storm_cloud</code>.
          </p>
          <button className="btn primary" onClick={() => setPicker(true)}>
            <Icon name="search" size={14} /> Find game sounds
          </button>
        </div>
      ) : (
        <ul className="items">
          {p.items.map((it) => (
            <ItemRow key={it.target} projectId={p.id} target={it.target} source={it.source} act={act} player={player} choose={() => choose(it.target)} />
          ))}
        </ul>
      )}

      <div className="build-bar">
        <div>
          {builtMod ? (
            <>
              <strong>In your library as "{builtMod.name}" (v{builtMod.version})</strong>
              <span>Rebuilding updates that mod. It stays on or off as it is.</span>
            </>
          ) : (
            <>
              <strong>Not built yet</strong>
              <span>{missing ? `${missing} sound(s) still need a file.` : "Volume is matched to each original automatically."}</span>
            </>
          )}
        </div>
        {builtMod && (
          <button className="btn ghost" onClick={() => goTo("library", builtMod.id)}>Show in Library</button>
        )}
        <button className="btn primary" disabled={!ov.csdkFound || p.items.length === 0 || missing > 0 || !!building} onClick={build}>
          {builtMod ? "Rebuild mod" : "Build mod"}
        </button>
      </div>

      {picker && (
        <SoundPicker
          existing={p.items.map((i) => i.target)}
          player={player}
          close={() => setPicker(false)}
          add={(target) => act(() => api.setProjectItem(p.id, target, ""), `Added ${shortName(target)}`)}
        />
      )}
      {building && (
        <div className="modal-backdrop">
          <div className="modal progress-modal" role="status">
            <div className="spinner" />
            <strong>Building "{p.name}"</strong>
            <span>{building}</span>
          </div>
        </div>
      )}
      {result && <BuildReportModal r={result} close={() => setResult(null)} goLibrary={() => goTo("library", result.modId)} />}
    </div>
  );
}

function ItemRow({
  projectId,
  target,
  source,
  act,
  player,
  choose,
}: {
  projectId: string;
  target: string;
  source: string;
  act: Act;
  player: ReturnType<typeof usePlayer>;
  choose: () => void;
}) {
  const orig = useLevel("game", target);
  const mine = useLevel("file", source || null);
  const looped = /_lp(\.|_|1)|_loop\./.test(target.split("/").pop() ?? "");
  const louderAvg = !!(orig.level && mine.level && mine.level.loudnessDb > orig.level.loudnessDb + 0.2);
  const louderPeak = !!(orig.level && mine.level && mine.level.peakDb > orig.level.peakDb + 0.2);
  return (
    <li className="item">
      <div className="item-side">
        <div className="item-title">
          <PlayBtn active={player.playing === `game:${target}`} onClick={() => void player.play("game", target)} label="Play original" />
          <span className="item-name">{shortName(target)}</span>
          {looped && <span className="tag">loop</span>}
        </div>
        <code className="item-path">{folder(target)}</code>
        <LevelText {...orig} />
      </div>
      <Icon name="swap" size={16} />
      <div className="item-side">
        {source ? (
          <>
            <div className="item-title">
              <PlayBtn active={player.playing === `file:${source}`} onClick={() => void player.play("file", source)} label="Play your file" />
              <span className="item-name" title={source}>{fileName(source)}</span>
              <button className="link small" onClick={choose}>Change</button>
            </div>
            <LevelText {...mine} />
            {louderAvg && <span className="lvl note">Louder than the original: will be turned down to match</span>}
            {!louderAvg && louderPeak && <span className="lvl note">Peaks higher than the original: will be turned down slightly</span>}
          </>
        ) : (
          <button className="btn small dashed" onClick={choose}>
            Choose your file…
          </button>
        )}
      </div>
      <button className="icon-btn" aria-label={`Remove ${shortName(target)}`} onClick={() => act(() => api.removeProjectItem(projectId, target))}>
        <Icon name="close" size={14} />
      </button>
    </li>
  );
}

function SoundPicker({
  existing,
  player,
  close,
  add,
}: {
  existing: string[];
  player: ReturnType<typeof usePlayer>;
  close: () => void;
  add: (target: string) => Promise<boolean>;
}) {
  const [q, setQ] = useState("");
  const [res, setRes] = useState<{ total: number; results: GameSound[] } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [added, setAdded] = useState<string[]>([]);
  const have = useMemo(() => new Set([...existing, ...added]), [existing, added]);

  useEffect(() => {
    if (q.trim().length < 2) {
      setRes(null);
      return;
    }
    const h = setTimeout(() => {
      api
        .searchGameSounds(q, 200)
        .then((r) => {
          setRes(r);
          setErr(null);
        })
        .catch((e) => setErr(errorText(e)));
    }, 250);
    return () => clearTimeout(h);
  }, [q]);

  return (
    <Modal title="Add game sounds" onClose={close} footer={<button className="btn primary" onClick={close}>Done</button>}>
      <label className="search wide">
        <Icon name="search" size={15} />
        <input autoFocus value={q} placeholder="Search, e.g. gigawatt storm_cloud" onChange={(e) => setQ(e.target.value)} aria-label="Search game sounds" />
      </label>
      <p className="hint">
        Sounds use the game's internal names: Seven is <code>gigawatt</code>. Several words narrow it down. Click ▶ to listen.
      </p>
      {err && <p className="hint bad">{err}</p>}
      {res && (
        <>
          <p className="hint">
            {res.total} match{res.total === 1 ? "" : "es"}
            {res.total > res.results.length ? `, showing the first ${res.results.length}` : ""}
          </p>
          <ul className="pick-list">
            {res.results.map((s) => (
              <li key={s.path}>
                <PlayBtn active={player.playing === `game:${s.path}`} onClick={() => void player.play("game", s.path)} label="Play" />
                <div className="pick-main">
                  <span className="item-name">{shortName(s.path)}</span>
                  <code className="item-path">{folder(s.path)}</code>
                </div>
                {s.looped && <span className="tag">loop</span>}
                {have.has(s.path) ? (
                  <span className="added"><Icon name="check" size={14} /> Added</span>
                ) : (
                  <button
                    className="btn small"
                    onClick={async () => {
                      if (await add(s.path)) setAdded((a) => [...a, s.path]);
                    }}
                  >
                    Add
                  </button>
                )}
              </li>
            ))}
          </ul>
        </>
      )}
    </Modal>
  );
}

function BuildReportModal({ r, close, goLibrary }: { r: BuildResult; close: () => void; goLibrary: () => void }) {
  const turnedDown = r.report.items.filter((i) => i.gainDb < 0).length;
  return (
    <Modal
      title="Sound mod built"
      onClose={close}
      footer={
        <>
          <span className="hint">It's in your library. Switch it on and apply to hear it in game.</span>
          <button className="btn ghost" onClick={close}>Close</button>
          <button className="btn primary" onClick={() => { close(); goLibrary(); }}>Go to Library</button>
        </>
      }
    >
      <p>
        <strong>"{r.modName}"</strong>: {r.report.items.length} sound(s), {(r.report.vpkSize / 1024).toFixed(0)} KB.{" "}
        {turnedDown ? `${turnedDown} turned down to match the originals.` : "All were already at or below the originals' volume."}
      </p>
      <table className="report">
        <thead>
          <tr>
            <th>Sound</th>
            <th>Original (LUFS)</th>
            <th>Yours (LUFS)</th>
            <th>Adjusted</th>
            <th>Notes</th>
          </tr>
        </thead>
        <tbody>
          {r.report.items.map((i) => (
            <tr key={i.target}>
              <td>{shortName(i.target)}</td>
              <td>{i.original.loudnessDb.toFixed(1)}</td>
              <td>{i.yours.loudnessDb.toFixed(1)}</td>
              <td className={i.gainDb < 0 ? "down" : ""}>{i.gainDb < 0 ? `${fmtDb(i.gainDb)} dB` : "—"}</td>
              <td>
                {[i.looped && "loop", i.convertedChannels && (i.original.channels === 1 ? "made mono" : "made stereo")].filter(Boolean).join(", ")}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <details>
        <summary>Compiler output</summary>
        <pre className="log-pre">{r.report.log}</pre>
      </details>
    </Modal>
  );
}
