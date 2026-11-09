import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorText, isTauri } from "./api";
import type { ModelBuildResult, ModelHero, ModelProject, ModelSummary, Overview, Returned } from "./types";
import { formatDate, formatSize, Icon, Modal } from "./ui";

type Act = (fn: () => Promise<unknown>, ok?: string) => Promise<boolean>;

/** Hero models from the game's data (loaded once). */
function useModelHeroes() {
  const [heroes, setHeroes] = useState<ModelHero[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api
      .modelHeroes()
      .then((h) => setHeroes([...h].sort((a, b) => a.name.localeCompare(b.name))))
      .catch((e) => setError(errorText(e)));
  }, []);
  return { heroes, error };
}

function shortPath(p: string) {
  return (p.split("/").pop() ?? p).replace(/\.(vmat|vmdl)$/, "");
}

function status(p: ModelProject) {
  if (p.lastBuilt) return "built";
  if (p.extracted) return "extracted";
  return "not extracted";
}

export function ModelBuilderView({
  ov,
  act,
  goTo,
}: {
  ov: Overview;
  act: Act;
  goTo: (view: "library" | "settings", modId?: string) => void;
}) {
  const [selectedId, setSelectedId] = useState<string | null>(ov.modelProjects[0]?.id ?? null);
  const [newName, setNewName] = useState("");
  const [newHero, setNewHero] = useState("");
  const { heroes, error } = useModelHeroes();
  const project = ov.modelProjects.find((p) => p.id === selectedId) ?? null;
  const heroName = (code: string) => heroes?.find((h) => h.code === code)?.name ?? code;

  useEffect(() => {
    if (!project && ov.modelProjects.length) setSelectedId(ov.modelProjects[0].id);
  }, [project, ov.modelProjects]);

  return (
    <div className="builder">
      <aside className="builder-list">
        <div className="builder-list-head">
          <h1>Model Builder</h1>
          <p className="sub">Replace a hero's model with your own.</p>
        </div>
        <ul className="project-list">
          {ov.modelProjects.map((p) => (
            <li key={p.id}>
              <button className={`project-item ${p.id === selectedId ? "active" : ""}`} onClick={() => setSelectedId(p.id)}>
                <span className="pi-name">{p.name}</span>
                <span className="pi-meta">
                  {heroName(p.hero)} · {status(p)}
                </span>
              </button>
            </li>
          ))}
        </ul>
        <form
          className="new-project"
          onSubmit={async (e) => {
            e.preventDefault();
            let created: ModelProject | null = null;
            const ok = await act(async () => {
              created = await api.createModelProject(newName, newHero);
            });
            if (ok && created) {
              setSelectedId((created as ModelProject).id);
              setNewName("");
            }
          }}
        >
          <strong>New model mod</strong>
          <input value={newName} placeholder="Name, e.g. Seven in a suit" onChange={(e) => setNewName(e.target.value)} aria-label="New model mod name" />
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
            <button className="btn small" onClick={() => goTo("settings")}>Open Settings</button>
          </div>
        )}
        {project ? (
          <ModelEditor key={project.id} p={project} ov={ov} act={act} heroName={heroName(project.hero)} goTo={goTo} />
        ) : (
          <div className="empty">
            <div className="empty-mark">
              <Icon name="layers" size={30} />
            </div>
            <h2>Make your first model mod</h2>
            <ol className="steps">
              <li>Name it in <strong>New model mod</strong> on the left, pick the hero and click <strong>Create</strong>.</li>
              <li>Click <strong>Extract</strong>. The app takes the hero's model apart and makes a file for Blender.</li>
              <li>Click <strong>Edit in Blender</strong>. Edit the model, then use <strong>Send to app</strong> in Blender's Deadlock tab.</li>
            </ol>
          </div>
        )}
      </section>
    </div>
  );
}

