import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { api, errorText, isTauri } from "./api";
import type { BuildResult, GameSound, HeroInfo, Level, Overview, SoundJob, SoundProject, SoundUse } from "./types";
import { Icon, Modal } from "./ui";

type Act = (fn: () => Promise<unknown>, ok?: string) => Promise<boolean>;

export const AUDIO_EXTS = ["mp3", "wav", "mp4", "m4a", "ogg", "flac", "aac"];
/** Must match audio::MAX_BOOST_DB / MIN_OFFSET_DB in the Rust core (enforced there). */
const MAX_BOOST = 3;
const MIN_OFFSET = -12;

// ---------- helpers ----------

function shortName(path: string) {
  return (path.split("/").pop() ?? path).replace(/\.vsnd_c$/, "");
}
function fileName(p: string) {
  return p.split(/[\\/]/).pop() ?? p;
}
function fmtDb(v: number) {
  return `${v > 0 ? "+" : v < 0 ? "−" : "±"}${Math.abs(v).toFixed(1)}`;
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
function slotLabel(slot: number) {
  return slot === 4 ? "Ult" : `${slot}`;
}
/** "Seven · Storm Cloud (Ult) · Bolt cast" */
export function useLabel(u: SoundUse, withHero = true) {
  const bits: string[] = [];
  if (withHero && u.heroName) bits.push(u.heroName);
  if (u.abilityName && u.slot) bits.push(`${u.abilityName} (${slotLabel(u.slot)})`);
  else if (u.category && u.category !== "Ability") bits.push(u.category);
  if (u.part) bits.push(u.part);
  return bits.join(" · ");
}
function volumeText(v: number) {
  if (Math.abs(v) < 0.05) return "Same as the original";
  return v > 0 ? `${fmtDb(v)} dB louder than the original` : `${fmtDb(v)} dB quieter than the original`;
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
type Player = ReturnType<typeof usePlayer>;

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

/** Heroes and abilities from the game's data (loaded once). */
function useHeroes() {
  const [heroes, setHeroes] = useState<HeroInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api
      .soundHeroes()
      .then((h) => setHeroes([...h].sort((a, b) => a.name.localeCompare(b.name))))
      .catch((e) => setError(errorText(e)));
  }, []);
  return { heroes, error };
}

