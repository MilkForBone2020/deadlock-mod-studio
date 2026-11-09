import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { api, isTauri } from "./api";
import type { IconBuildResult, IconHero, IconProject, IconSlot, Overview } from "./types";
import { Icon, Modal, formatSize } from "./ui";

type Act = (fn: () => Promise<unknown>, ok?: string) => Promise<boolean>;

const HINT: Record<string, string> = {
  m_strIconHeroCard: "The top bar and menus. Shown most of the time.",
  m_strIconHeroCardCritical: "Swapped in when the hero is low on health.",
  m_strIconHeroCardGloat: "Swapped in when the hero is doing well.",
  m_strTopBarVertical: "Tall icon used in some lists.",
  m_strIconImageSmall: "Small square icon (scoreboard, kill feed).",
  m_strMinimapImage: "Tiny icon on the minimap.",
};

function useIconHeroes() {
  const [data, setData] = useState<[IconHero[], IconSlot[]] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.iconHeroes().then(setData).catch((e) => setError(String(e)));
  }, []);
  return { heroes: data?.[0] ?? null, slots: data?.[1] ?? [], error };
}

export function IconBuilderView({ ov, act, goTo }: { ov: Overview; act: Act; goTo: (view: "library" | "settings", modId?: string) => void }) {
  const [selectedId, setSelectedId] = useState<string | null>(ov.iconProjects[0]?.id ?? null);
  const [newName, setNewName] = useState("");
  const [newHero, setNewHero] = useState("");
  const { heroes, slots, error } = useIconHeroes();
  const project = ov.iconProjects.find((p) => p.id === selectedId) ?? null;
  const heroName = (code: string) => heroes?.find((h) => h.code === code)?.name ?? code;

  useEffect(() => {
    if (!project && ov.iconProjects.length) setSelectedId(ov.iconProjects[0].id);
  }, [project, ov.iconProjects]);

  return (
    <div className="builder">
      <aside className="builder-list">
        <div className="builder-list-head">
          <h1>Portraits</h1>
          <p className="sub">Replace a hero's portraits in the top bar and menus.</p>
        </div>
        <ul className="project-list">
          {ov.iconProjects.map((p) => (
            <li key={p.id}>
              <button className={`project-item ${p.id === selectedId ? "active" : ""}`} onClick={() => setSelectedId(p.id)}>
                <span className="pi-name">{p.name}</span>
                <span className="pi-meta">
                  {heroName(p.hero)} · {Object.keys(p.images).length} picture(s){p.lastBuilt ? " · built" : ""}
                </span>
              </button>
            </li>
          ))}
        </ul>
        <form
          className="new-project"
          onSubmit={async (e) => {
            e.preventDefault();
            let created: IconProject | null = null;
            const ok = await act(async () => {
              created = await api.createIconProject(newName, newHero);
            });
            if (ok && created) {
              setSelectedId((created as IconProject).id);
              setNewName("");
            }
          }}
        >
          <strong>New portraits</strong>
          <input value={newName} placeholder="Name, e.g. Monochrome portraits" onChange={(e) => setNewName(e.target.value)} aria-label="New portraits name" />
          <select value={newHero} onChange={(e) => setNewHero(e.target.value)} aria-label="Hero">
            <option value="">{heroes ? "Pick a hero" : error ? "Couldn't read heroes" : "Loading heroes…"}</option>
            {heroes?.map((h) => (
              <option key={h.code} value={h.code}>
                {h.name}
              </option>
            ))}
          </select>
          <button className="btn primary small" disabled={!newName.trim() || !newHero}>
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
            <button className="btn small" onClick={() => goTo("settings")}>
              Open Settings
            </button>
          </div>
        )}
        {project ? (
          <IconEditor key={project.id} p={project} ov={ov} act={act} hero={heroes?.find((h) => h.code === project.hero) ?? null} slots={slots} goTo={goTo} />
        ) : (
          <div className="empty">
            <div className="empty-mark">
              <Icon name="portrait" size={30} />
            </div>
            <h2>Make your first portraits</h2>
            <ol className="steps">
              <li>
                Name them in <strong>New portraits</strong> on the left, pick the hero and click <strong>Create</strong>.
              </li>
              <li>Choose a picture for each slot you want to change. Cards are 280 × 380 (render at 560 × 760 for sharper results).</li>
              <li>
                Click <strong>Build</strong>, then switch the mod on in the Library.
              </li>
            </ol>
          </div>
        )}
      </section>
    </div>
  );
}

function SlotPreview({ field, source, slot }: { field: string; source: string | undefined; slot: IconSlot }) {
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setUrl(null);
    setError(null);
    if (!source) return;
    let u: string | null = null;
    let cancelled = false;
    api
      .iconPreview(field, source)
      .then((b) => {
        if (cancelled) return;
        u = URL.createObjectURL(new Blob([b], { type: "image/png" }));
        setUrl(u);
      })
      .catch((e) => !cancelled && setError(String(e)));
    return () => {
      cancelled = true;
      if (u) URL.revokeObjectURL(u);
    };
  }, [field, source]);
  const scale = Math.min(1, 190 / slot.height, 150 / slot.width);
  return (
    <div className="slot-frame" style={{ width: Math.round(slot.width * Math.max(scale, slot.width < 100 ? 1.6 : scale)), aspectRatio: `${slot.width} / ${slot.height}` }}>
      {url ? <img src={url} alt="" /> : <span>{error ? "Can't read this picture" : source ? "…" : "Game's own"}</span>}
    </div>
  );
}