function ModelEditor({
  p,
  ov,
  act,
  heroName,
  goTo,
}: {
  p: ModelProject;
  ov: Overview;
  act: Act;
  heroName: string;
  goTo: (view: "library" | "settings", modId?: string) => void;
}) {
  const [name, setName] = useState(p.name);
  const [busy, setBusy] = useState<{ title: string; step: string } | null>(null);
  const [summary, setSummary] = useState<ModelSummary | null>(null);
  const [result, setResult] = useState<ModelBuildResult | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [returns, setReturns] = useState<Returned[]>([]);

  useEffect(() => {
    if (!p.extracted) return setSummary(null);
    api.modelSummary(p.id).then(setSummary).catch(() => setSummary(null));
  }, [p.id, p.extracted]);

  // Models sent back from Blender: check now and whenever the app regains focus.
  useEffect(() => {
    const load = () => api.modelReturns(p.id).then(setReturns).catch(() => undefined);
    void load();
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  }, [p.id]);
  const latest = returns[0] ?? null;

  const builtMod = ov.mods.find((m) => m.id === p.modId) ?? null;
  const outdated = !!p.extracted && !!p.extractedBuild && !!ov.game.buildId && p.extractedBuild !== ov.game.buildId;
  const oldTool = !!p.extracted && p.extractedTool !== ov.s2vVersion;

  const run = async (title: string, fn: () => Promise<void>) => {
    setBusy({ title, step: "Starting…" });
    const un = isTauri ? await listen<string>("model-progress", (e) => setBusy({ title, step: e.payload })) : () => undefined;
    try {
      await fn();
    } finally {
      un();
      setBusy(null);
    }
  };

  const extract = () =>
    run(`Extracting ${heroName}`, async () => {
      let cloth = false;
      const ok = await act(async () => {
        const r = await api.extractModelProject(p.id);
        setSummary(r.report.summary);
        cloth = r.report.clothMissing;
      }, `${heroName} extracted`);
      if (ok && cloth) void act(() => Promise.reject(`${heroName}'s cloth couldn't be rebuilt from the original, so it won't move in this build.`));
    });

  const [confirmFresh, setConfirmFresh] = useState(false);
  const build = (withModel: boolean) =>
    run(`Building "${p.name}"`, async () => {
      let r: ModelBuildResult | null = null;
      const ok = await act(async () => {
        r = await api.buildModelProject(p.id, withModel);
      });
      if (ok && r) setResult(r);
    });

  return (
    <div className="editor">
      <div className="editor-head">
        <div className="form-row">
          <label>
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} onBlur={() => name !== p.name && act(() => api.renameModelProject(p.id, name))} />
          </label>
          <label>
            Hero
            <input value={heroName} disabled title={p.model} />
          </label>
        </div>
        <div className="head-actions">
          {confirmDelete ? (
            <span className="confirm">
              Delete project and its files?
              <button className="btn danger small" onClick={() => act(() => api.deleteModelProject(p.id), `Deleted "${p.name}"`)}>Delete</button>
              <button className="btn ghost small" onClick={() => setConfirmDelete(false)}>Cancel</button>
            </span>
          ) : (
            <button className="icon-btn" aria-label="Delete project" title="Delete project (the built mod stays in your library)" onClick={() => setConfirmDelete(true)}>
              <Icon name="trash" />
            </button>
          )}
        </div>
      </div>

      <ol className="model-steps">
        <li className={`model-step ${p.extracted ? "done" : "current"}`}>
          <div className="ms-num">{p.extracted ? <Icon name="check" size={14} /> : 1}</div>
          <div className="ms-body">
            <strong>Extract {heroName}</strong>
            {p.extracted ? (
              <span className="hint">
                Extracted {formatDate(p.extracted)}
                {p.extractedBuild ? ` from game build ${p.extractedBuild}` : ""}.
              </span>
            ) : (
              <span className="hint">
                Takes the hero's model apart into an editable source, mesh files and a Blender file. About a minute.
                {!ov.s2vInstalled && " The first time, it downloads Source 2 Viewer's command-line tool (60 MB, MIT licence), built from its official source."}
              </span>
            )}
            {outdated && (
              <span className="ms-warn">
                <Icon name="warn" size={14} /> The game has updated since this was extracted. Extract again before building.
              </span>
            )}
            {!outdated && oldTool && (
              <span className="ms-warn">
                <Icon name="warn" size={14} /> Extracted with an older Source 2 Viewer that couldn't rebuild cloth. Extract again to get working cloth.
              </span>
            )}
          </div>
          <button className={`btn ${p.extracted && !outdated && !oldTool ? "" : "primary"}`} disabled={!!busy || !ov.game.found} onClick={extract}>
            {p.extracted ? "Extract again" : "Extract"}
          </button>
        </li>

        <li className={`model-step ${!p.extracted ? "later" : latest ? "done" : "current"}`}>
          <div className="ms-num">{latest ? <Icon name="check" size={14} /> : 2}</div>
          <div className="ms-body">
            <strong>Edit in Blender</strong>
            {ov.settings.blenderExe ? (
              <span className="hint">
                Opens Blender with {heroName} ready to edit. The first time, the app installs its Blender add-on and saves a
                work file. Use the <em>Deadlock</em> tab in the sidebar (press N) to assign parts, check the model and{" "}
                <em>Send to app</em>.
              </span>
            ) : (
              <span className="ms-warn">
                <Icon name="warn" size={14} /> Blender wasn't found. Set it in Settings (4.2 or newer).
              </span>
            )}
          </div>
          <div className="row-actions">
            {confirmFresh ? (
              <span className="confirm">
                Start over from the original?
                <button
                  className="btn danger small"
                  onClick={() => {
                    setConfirmFresh(false);
                    void act(() => api.editInBlender(p.id, true), "Re-importing in Blender. Your old work file is kept as a .bak.");
                  }}
                >
                  Re-import
                </button>
                <button className="btn ghost small" onClick={() => setConfirmFresh(false)}>Cancel</button>
              </span>
            ) : (
              <button className="btn ghost" disabled={!p.extracted || !ov.settings.blenderExe} onClick={() => setConfirmFresh(true)} title="Start a fresh Blender work file from the original hero (the old one is kept as a .bak)">
                Re-import
              </button>
            )}
            <button className="btn ghost" disabled={!p.extracted} onClick={() => act(() => api.openModelFolder(p.id, "blender"))} title="Open the project's blender folder">
              <Icon name="folder" size={14} />
            </button>
            <button
              className={`btn ${latest ? "" : "primary"}`}
              disabled={!p.extracted || !!busy || outdated}
              onClick={() =>
                ov.settings.blenderExe
                  ? act(() => api.editInBlender(p.id), "Opening Blender. The first time, it installs the add-on and imports the hero (about a minute).")
                  : goTo("settings")
              }
            >
              {ov.settings.blenderExe ? "Edit in Blender" : "Set up Blender"}
            </button>
          </div>
        </li>

        {latest && (
          <li className="model-step done">
            <div className="ms-num">
              <Icon name="check" size={14} />
            </div>
            <div className="ms-body">
              <strong>Back from Blender</strong>
              <span className="hint">
                Sent {formatDate(latest.sent)} {new Date(latest.sent).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}:{" "}
                {latest.objects.length} object(s), {latest.objects.reduce((n, o) => n + o.triangles, 0).toLocaleString()} triangles, parts{" "}
                {[...new Set(latest.objects.map((o) => o.part))].join(", ")}
                {latest.materialFiles?.some((m) => m.color && !m.new)
                  ? `; new textures for ${latest.materialFiles.filter((m) => m.color && !m.new).map((m) => m.name).join(", ")}`
                  : ""}
                {latest.materialFiles?.some((m) => m.new) ? `; new materials ${latest.materialFiles.filter((m) => m.new).map((m) => m.name).join(", ")}` : ""}
                .
              </span>
              <span className="hint">Parts you changed get new meshes. Parts you didn't touch keep Valve's original files.</span>
            </div>
          </li>
        )}

        <li className={`model-step ${!p.extracted ? "later" : p.lastBuilt ? "done" : "current"}`}>
          <div className="ms-num">{p.lastBuilt ? <Icon name="check" size={14} /> : latest ? 4 : 3}</div>
          <div className="ms-body">
            <strong>{latest ? "Build your model" : "Build test mod"}</strong>
            <span className="hint">
              {latest
                ? `Builds ${heroName} with your model from Blender: skeleton, animations, hitboxes, cloth and settings stay Valve's. Needs CSDK and Steam running.`
                : `Rebuilds ${heroName} unchanged from the extracted files, to prove the pipeline works in game. Cloth is rebuilt from the original's settings. Needs CSDK and Steam running.`}
            </span>
          </div>
          <div className="row-actions">
            {latest && (
              <button className="btn ghost" disabled={!p.extracted || !ov.csdkFound || !!busy || outdated} onClick={() => build(false)} title="Rebuild the original model instead">
                Original
              </button>
            )}
            <button className="btn primary" disabled={!p.extracted || !ov.csdkFound || !!busy || outdated} onClick={() => build(!!latest)}>
              {latest ? "Build with my model" : p.lastBuilt ? "Rebuild" : "Build test mod"}
            </button>
          </div>
        </li>
      </ol>

      {summary && <SummaryCard s={summary} heroName={heroName} openSource={() => act(() => api.openModelFolder(p.id, "original"))} />}

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
              <span>{p.extracted ? "Build the test mod, switch it on and check the hero in game." : "Extract the hero to start."}</span>
            </>
          )}
        </div>
        {builtMod && (
          <button className="btn ghost" onClick={() => goTo("library", builtMod.id)}>Show in Library</button>
        )}
      </div>

      {busy && (
        <div className="modal-backdrop">
          <div className="modal progress-modal" role="status">
            <div className="spinner" />
            <strong>{busy.title}</strong>
            <span>{busy.step}</span>
          </div>
        </div>
      )}
      {result && <ModelReportModal r={result} heroName={heroName} close={() => setResult(null)} goLibrary={() => goTo("library", result.modId)} />}
    </div>
  );
}

