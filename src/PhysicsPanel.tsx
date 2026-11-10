import { useEffect, useState } from "react";
import { api } from "./api";
import type { PhysicsChain, PhysicsTuning } from "./types";

type Act = (fn: () => Promise<unknown>, ok?: string) => Promise<boolean>;

const DEFAULT: PhysicsTuning = { preset: "", stiffness: 1, damping: 1, gravity: 1, wander: 1 };

const PRESETS: Record<string, string> = {
  wobbly: "Wobbly (holds its shape)",
  dangle: "Dangling (calm)",
  stiff: "Stiff",
  floppy: "Floppy",
  skirt: "Skirt / cape",
};

const SLIDERS: { key: keyof PhysicsTuning; label: string; min: number; max: number; hint: string }[] = [
  { key: "stiffness", label: "Stiffness", min: 0.25, max: 2, hint: "How strongly it springs back to its animated shape" },
  { key: "damping", label: "Damping", min: 0.25, max: 4, hint: "How quickly the swinging settles" },
  { key: "gravity", label: "Weight", min: 0, max: 2, hint: "How much it droops" },
  { key: "wander", label: "Reach", min: 0.5, max: 2, hint: "How far it may move from its animated place" },
];

function Group({ projectId, chain, preset, bones, strands, saved, act }: { projectId: string; chain: string; preset: string; bones: number; strands: number; saved: PhysicsTuning | undefined; act: Act }) {
  const key = `${chain}/${preset}`;
  const [t, setT] = useState<PhysicsTuning>(saved ?? DEFAULT);
  const [params, setParams] = useState<number[] | null>(null);
  useEffect(() => setT(saved ?? DEFAULT), [saved]);
  const effective = t.preset || preset;
  useEffect(() => {
    api.physicsParams(effective, 0, t).then(setParams).catch(() => setParams(null));
  }, [effective, t]);
  const save = (next: PhysicsTuning) => act(() => api.setModelPhysics(projectId, key, next));
  const changed = JSON.stringify(t) !== JSON.stringify(DEFAULT);
  return (
    <div className="phys-group">
      <div className="phys-head">
        <strong>{preset === "wobbly" ? "Wobbly parts" : preset === "dangle" ? "Dangling parts" : preset === "skirt" ? "Skirt / cape" : preset}</strong>
        <span className="hint">
          {bones} bone(s) on {chain.replace(/^dms_/, "")}
          {strands > 1 ? ` (${strands} strands)` : ""}
        </span>
        <select
          value={t.preset}
          onChange={(e) => {
            const next = { ...t, preset: e.target.value };
            setT(next);
            void save(next);
          }}
          aria-label="Preset"
        >
          <option value="">Suggested: {PRESETS[preset] ?? preset}</option>
          {Object.entries(PRESETS)
            .filter(([k]) => k !== preset)
            .map(([k, v]) => (
              <option key={k} value={k}>
                {v}
              </option>
            ))}
        </select>
      </div>
      <div className="phys-sliders">
        {SLIDERS.map((s) => (
          <label key={s.key} title={s.hint}>
            <span>
              {s.label} <em>×{(t[s.key] as number).toFixed(2)}</em>
            </span>
            <input
              type="range"
              min={s.min}
              max={s.max}
              step={0.05}
              value={t[s.key] as number}
              onChange={(e) => setT({ ...t, [s.key]: Number(e.target.value) })}
              onPointerUp={() => void save(t)}
              onKeyUp={() => void save(t)}
            />
          </label>
        ))}
      </div>
      <div className="phys-foot">
        {params && (
          <span className="hint mono">
            First bone: stiffness {params[0].toFixed(2)} · damping {params[1].toFixed(3)} · gravity {params[2].toFixed(2)} · reach {params[3].toFixed(1)}
          </span>
        )}
        {changed && (
          <button
            className="link"
            onClick={() => {
              setT(DEFAULT);
              void act(() => api.setModelPhysics(projectId, key, null));
            }}
          >
            Reset
          </button>
        )}
      </div>
    </div>
  );
}

/** Chains made together (a skirt's strands) are tuned as one. */
function groupChains(chains: PhysicsChain[]): { name: string; members: PhysicsChain[] }[] {
  const out: { name: string; members: PhysicsChain[] }[] = [];
  for (const c of chains) {
    const name = c.group || c.name;
    const g = out.find((x) => x.name === name);
    if (g) g.members.push(c);
    else out.push({ name, members: [c] });
  }
  return out;
}

/** Tuning for the physics bones sent from Blender, by chain and kind. */
export function PhysicsPanel({ projectId, chains, tuning, act }: { projectId: string; chains: PhysicsChain[]; tuning: Record<string, PhysicsTuning>; act: Act }) {
  return (
    <section className="phys-panel">
      <div>
        <h3>Physics</h3>
        <p className="hint">
          Valve's cloth physics, on the bones Blender added. Changes apply on the next build; check the feel in game. The first bone of each chain is pinned to the
          bone it hangs from.
        </p>
      </div>
      {groupChains(chains).map(({ name, members }) => {
        const joints = members.flatMap((c) => c.joints.filter((j) => j.simulate));
        const kinds = [...new Set(joints.map((j) => j.preset))];
        return kinds.map((g) => (
          <Group
            key={`${name}/${g}`}
            projectId={projectId}
            chain={name}
            preset={g}
            bones={joints.filter((j) => j.preset === g).length}
            strands={members.length}
            saved={tuning[`${name}/${g}`]}
            act={act}
          />
        ));
      })}
    </section>
  );
}