function LevelText({ level, error }: { level: Level | null; error: string | null }) {
  if (error) return <span className="lvl bad" title={error}>can't read</span>;
  if (!level) return <span className="lvl muted">measuring…</span>;
  return (
    <span className="lvl" title={`${level.rate} Hz, ${level.channels === 1 ? "mono" : "stereo"}, peak ${level.peakDb.toFixed(1)} dBFS`}>
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

function Uses({ uses, withHero = true }: { uses: SoundUse[]; withHero?: boolean }) {
  if (!uses.length) return <span className="use none">Not referenced by any sound event (may be unused)</span>;
  const first = useLabel(uses[0], withHero);
  const rest = uses.slice(1);
  return (
    <span className="use" title={uses.map((u) => `${useLabel(u)}  [${u.event}]`).join("\n")}>
      {first}
      {rest.length > 0 && <span className="use-more"> +{rest.length} more</span>}
    </span>
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
  const heroes = useHeroes();
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
          <input value={newHero} list="hero-names" placeholder="Hero (optional), e.g. Seven" onChange={(e) => setNewHero(e.target.value)} aria-label="Hero" />
          <datalist id="hero-names">
            {heroes.heroes?.map((h) => <option key={h.code} value={h.name} />)}
          </datalist>
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
          <ProjectEditor key={project.id} p={project} ov={ov} act={act} player={player} heroes={heroes.heroes} goTo={goTo} dropRef={dropRef} />
        ) : (
          <div className="empty">
            <div className="empty-mark">
              <Icon name="wave" size={30} />
            </div>
            <h2>Make your first sound mod</h2>
            <ol className="steps">
              <li>Name it in <strong>New sound mod</strong> on the left and click <strong>Create</strong>.</li>
              <li>Click <strong>Add game sound</strong>, pick a hero and ability, and choose the sounds to replace.</li>
              <li>Choose your file for each one: MP3, WAV, MP4, M4A, OGG or FLAC. You can also drop files on the window.</li>
              <li>Set each one's volume (it starts matched to the original), then click <strong>Build mod</strong>.</li>
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
  heroes,
  goTo,
  dropRef,
}: {
  p: SoundProject;
  ov: Overview;
  act: Act;
  player: Player;
  heroes: HeroInfo[] | null;
  goTo: (view: "library" | "settings", modId?: string) => void;
  dropRef: React.MutableRefObject<((paths: string[]) => void) | null>;
}) {
  const [name, setName] = useState(p.name);
  const [hero, setHero] = useState(p.hero);
  const [picker, setPicker] = useState(false);
  const [building, setBuilding] = useState<string | null>(null);
  const [result, setResult] = useState<BuildResult | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [uses, setUses] = useState<Record<string, SoundUse[]>>({});

  const targetsKey = p.items.map((i) => i.target).join("|");
  useEffect(() => {
    if (!p.items.length) return;
    api.describeSounds(p.items.map((i) => i.target)).then(setUses).catch(() => undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [targetsKey]);

  // The hero this mod is about: from its sounds, else from the Hero field.
  const projectHero = useMemo(() => {
    if (!heroes) return null;
    const counts = new Map<string, number>();
    for (const list of Object.values(uses)) for (const u of list) if (u.hero) counts.set(u.hero, (counts.get(u.hero) ?? 0) + 1);
    const top = [...counts.entries()].sort((a, b) => b[1] - a[1])[0]?.[0];
    return heroes.find((h) => h.code === top) ?? heroes.find((h) => h.name.toLowerCase() === p.hero.trim().toLowerCase()) ?? null;
  }, [heroes, uses, p.hero]);

  const missing = p.items.filter((i) => !i.source).length;
  const builtMod = ov.mods.find((m) => m.id === p.modId) ?? null;
  const canBuild = ov.csdkFound && (p.items.length > 0 || p.muteVoice.length > 0) && missing === 0 && !building;

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
              list="hero-names"
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
          <p>Pick a hero and ability to see exactly which sounds it plays, then choose the ones to replace.</p>
          <button className="btn primary" onClick={() => setPicker(true)}>
            <Icon name="search" size={14} /> Browse game sounds
          </button>
        </div>
      ) : (
        <ul className="items">
          {p.items.map((it) => (
            <ItemRow key={it.target} projectId={p.id} item={it} uses={uses[it.target]} act={act} player={player} choose={() => choose(it.target)} />
          ))}
        </ul>
      )}

      {projectHero && projectHero.abilities.some((a) => a.voiceLines > 0) && (
        <section className="voice-card">
          <h3>{projectHero.name}'s voice lines</h3>
          <p className="hint">
            Heroes often speak when they cast. Mute those lines in this mod so they don't talk over your sounds. It only
            changes what you hear, and also mutes the lines you'd hear from an enemy {projectHero.name}.
          </p>
          <div className="voice-list">
            {projectHero.abilities.map((a) => {
              const on = p.muteVoice.some((m) => m.hero === projectHero.code && m.slot === a.slot);
              return (
                <label key={a.slot} className={`voice-opt ${on ? "on" : ""} ${a.voiceLines ? "" : "disabled"}`}>
                  <input
                    type="checkbox"
                    checked={on}
                    disabled={!a.voiceLines}
                    onChange={(e) => act(() => api.setVoiceMute(p.id, projectHero.code, a.slot, e.target.checked))}
                  />
                  <span>
                    <strong>{a.name}</strong> <span className="slot-tag">{slotLabel(a.slot)}</span>
                    <span className="hint"> · {a.voiceLines} line{a.voiceLines === 1 ? "" : "s"}</span>
                  </span>
                </label>
              );
            })}
          </div>
        </section>
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
              <span>{missing ? `${missing} sound(s) still need a file.` : "Each sound starts at the original's loudness; adjust with the sliders."}</span>
            </>
          )}
        </div>
        {builtMod && (
          <button className="btn ghost" onClick={() => goTo("library", builtMod.id)}>Show in Library</button>
        )}
        <button className="btn primary" disabled={!canBuild} onClick={build}>
          {builtMod ? "Rebuild mod" : "Build mod"}
        </button>
      </div>

      {picker && (
        <SoundPicker
          heroes={heroes}
          defaultHero={projectHero?.code ?? null}
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
  item,
  uses,
  act,
  player,
  choose,
}: {
  projectId: string;
  item: SoundJob;
  uses: SoundUse[] | undefined;
  act: Act;
  player: Player;
  choose: () => void;
}) {
  const { target, source } = item;
  const orig = useLevel("game", target);
  const mine = useLevel("file", source || null);
  const [vol, setVol] = useState(item.volumeDb ?? 0);
  useEffect(() => setVol(item.volumeDb ?? 0), [item.volumeDb]);
  const commit = () => {
    if (Math.abs(vol - (item.volumeDb ?? 0)) > 0.01) void act(() => api.setItemVolume(projectId, target, vol));
  };
  const looped = /_lp(\.|_|1)|_loop\./.test(target.split("/").pop() ?? "");
  const change = orig.level && mine.level ? orig.level.loudnessDb + vol - mine.level.loudnessDb : null;

  return (
    <li className="item">
      <div className="item-top">
        <div className="item-side">
          <div className="item-title">
            <PlayBtn active={player.playing === `game:${target}`} onClick={() => void player.play("game", target)} label="Play original" />
            <span className="item-name">{uses ? <Uses uses={uses} /> : shortName(target)}</span>
            {looped && <span className="tag">loop</span>}
          </div>
          <code className="item-path" title={target}>{shortName(target)}</code>
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
      </div>
      {source && (
        <div className="vol-row">
          <span className="vol-label">Volume</span>
          <input
            type="range"
            min={MIN_OFFSET}
            max={MAX_BOOST}
            step={0.5}
            value={vol}
            aria-label="Volume compared to the original"
            onChange={(e) => setVol(Number(e.target.value))}
            onMouseUp={commit}
            onKeyUp={commit}
            onTouchEnd={commit}
            onBlur={commit}
          />
          <span className={`vol-value ${vol > 0 ? "up" : ""}`}>{volumeText(vol)}</span>
          {vol !== 0 && (
            <button className="link small" onClick={() => { setVol(0); void act(() => api.setItemVolume(projectId, target, 0)); }}>
              Reset
            </button>
          )}
          {change !== null && Math.abs(change) >= 0.5 && (
            <span className="vol-note">
              Your file will be turned {change > 0 ? "up" : "down"} {Math.abs(change).toFixed(1)} dB
            </span>
          )}
        </div>
      )}
    </li>
  );
}

type Group = { key: string; label: string };

function groupsFor(h: HeroInfo | undefined): Group[] {
  const g: Group[] = [{ key: "", label: "Everything" }];
  if (h) for (const a of h.abilities) g.push({ key: `slot:${a.slot}`, label: `${a.name} (${slotLabel(a.slot)})` });
  for (const c of ["Weapon", "Melee", "Movement", "Voice", "Other"]) g.push({ key: c, label: c });
  return g;
}

function SoundPicker({
  heroes,
  defaultHero,
  existing,
  player,
  close,
  add,
}: {
  heroes: HeroInfo[] | null;
  defaultHero: string | null;
  existing: string[];
  player: Player;
  close: () => void;
  add: (target: string) => Promise<boolean>;
}) {
  const [hero, setHero] = useState<string>(defaultHero ?? "");
  const [group, setGroup] = useState("");
  const [q, setQ] = useState("");
  const [res, setRes] = useState<{ total: number; results: GameSound[] } | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [added, setAdded] = useState<string[]>([]);
  const have = useMemo(() => new Set([...existing, ...added]), [existing, added]);
  const heroInfo = heroes?.find((h) => h.code === hero);

  useEffect(() => setGroup(""), [hero]);

  useEffect(() => {
    if (!hero && q.trim().length < 2) {
      setRes(null);
      return;
    }
    setLoading(true);
    const h = setTimeout(() => {
      api
        .searchGameSounds(q, hero || null, group || null, 300)
        .then((r) => {
          setRes(r);
          setErr(null);
        })
        .catch((e) => setErr(errorText(e)))
        .finally(() => setLoading(false));
    }, 200);
    return () => clearTimeout(h);
  }, [q, hero, group]);

  return (
    <Modal title="Add game sounds" onClose={close} footer={<button className="btn primary" onClick={close}>Done</button>}>
      <div className="picker-filters">
        <select value={hero} onChange={(e) => setHero(e.target.value)} aria-label="Hero">
          <option value="">All heroes</option>
          {heroes?.map((h) => (
            <option key={h.code} value={h.code}>{h.name}</option>
          ))}
        </select>
        <label className="search wide">
          <Icon name="search" size={15} />
          <input autoFocus value={q} placeholder={hero ? "Filter, e.g. cast, loop, bolt" : "Search, e.g. seven storm cloud"} onChange={(e) => setQ(e.target.value)} aria-label="Search game sounds" />
        </label>
      </div>
      {hero && (
        <div className="chips wrap" role="group" aria-label="Ability or category">
          {groupsFor(heroInfo).map((g) => (
            <button key={g.key} className={`chip ${group === g.key ? "active" : ""}`} onClick={() => setGroup(g.key)}>
              {g.label}
            </button>
          ))}
        </div>
      )}
      {!heroes && <p className="hint">Reading the game's sound data… (first time takes a few seconds)</p>}
      {err && <p className="hint bad">{err}</p>}
      {!hero && !q && <p className="hint">Pick a hero to browse their sounds by ability, or search everything.</p>}
      {res && (
        <>
          <p className="hint">
            {loading ? "Searching…" : `${res.total} sound${res.total === 1 ? "" : "s"}`}
            {res.total > res.results.length ? `, showing the first ${res.results.length}` : ""}. Click ▶ to listen.
          </p>
          <ul className="pick-list">
            {res.results.map((s) => (
              <li key={s.path}>
                <PlayBtn active={player.playing === `game:${s.path}`} onClick={() => void player.play("game", s.path)} label="Play" />
                <div className="pick-main">
                  <span className="item-name">{s.uses.length ? <Uses uses={hero ? s.uses.filter((u) => u.hero === hero).concat(s.uses.filter((u) => u.hero !== hero)) : s.uses} withHero={!hero} /> : shortName(s.path)}</span>
                  <code className="item-path" title={s.path}>{s.path.replace(/^sounds\//, "")}</code>
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
        <strong>"{r.modName}"</strong>: {r.report.items.length} sound(s){r.report.muted ? `, ${r.report.muted} voice line(s) muted` : ""},{" "}
        {(r.report.vpkSize / 1024).toFixed(0)} KB.
      </p>
      {r.report.items.length > 0 && (
        <table className="report">
          <thead>
            <tr>
              <th>Sound</th>
              <th>Original</th>
              <th>Your file</th>
              <th>In game</th>
              <th>Notes</th>
            </tr>
          </thead>
          <tbody>
            {r.report.items.map((i) => (
              <tr key={i.target}>
                <td>{shortName(i.target)}</td>
                <td>{i.original.loudnessDb.toFixed(1)}</td>
                <td>{i.yours.loudnessDb.toFixed(1)}</td>
                <td className={i.volumeDb > 0 ? "up" : ""}>
                  {i.result.loudnessDb.toFixed(1)} <span className="hint">({fmtDb(i.result.loudnessDb - i.original.loudnessDb)})</span>
                </td>
                <td>
                  {[
                    i.looped && "loop",
                    i.limited && "limiter",
                    i.convertedChannels && (i.original.channels === 1 ? "made mono" : "made stereo"),
                  ]
                    .filter(Boolean)
                    .join(", ")}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <p className="hint">Loudness in LUFS. "In game" is never more than +{MAX_BOOST} dB above the original.</p>
      <details>
        <summary>Compiler output</summary>
        <pre className="log-pre">{r.report.log}</pre>
      </details>
    </Modal>
  );
}
