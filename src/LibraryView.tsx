import { useEffect, useMemo, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import type { ModEntry, ModKind, Overview, Pack } from "./types";
import { Icon, Modal, Toggle, formatDate, formatSize, kindLabel } from "./ui";

type Act = (fn: () => Promise<unknown>, ok?: string) => Promise<boolean>;

export type Verify = "verified" | "stale" | "untested";

export function verifyState(m: ModEntry, build: string | null): Verify {
  if (!m.verifiedBuild) return "untested";
  return build && m.verifiedBuild === build ? "verified" : "stale";
}

const verifyText: Record<Verify, string> = {
  verified: "Verified",
  stale: "Needs testing",
  untested: "Untested",
};

const KINDS: (ModKind | "all")[] = ["all", "sound", "skin", "ui", "other"];

type Block = { pack: Pack | null; mods: ModEntry[] };

/** Top-level blocks of the load order: a pack (its mods together) or one loose mod. */
function blocksOf(mods: ModEntry[], packs: Pack[]): Block[] {
  const out: Block[] = [];
  for (const m of mods) {
    const p = packs.find((x) => x.mods.includes(m.id)) ?? null;
    const last = out[out.length - 1];
    if (p && last?.pack?.id === p.id) last.mods.push(m);
    else out.push({ pack: p, mods: [m] });
  }
  return out;
}

export function LibraryView({
  ov,
  act,
  selectedId,
  select,
  onImport,
}: {
  ov: Overview;
  act: Act;
  selectedId: string | null;
  select: (id: string | null) => void;
  onImport: () => void;
}) {
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<ModKind | "all">("all");
  const [dragId, setDragId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);
  const [newPack, setNewPack] = useState<string[] | null>(null);

  const packOf = useMemo(() => {
    const m = new Map<string, Pack>();
    for (const p of ov.packs) for (const id of p.mods) m.set(id, p);
    return m;
  }, [ov.packs]);
  const isActive = (m: ModEntry) => m.enabled && (packOf.get(m.id)?.enabled ?? true);

  const slots = useMemo(() => {
    const m = new Map<string, number>();
    ov.mods.filter((x) => x.enabled && (packOf.get(x.id)?.enabled ?? true)).forEach((x, i) => m.set(x.id, i + 1));
    return m;
  }, [ov.mods, packOf]);

  const conflictCount = useMemo(() => {
    const m = new Map<string, number>();
    for (const c of ov.conflicts) for (const id of c.mods) m.set(id, (m.get(id) ?? 0) + 1);
    return m;
  }, [ov.conflicts]);

  const filtering = query.trim() !== "" || kind !== "all";
  const matches = (m: ModEntry) => {
    if (kind !== "all" && m.kind !== kind) return false;
    const q = query.trim().toLowerCase();
    if (!q) return true;
    return [m.name, m.hero, m.notes, m.originalName, packOf.get(m.id)?.name ?? ""].some((s) => s.toLowerCase().includes(q));
  };
  const shown = ov.mods.filter(matches);
  const blocks = blocksOf(shown, ov.packs);
  const allBlocks = blocksOf(ov.mods, ov.packs);

  const activeCount = ov.mods.filter(isActive).length;
  const selected = ov.mods.find((m) => m.id === selectedId) ?? null;
  const selectedPack = selectedId?.startsWith("pack:") ? ov.packs.find((p) => `pack:${p.id}` === selectedId) ?? null : null;

  const move = (id: string, toIndex: number) => {
    const ids = ov.mods.map((m) => m.id);
    const from = ids.indexOf(id);
    if (from < 0 || toIndex < 0 || toIndex >= ids.length || from === toIndex) return;
    ids.splice(from, 1);
    ids.splice(toIndex, 0, id);
    void act(() => api.reorder(ids));
  };

  /** Move a whole pack to where block `to` is now. */
  const moveBlock = (packId: string, to: number) => {
    const bs = [...allBlocks];
    const from = bs.findIndex((b) => b.pack?.id === packId);
    if (from < 0 || to < 0 || to >= bs.length || from === to) return;
    const [b] = bs.splice(from, 1);
    bs.splice(to, 0, b);
    void act(() => api.reorder(bs.flatMap((x) => x.mods.map((m) => m.id))));
  };
  const blockIndexOf = (modId: string) => allBlocks.findIndex((b) => b.mods.some((m) => m.id === modId));

  const drop = (targetModId: string) => {
    if (!dragId) return;
    if (dragId.startsWith("pack:")) moveBlock(dragId.slice(5), blockIndexOf(targetModId));
    else move(dragId, ov.mods.findIndex((m) => m.id === targetModId));
  };

  const dragProps = (id: string, targetModId: string) => ({
    draggable: !filtering,
    onDragStart: (e: React.DragEvent) => {
      e.stopPropagation();
      setDragId(id);
      e.dataTransfer.effectAllowed = "move";
    },
    onDragOver: (e: React.DragEvent) => {
      if (!dragId) return;
      e.preventDefault();
      e.stopPropagation();
      setOverId(targetModId);
    },
    onDragEnd: () => {
      setDragId(null);
      setOverId(null);
    },
    onDrop: (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      drop(targetModId);
      setDragId(null);
      setOverId(null);
    },
  });

  const row = (m: ModEntry, inPack: Pack | null) => {
    const idx = ov.mods.indexOf(m);
    const v = verifyState(m, ov.game.buildId);
    const slot = slots.get(m.id);
    const conflicts = conflictCount.get(m.id) ?? 0;
    const first = inPack ? inPack.mods[0] === m.id : idx === 0;
    const last = inPack ? inPack.mods[inPack.mods.length - 1] === m.id : idx === ov.mods.length - 1;
    // Loose mods step over a whole pack; mods in a pack stay inside it.
    const step = (dir: -1 | 1) => {
      if (inPack) return move(m.id, idx + dir);
      const b = blockIndexOf(m.id) + dir;
      const target = allBlocks[b];
      if (!target) return;
      move(m.id, dir < 0 ? ov.mods.indexOf(target.mods[0]) : ov.mods.indexOf(target.mods[target.mods.length - 1]));
    };
    return (
      <li
        key={m.id}
        className={`mod-row ${isActive(m) ? "enabled" : ""} ${inPack ? "in-pack" : ""} ${selectedId === m.id ? "selected" : ""} ${
          overId === m.id && dragId !== m.id ? "drop-target" : ""
        } ${dragId === m.id ? "dragging" : ""}`}
        {...dragProps(m.id, m.id)}
        onClick={() => select(selectedId === m.id ? null : m.id)}
      >
        <span className={`grip ${filtering ? "hidden" : ""}`} title="Drag to change load order">
          <Icon name="grip" />
        </span>
        <span className="slot" title={slot ? `Loads as pak${String(slot).padStart(2, "0")}_dir.vpk` : inPack && !inPack.enabled && m.enabled ? "Off with its pack" : "Off"}>
          {slot ? String(slot).padStart(2, "0") : "—"}
        </span>
        <Toggle on={m.enabled} label={`${m.enabled ? "Turn off" : "Turn on"} ${m.name}`} onChange={(on) => act(() => api.setEnabled(m.id, on))} />
        <div className="mod-main">
          <div className="mod-name">{m.name}</div>
          <div className="mod-meta">
            <span className={`kind kind-${m.kind}`}>{kindLabel[m.kind]}</span>
            {m.hero && <span>{m.hero}</span>}
            <span>v{m.version}</span>
            <span>{formatSize(m.size)}</span>
          </div>
        </div>
        {conflicts > 0 ? (
          <span className="conflict" title={`Shares ${conflicts} file(s) with another enabled mod`}>
            <Icon name="warn" size={14} /> {conflicts}
          </span>
        ) : (
          <span />
        )}
        <span className={`verify v-${v}`}>{verifyText[v]}</span>
        {!filtering ? (
          <span className="order-btns">
            <button className="icon-btn" aria-label={`Move ${m.name} up`} disabled={first} onClick={(e) => (e.stopPropagation(), step(-1))}>
              <Icon name="up" size={14} />
            </button>
            <button className="icon-btn" aria-label={`Move ${m.name} down`} disabled={last} onClick={(e) => (e.stopPropagation(), step(1))}>
              <Icon name="down" size={14} />
            </button>
          </span>
        ) : (
          <span />
        )}
      </li>
    );
  };

  return (
    <div className="library">
      <div className="library-main">
        <div className="view-head">
          <div>
            <h1>Library</h1>
            <p className="sub">
              {ov.mods.length} mod{ov.mods.length === 1 ? "" : "s"}
              {ov.packs.length ? ` in ${ov.packs.length} pack${ov.packs.length === 1 ? "" : "s"} and loose` : ""} · {activeCount} on. Top of the list loads first (pak01).
            </p>
          </div>
          <div className="head-actions">
            <button
              className="btn ghost"
              disabled={activeCount === 0}
              onClick={() => act(() => api.setAllEnabled(false), "All mods switched off. Apply to update the game.")}
            >
              All off
            </button>
            <button className="btn" disabled={ov.mods.length === 0} onClick={() => setNewPack([])} title="Group mods into a pack with one switch">
              <Icon name="layers" /> New pack
            </button>
            <button className="btn primary" onClick={onImport}>
              <Icon name="plus" /> Import
            </button>
          </div>
        </div>

        <div className="filters">
          <label className="search">
            <Icon name="search" size={15} />
            <input placeholder="Search name, hero, pack or notes" value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Search mods" />
          </label>
          <div className="chips" role="group" aria-label="Filter by type">
            {KINDS.map((k) => (
              <button key={k} className={`chip ${kind === k ? "active" : ""}`} onClick={() => setKind(k)}>
                {k === "all" ? "All" : kindLabel[k]}
              </button>
            ))}
          </div>
        </div>

        {ov.mods.length === 0 ? (
          <div className="empty">
            <div className="empty-mark">
              <Icon name="layers" size={34} />
            </div>
            <h2>No mods yet</h2>
            <p>
              Import a VPK you've built or a mod pack, or drag one onto this window. It's stored in the app's library, and nothing in the game folder
              changes until you apply.
            </p>
            <button className="btn primary" onClick={onImport}>
              <Icon name="plus" /> Import
            </button>
          </div>
        ) : (
          <ol className="mod-list" aria-label="Mods in load order">
            {blocks.map((b) => {
              if (!b.pack) return row(b.mods[0], null);
              const p = b.pack;
              const members = ov.mods.filter((m) => p.mods.includes(m.id));
              const on = members.filter((m) => m.enabled).length;
              const bi = allBlocks.findIndex((x) => x.pack?.id === p.id);
              const key = `pack:${p.id}`;
              return (
                <li key={key} className={`pack-block ${p.enabled ? "enabled" : ""} ${selectedId === key ? "selected" : ""} ${dragId === key ? "dragging" : ""}`}>
                  <div
                    className={`pack-head ${overId === members[0]?.id && dragId && dragId !== key ? "drop-target" : ""}`}
                    {...dragProps(key, members[0].id)}
                    onClick={() => select(selectedId === key ? null : key)}
                  >
                    <span className={`grip ${filtering ? "hidden" : ""}`} title="Drag to move the whole pack">
                      <Icon name="grip" />
                    </span>
                    <span className="pack-mark">
                      <Icon name="layers" size={16} />
                    </span>
                    <Toggle on={p.enabled} label={`${p.enabled ? "Turn off" : "Turn on"} the pack ${p.name}`} onChange={(v) => act(() => api.setPackEnabled(p.id, v))} />
                    <div className="mod-main">
                      <div className="mod-name">{p.name}</div>
                      <div className="mod-meta">
                        <span className="kind">Pack</span>
                        <span>
                          {members.length} mod{members.length === 1 ? "" : "s"} · {on} on
                        </span>
                        <span>{formatSize(members.reduce((n, m) => n + m.size, 0))}</span>
                      </div>
                    </div>
                    {!filtering ? (
                      <span className="order-btns">
                        <button className="icon-btn" aria-label={`Move pack ${p.name} up`} disabled={bi === 0} onClick={(e) => (e.stopPropagation(), moveBlock(p.id, bi - 1))}>
                          <Icon name="up" size={14} />
                        </button>
                        <button className="icon-btn" aria-label={`Move pack ${p.name} down`} disabled={bi === allBlocks.length - 1} onClick={(e) => (e.stopPropagation(), moveBlock(p.id, bi + 1))}>
                          <Icon name="down" size={14} />
                        </button>
                      </span>
                    ) : (
                      <span />
                    )}
                  </div>
                  <ol className="pack-mods">{b.mods.map((m) => row(m, p))}</ol>
                </li>
              );
            })}
            {shown.length === 0 && <li className="no-match">No mods match that search.</li>}
          </ol>
        )}
      </div>

      {selected && <ModDrawer key={selected.id} m={selected} ov={ov} act={act} close={() => select(null)} newPack={(ids) => setNewPack(ids)} />}
      {selectedPack && <PackDrawer key={selectedPack.id} p={selectedPack} ov={ov} act={act} close={() => select(null)} selectMod={select} />}
      {newPack && (
        <NewPackModal
          ov={ov}
          initial={newPack}
          close={() => setNewPack(null)}
          create={async (name, ids) => {
            let made: Pack | null = null;
            const ok = await act(async () => {
              made = await api.createPack(name, ids);
            }, `Pack "${name}" made. One switch now turns all ${ids.length} on or off.`);
            if (ok && made) {
              setNewPack(null);
              select(`pack:${(made as Pack).id}`);
            }
          }}
        />
      )}
    </div>
  );
}

function NewPackModal({ ov, initial, close, create }: { ov: Overview; initial: string[]; close: () => void; create: (name: string, ids: string[]) => void }) {
  const [name, setName] = useState(() => {
    const heroes = [...new Set(ov.mods.filter((m) => initial.includes(m.id)).map((m) => m.hero).filter(Boolean))];
    return heroes.length === 1 ? `${heroes[0]} pack` : "";
  });
  const [ids, setIds] = useState<string[]>(initial);
  const packName = (id: string) => ov.packs.find((p) => p.mods.includes(id))?.name;
  const toggle = (id: string) => setIds((x) => (x.includes(id) ? x.filter((y) => y !== id) : [...x, id]));
  return (
    <Modal
      title="New mod pack"
      onClose={close}
      footer={
        <>
          <span className="hint">The pack's switch turns them all on or off. Each mod keeps its own switch too.</span>
          <button className="btn ghost" onClick={close}>
            Cancel
          </button>
          <button className="btn primary" disabled={!name.trim() || ids.length === 0} onClick={() => create(name.trim(), ov.mods.filter((m) => ids.includes(m.id)).map((m) => m.id))}>
            Make pack ({ids.length})
          </button>
        </>
      }
    >
      <label className="form">
        Name
        <input autoFocus value={name} placeholder="e.g. Full Seven" onChange={(e) => setName(e.target.value)} />
      </label>
      <div>
        <h3>Mods</h3>
        <ul className="pick-list">
          {ov.mods.map((m) => (
            <li key={m.id}>
              <label>
                <input type="checkbox" checked={ids.includes(m.id)} onChange={() => toggle(m.id)} />
                <span className="pick-name">{m.name}</span>
                <span className="hint">
                  {kindLabel[m.kind]}
                  {m.hero ? ` · ${m.hero}` : ""}
                  {packName(m.id) ? ` · moves from "${packName(m.id)}"` : ""}
                </span>
              </label>
            </li>
          ))}
        </ul>
      </div>
    </Modal>
  );
}

function PackDrawer({ p, ov, act, close, selectMod }: { p: Pack; ov: Overview; act: Act; close: () => void; selectMod: (id: string) => void }) {
  const [name, setName] = useState(p.name);
  const [notes, setNotes] = useState(p.notes);
  const [confirm, setConfirm] = useState<null | "ungroup" | "delete">(null);
  const members = ov.mods.filter((m) => p.mods.includes(m.id));
  const dirty = name !== p.name || notes !== p.notes;

  const exportPack = async () => {
    const safe = p.name.replace(/[^A-Za-z0-9 _-]/g, "").trim() || "pack";
    const path = await save({ defaultPath: `${safe}.dmpack`, filters: [{ name: "Mod pack", extensions: ["dmpack"] }] });
    if (path) await act(() => api.exportPack(p.id, path), `Exported "${p.name}" (${members.length} mods in one file)`);
  };

  return (
    <aside className="drawer" aria-label={`Pack ${p.name}`}>
      <header className="drawer-head">
        <h2>{p.name}</h2>
        <button className="icon-btn" onClick={close} aria-label="Close details">
          <Icon name="close" />
        </button>
      </header>
      <div className="drawer-body">
        <section className="block">
          <div className="pack-switch">
            <Toggle on={p.enabled} label={`${p.enabled ? "Turn off" : "Turn on"} the pack`} onChange={(v) => act(() => api.setPackEnabled(p.id, v))} />
            <div>
              <strong>{p.enabled ? "Pack on" : "Pack off"}</strong>
              <p className="hint">{p.enabled ? "Its mods load if their own switch is on." : "None of its mods load, whatever their own switch says."}</p>
            </div>
          </div>
        </section>
        <form
          className="form"
          onSubmit={(e) => {
            e.preventDefault();
            void act(() => api.updatePack(p.id, name, notes), "Saved");
          }}
        >
          <label>
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <label>
            Notes
            <textarea rows={3} value={notes} placeholder="What's in it, credits" onChange={(e) => setNotes(e.target.value)} />
          </label>
          <div className="form-actions">
            <button className="btn primary small" type="submit" disabled={!dirty}>
              Save details
            </button>
          </div>
        </form>
        <section className="block">
          <h3>Mods in this pack ({members.length})</h3>
          <ul className="pack-member-list">
            {members.map((m) => (
              <li key={m.id}>
                <button className="link" onClick={() => selectMod(m.id)}>
                  {m.name}
                </button>
                <span className="hint">
                  {kindLabel[m.kind]} · {m.enabled ? "on" : "off"}
                </span>
                <button className="btn ghost small" onClick={() => act(() => api.setModPack(m.id, null), `"${m.name}" is now a loose mod`)}>
                  Take out
                </button>
              </li>
            ))}
          </ul>
          <p className="hint">To add a mod, open it and pick this pack, or drag it next to the pack's mods.</p>
        </section>
        <div className="drawer-actions">
          <button className="btn" onClick={exportPack}>
            <Icon name="up" size={14} /> Export pack…
          </button>
          {confirm === null && (
            <>
              <button className="btn ghost" onClick={() => setConfirm("ungroup")}>
                Ungroup
              </button>
              <button className="btn ghost danger-text" onClick={() => setConfirm("delete")}>
                <Icon name="trash" size={14} /> Delete
              </button>
            </>
          )}
          {confirm && (
            <span className="confirm">
              {confirm === "ungroup" ? "Ungroup? The mods stay as loose mods." : `Delete the pack and its ${members.length} mods from the library?`}
              <button
                className={`btn small ${confirm === "delete" ? "danger" : "primary"}`}
                onClick={async () => {
                  const ok = await act(
                    () => api.deletePack(p.id, confirm === "delete"),
                    confirm === "delete" ? `Deleted "${p.name}" and its mods. Apply to update the game.` : `Ungrouped "${p.name}"`,
                  );
                  if (ok) close();
                }}
              >
                {confirm === "delete" ? "Delete" : "Ungroup"}
              </button>
              <button className="btn ghost small" onClick={() => setConfirm(null)}>
                Cancel
              </button>
            </span>
          )}
        </div>
      </div>
    </aside>
  );
}

function ModDrawer({ m, ov, act, close, newPack }: { m: ModEntry; ov: Overview; act: Act; close: () => void; newPack: (ids: string[]) => void }) {
  const [name, setName] = useState(m.name);
  const [kind, setKind] = useState<ModKind>(m.kind);
  const [hero, setHero] = useState(m.hero);
  const [version, setVersion] = useState(m.version);
  const [notes, setNotes] = useState(m.notes);
  const [showAll, setShowAll] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);

  useEffect(() => setConfirmRemove(false), [m.id]);

  const dirty =
    name !== m.name || kind !== m.kind || hero !== m.hero || version !== m.version || notes !== m.notes;
  const v = verifyState(m, ov.game.buildId);
  const shared = ov.conflicts.filter((c) => c.mods.includes(m.id));
  const nameOf = (id: string) => ov.mods.find((x) => x.id === id)?.name ?? "Unknown";
  const files = showAll ? m.files : m.files.slice(0, 40);

  const replace = async () => {
    const picked = await open({ multiple: false, filters: [{ name: "VPK", extensions: ["vpk"] }] });
    if (typeof picked === "string") await act(() => api.replaceModFile(m.id, picked), "VPK replaced. Apply to update the game.");
  };

  return (
    <aside className="drawer" aria-label={`Details for ${m.name}`}>
      <header className="drawer-head">
        <h2>{m.name}</h2>
        <button className="icon-btn" onClick={close} aria-label="Close details">
          <Icon name="close" />
        </button>
      </header>

      <div className="drawer-body">
        <section className={`verify-card v-${v}`}>
          <div>
            <strong>{verifyText[v]}</strong>
            <p>
              {v === "verified" && `Confirmed working on build ${m.verifiedBuild}.`}
              {v === "stale" && `Last confirmed on build ${m.verifiedBuild}. Deadlock is now on ${ov.game.buildId ?? "an unknown build"}.`}
              {v === "untested" && "Not confirmed in game yet."}
            </p>
          </div>
          {v === "verified" ? (
            <button className="btn ghost small" onClick={() => act(() => api.markVerified(m.id, false))}>
              Clear
            </button>
          ) : (
            <button
              className="btn small"
              disabled={!ov.game.buildId}
              onClick={() => act(() => api.markVerified(m.id, true), `Marked verified on build ${ov.game.buildId}`)}
            >
              <Icon name="check" size={14} /> Works on this build
            </button>
          )}
        </section>

        <form
          className="form"
          onSubmit={(e) => {
            e.preventDefault();
            void act(() => api.updateMod(m.id, { name, kind, hero, version, notes }), "Saved");
          }}
        >
          <label>
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <div className="form-row">
            <label>
              Type
              <select value={kind} onChange={(e) => setKind(e.target.value as ModKind)}>
                <option value="sound">Sound</option>
                <option value="skin">Skin</option>
                <option value="ui">UI</option>
                <option value="other">Other</option>
              </select>
            </label>
            <label>
              Hero
              <input value={hero} placeholder="e.g. Seven" onChange={(e) => setHero(e.target.value)} />
            </label>
            <label className="narrow">
              Version
              <input value={version} onChange={(e) => setVersion(e.target.value)} />
            </label>
          </div>
          <label>
            Notes
            <textarea rows={3} value={notes} placeholder="What it changes, credits, to-dos" onChange={(e) => setNotes(e.target.value)} />
          </label>
          <div className="form-actions">
            <button className="btn primary small" type="submit" disabled={!dirty}>
              Save details
            </button>
          </div>
        </form>

        <section className="block">
          <h3>Pack</h3>
          <div className="pack-pick">
            <select
              value={ov.packs.find((p) => p.mods.includes(m.id))?.id ?? ""}
              onChange={(e) => {
                const v = e.target.value;
                if (v === "__new") newPack([m.id]);
                else void act(() => api.setModPack(m.id, v || null), v ? `Added to "${ov.packs.find((p) => p.id === v)?.name}"` : "Now a loose mod");
              }}
            >
              <option value="">Not in a pack</option>
              {ov.packs.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
              <option value="__new">New pack…</option>
            </select>
          </div>
        </section>

        {shared.length > 0 && (
          <section className="block warn-block">
            <h3>
              <Icon name="warn" size={14} /> Shared files
            </h3>
            <p className="hint">
              These files are also changed by another enabled mod. Only one version can load; check in game which one
              wins.
            </p>
            <ul className="mono-list">
              {shared.slice(0, 12).map((c) => (
                <li key={c.path}>
                  <code>{c.path}</code>
                  <span>also in {c.mods.filter((id) => id !== m.id).map(nameOf).join(", ")}</span>
                </li>
              ))}
            </ul>
          </section>
        )}

        <section className="block">
          <h3>Files inside ({m.files.length})</h3>
          <ul className="mono-list">
            {files.map((f) => (
              <li key={f}>
                <code>{f}</code>
              </li>
            ))}
          </ul>
          {m.files.length > 40 && !showAll && (
            <button className="link" onClick={() => setShowAll(true)}>
              Show all {m.files.length}
            </button>
          )}
        </section>

        <section className="block facts">
          <div>
            <span>Imported from</span>
            <code>{m.originalName}</code>
          </div>
          <div>
            <span>Added</span>
            {formatDate(m.addedAt)}
          </div>
          <div>
            <span>Updated</span>
            {formatDate(m.updatedAt)}
          </div>
        </section>

        <div className="drawer-actions">
          <button className="btn" onClick={replace}>
            <Icon name="swap" size={14} /> Replace VPK…
          </button>
          {confirmRemove ? (
            <span className="confirm">
              Remove from library?
              <button
                className="btn danger small"
                onClick={async () => {
                  if (await act(() => api.removeMod(m.id), `Removed "${m.name}". Apply to update the game.`)) close();
                }}
              >
                Remove
              </button>
              <button className="btn ghost small" onClick={() => setConfirmRemove(false)}>
                Cancel
              </button>
            </span>
          ) : (
            <button className="btn ghost danger-text" onClick={() => setConfirmRemove(true)}>
              <Icon name="trash" size={14} /> Remove
            </button>
          )}
        </div>
      </div>
    </aside>
  );
}