function IconEditor({
  p,
  ov,
  act,
  hero,
  slots,
  goTo,
}: {
  p: IconProject;
  ov: Overview;
  act: Act;
  hero: IconHero | null;
  slots: IconSlot[];
  goTo: (view: "library" | "settings", modId?: string) => void;
}) {
  const [name, setName] = useState(p.name);
  const [busy, setBusy] = useState<string | null>(null);
  const [result, setResult] = useState<IconBuildResult | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const available = slots.filter((s) => hero?.images.some(([f]) => f === s.field));
  const card = p.images.m_strIconHeroCard;
  const builtMod = p.modId ? ov.mods.find((m) => m.id === p.modId) : null;

  const choose = async (field: string) => {
    const picked = await open({ multiple: false, filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp", "tga", "bmp"] }] });
    if (typeof picked === "string") await act(() => api.updateIconProject(p.id, undefined, field, picked));
  };

  const fillFromCard = async () => {
    if (!card) return;
    await act(async () => {
      for (const s of available) if (!p.images[s.field] && s.field !== "m_strIconHeroCardCritical" && s.field !== "m_strIconHeroCardGloat") await api.updateIconProject(p.id, undefined, s.field, card);
    }, "The other icons now use your card picture, cropped to fit");
  };

  const build = async () => {
    setBusy("Starting…");
    const un = isTauri ? await listen<string>("icon-progress", (e) => setBusy(e.payload)) : () => undefined;
    try {
      let r: IconBuildResult | null = null;
      const ok = await act(async () => {
        r = await api.buildIconProject(p.id);
      });
      if (ok && r) setResult(r);
    } finally {
      un();
      setBusy(null);
    }
  };

  return (
    <div className="model-editor">
      <div className="editor-head">
        <input className="title-input" value={name} onChange={(e) => setName(e.target.value)} onBlur={() => name !== p.name && act(() => api.updateIconProject(p.id, name))} aria-label="Name" />
        <span className="hint">{hero ? hero.name : p.hero}</span>
      </div>

      {!hero && <p className="hint">Loading the hero's images…</p>}

      <div className="slot-grid">
        {available.map((s) => {
          const src = p.images[s.field];
          return (
            <div key={s.field} className={`slot-card ${src ? "set" : ""}`}>
              <SlotPreview field={s.field} source={src} slot={s} />
              <div className="slot-info">
                <strong>{s.label}</strong>
                <span className="hint">
                  {s.width} × {s.height}. {HINT[s.field]}
                </span>
                {src && <code className="slot-file">{src.split(/[\\/]/).pop()}</code>}
                <div className="row-actions">
                  <button className="btn small" onClick={() => choose(s.field)}>
                    {src ? "Change…" : "Choose picture…"}
                  </button>
                  {src && (
                    <button className="btn ghost small" onClick={() => act(() => api.updateIconProject(p.id, undefined, s.field, null))}>
                      Use the game's
                    </button>
                  )}
                </div>
              </div>
            </div>
          );
        })}
      </div>

      <div className="build-bar">
        <div>
          {builtMod ? (
            <>
              <strong>
                In your library as "{builtMod.name}" (v{builtMod.version})
              </strong>
              <span>Rebuilding updates that mod. Slots left on "Game's own" keep Valve's pictures.</span>
            </>
          ) : (
            <>
              <strong>Not built yet</strong>
              <span>Pictures are cropped to each slot's shape, as the previews show. Slots left empty keep Valve's.</span>
            </>
          )}
        </div>
        {card && available.some((s) => !p.images[s.field] && !s.field.includes("Card")) && (
          <button className="btn ghost" onClick={fillFromCard} title="Crop your card picture for the small, tall and minimap icons">
            Fill icons from card
          </button>
        )}
        <button className="btn primary" disabled={!ov.csdkFound || !Object.keys(p.images).length || !!busy} onClick={build}>
          {p.lastBuilt ? "Rebuild" : "Build"}
        </button>
        {builtMod && (
          <button className="btn ghost" onClick={() => goTo("library", builtMod.id)}>
            Show in Library
          </button>
        )}
      </div>

      <div className="danger-zone">
        {confirmDelete ? (
          <span className="confirm">
            Delete these portraits? (The built mod stays in your library.)
            <button className="btn danger small" onClick={() => act(() => api.deleteIconProject(p.id))}>
              Delete
            </button>
            <button className="btn ghost small" onClick={() => setConfirmDelete(false)}>
              Cancel
            </button>
          </span>
        ) : (
          <button className="btn ghost danger-text small" onClick={() => setConfirmDelete(true)}>
            <Icon name="trash" size={14} /> Delete project
          </button>
        )}
      </div>

      {busy && (
        <div className="modal-backdrop">
          <div className="modal progress-modal" role="status">
            <div className="spinner" />
            <strong>Building portraits</strong>
            <span>{busy}</span>
          </div>
        </div>
      )}
      {result && (
        <Modal
          title="Portraits built"
          onClose={() => setResult(null)}
          footer={
            <>
              <span className="hint">Switch it on, Apply & launch, and check the top bar in game.</span>
              <button className="btn" onClick={() => setResult(null)}>
                Close
              </button>
              <button className="btn primary" onClick={() => goTo("library", result.modId)}>
                Go to Library
              </button>
            </>
          }
        >
          <p>
            "{result.modName}" is in your library ({formatSize(result.report.vpkSize)}). Its pictures replace:
          </p>
          <ul className="file-list">
            {result.report.files.map((f) => (
              <li key={f}>
                <code>{f}</code>
              </li>
            ))}
          </ul>
          <details>
            <summary>Compiler output</summary>
            <pre className="log-pre">{result.report.log}</pre>
          </details>
        </Modal>
      )}
    </div>
  );
}
