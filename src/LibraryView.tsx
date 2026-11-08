import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import type { ModEntry, ModKind, Overview } from "./types";
import { Icon, Toggle, formatDate, formatSize, kindLabel } from "./ui";

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

  const slots = useMemo(() => {
    const m = new Map<string, number>();
    ov.mods.filter((x) => x.enabled).forEach((x, i) => m.set(x.id, i + 1));
    return m;
  }, [ov.mods]);

  const conflictCount = useMemo(() => {
    const m = new Map<string, number>();
    for (const c of ov.conflicts) for (const id of c.mods) m.set(id, (m.get(id) ?? 0) + 1);
    return m;
  }, [ov.conflicts]);

  const filtering = query.trim() !== "" || kind !== "all";
  const shown = ov.mods.filter((m) => {
    if (kind !== "all" && m.kind !== kind) return false;
    const q = query.trim().toLowerCase();
    if (!q) return true;
    return [m.name, m.hero, m.notes, m.originalName].some((s) => s.toLowerCase().includes(q));
  });

  const enabledCount = ov.mods.filter((m) => m.enabled).length;
  const selected = ov.mods.find((m) => m.id === selectedId) ?? null;

  const move = (id: string, toIndex: number) => {
    const ids = ov.mods.map((m) => m.id);
    const from = ids.indexOf(id);
    if (from < 0 || toIndex < 0 || toIndex >= ids.length || from === toIndex) return;
    ids.splice(from, 1);
    ids.splice(toIndex, 0, id);
    void act(() => api.reorder(ids));
  };

  return (
    <div className="library">
      <div className="library-main">
        <div className="view-head">
          <div>
            <h1>Library</h1>
            <p className="sub">
              {ov.mods.length} mod{ov.mods.length === 1 ? "" : "s"} · {enabledCount} on. Top of the list loads first
              (pak01).
            </p>
          </div>
          <div className="head-actions">
            <button
              className="btn ghost"
              disabled={enabledCount === 0}
              onClick={() => act(() => api.setAllEnabled(false), "All mods switched off. Apply to update the game.")}
            >
              All off
            </button>
            <button className="btn primary" onClick={onImport}>
              <Icon name="plus" /> Import VPK
            </button>
          </div>
        </div>

        <div className="filters">
          <label className="search">
            <Icon name="search" size={15} />
            <input
              placeholder="Search name, hero or notes"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              aria-label="Search mods"
            />
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
              Import a VPK you've built, or drag one onto this window. It's stored in the app's library, and nothing in
              the game folder changes until you apply.
            </p>
            <button className="btn primary" onClick={onImport}>
              <Icon name="plus" /> Import VPK
            </button>
          </div>
        ) : (
          <ol className="mod-list" aria-label="Mods in load order">
            {shown.map((m) => {
              const idx = ov.mods.indexOf(m);
              const v = verifyState(m, ov.game.buildId);
              const slot = slots.get(m.id);
              const conflicts = conflictCount.get(m.id) ?? 0;
              return (
                <li
                  key={m.id}
                  className={`mod-row ${m.enabled ? "enabled" : ""} ${selectedId === m.id ? "selected" : ""} ${
                    overId === m.id && dragId !== m.id ? "drop-target" : ""
                  } ${dragId === m.id ? "dragging" : ""}`}
                  draggable={!filtering}
                  onDragStart={(e) => {
                    setDragId(m.id);
                    e.dataTransfer.effectAllowed = "move";
                  }}
                  onDragOver={(e) => {
                    if (!dragId) return;
                    e.preventDefault();
                    setOverId(m.id);
                  }}
                  onDragEnd={() => {
                    setDragId(null);
                    setOverId(null);
                  }}
                  onDrop={(e) => {
                    e.preventDefault();
                    if (dragId) move(dragId, idx);
                    setDragId(null);
                    setOverId(null);
                  }}
                  onClick={() => select(selectedId === m.id ? null : m.id)}
                >
                  <span className={`grip ${filtering ? "hidden" : ""}`} title="Drag to change load order">
                    <Icon name="grip" />
                  </span>
                  <span className="slot" title={slot ? `Loads as pak${String(slot).padStart(2, "0")}_dir.vpk` : "Off"}>
                    {slot ? String(slot).padStart(2, "0") : "—"}
                  </span>
                  <Toggle
                    on={m.enabled}
                    label={`${m.enabled ? "Turn off" : "Turn on"} ${m.name}`}
                    onChange={(on) => act(() => api.setEnabled(m.id, on))}
                  />
                  <div className="mod-main">
                    <div className="mod-name">{m.name}</div>
                    <div className="mod-meta">
                      <span className={`kind kind-${m.kind}`}>{kindLabel[m.kind]}</span>
                      {m.hero && <span>{m.hero}</span>}
                      <span>v{m.version}</span>
                      <span>{formatSize(m.size)}</span>
                    </div>
                  </div>
                  {conflicts > 0 && (
                    <span className="conflict" title={`Shares ${conflicts} file(s) with another enabled mod`}>
                      <Icon name="warn" size={14} /> {conflicts}
                    </span>
                  )}
                  <span className={`verify v-${v}`}>{verifyText[v]}</span>
                  {!filtering && (
                    <span className="order-btns">
                      <button
                        className="icon-btn"
                        aria-label={`Move ${m.name} up`}
                        disabled={idx === 0}
                        onClick={(e) => {
                          e.stopPropagation();
                          move(m.id, idx - 1);
                        }}
                      >
                        <Icon name="up" size={14} />
                      </button>
                      <button
                        className="icon-btn"
                        aria-label={`Move ${m.name} down`}
                        disabled={idx === ov.mods.length - 1}
                        onClick={(e) => {
                          e.stopPropagation();
                          move(m.id, idx + 1);
                        }}
                      >
                        <Icon name="down" size={14} />
                      </button>
                    </span>
                  )}
                </li>
              );
            })}
            {shown.length === 0 && <li className="no-match">No mods match that search.</li>}
          </ol>
        )}
      </div>

      {selected && <ModDrawer key={selected.id} m={selected} ov={ov} act={act} close={() => select(null)} />}
    </div>
  );
}

function ModDrawer({ m, ov, act, close }: { m: ModEntry; ov: Overview; act: Act; close: () => void }) {
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