function SummaryCard({ s, heroName, openSource }: { s: ModelSummary; heroName: string; openSource: () => void }) {
  const stats: [string, string][] = [
    ["Bones", `${s.bones.length}`],
    ["Meshes", `${s.meshes} (${s.lods} detail levels)`],
    ["Triangles", `${Math.round(s.trianglesLod0 / 1000)}k at full detail`],
    ["Materials", `${s.materials.length}`],
    ["Attachment points", `${s.attachments.length}`],
    ["Hitboxes", `${s.hitboxes}`],
    ["Ragdoll bodies", `${s.ragdollBodies}`],
    ["Cloth points", s.clothNodes ? `${s.clothNodes}` : "none"],
  ];
  return (
    <section className="voice-card">
      <div className="items-head">
        <h3>What {heroName}'s model has</h3>
        <button className="btn ghost small" onClick={openSource}>
          <Icon name="folder" size={14} /> Model source
        </button>
      </div>
      <dl className="stat-grid">
        {stats.map(([k, v]) => (
          <div key={k}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
      {s.bodyGroups.length > 0 && (
        <p className="hint">
          <strong>Body groups:</strong> {s.bodyGroups.map((g) => g.replace(/^DEFAULT: /, "")).join(", ")}. A custom model needs a version for each.
        </p>
      )}
      <p className="hint">
        <strong>Materials:</strong> {s.materials.map(shortPath).join(", ")}
      </p>
      <details>
        <summary>Bone and attachment names</summary>
        <p className="hint mono-list">
          <strong>Bones:</strong> {s.bones.join(", ")}
        </p>
        <p className="hint mono-list">
          <strong>Attachments:</strong> {s.attachments.join(", ")}
        </p>
      </details>
    </section>
  );
}

function ModelReportModal({ r, heroName, close, goLibrary }: { r: ModelBuildResult; heroName: string; close: () => void; goLibrary: () => void }) {
  return (
    <Modal
      title="Model mod built"
      onClose={close}
      footer={
        <>
          <span className="hint">Switch it on, Apply & launch, and check {heroName} in game.</span>
          <button className="btn ghost" onClick={close}>Close</button>
          <button className="btn primary" onClick={() => { close(); goLibrary(); }}>Go to Library</button>
        </>
      }
    >
      <p>
        <strong>"{r.modName}"</strong>: {r.report.files.length} file(s), {formatSize(r.report.vpkSize)}.
      </p>
      {r.report.parts.length > 0 && (
        <table className="report">
          <thead>
            <tr>
              <th>Part</th>
              <th>Result</th>
              <th>Vertices</th>
              <th>Triangles</th>
            </tr>
          </thead>
          <tbody>
            {r.report.parts.map((x) => (
              <tr key={x.part}>
                <td>{x.part}</td>
                <td>{x.status === "replaced" ? "Your mesh" : x.status === "kept" ? "Unchanged (original kept)" : "Removed"}</td>
                <td>{x.status === "replaced" ? x.vertices.toLocaleString() : ""}</td>
                <td>{x.status === "replaced" ? x.triangles.toLocaleString() : ""}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {r.report.materials.length > 0 && (
        <table className="report">
          <thead>
            <tr>
              <th>Material</th>
              <th>Result</th>
              <th>Texture</th>
            </tr>
          </thead>
          <tbody>
            {r.report.materials.map((m) => (
              <tr key={m.path}>
                <td>{m.material}</td>
                <td>{m.new ? "New material (from the hero's main material)" : "Your colour texture"}</td>
                <td>
                  {m.size[0] <= 4 && m.new ? "Plain colour" : `${m.size[0]}×${m.size[1]}`}
                  {m.resized ? " (resized to a power of two)" : ""}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <p className="hint">What to check in game (Hero Labs or a bot match): {heroName} looks right, every animation plays, cloth moves, the gun hides during the ult, and the hero shows in the menus.</p>
      <ul className="file-list">
        {r.report.files.map((f) => (
          <li key={f}>
            <code>{f}</code>
          </li>
        ))}
      </ul>
      {r.report.warnings.length > 0 && (
        <>
          <h3>Compiler warnings ({r.report.warnings.length})</h3>
          {r.report.warnings.some((w) => /cloth/i.test(w)) && (
            <p className="hint">Cloth warnings come from Valve's own cloth setup and show up on the unchanged hero too. If the cloth moves in game, they're harmless.</p>
          )}
          <ul className="warn-lines">
            {r.report.warnings.map((w, i) => (
              <li key={i}>{w.replace(/^RESOURCE COMPILE WARNING:\s*/i, "")}</li>
            ))}
          </ul>
        </>
      )}
      <details>
        <summary>Compiler output</summary>
        <pre className="log-pre">{r.report.log}</pre>
      </details>
    </Modal>
  );
}
