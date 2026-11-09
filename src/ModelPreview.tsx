import { useEffect, useRef, useState } from "react";
import type * as THREE from "three";
import type { GLTF } from "three/examples/jsm/loaders/GLTFLoader.js";
import { api } from "./api";
import type { PreviewInfo } from "./types";
import { Icon } from "./ui";
import { PreviewScene, type LoadedModel } from "./preview3d";

const prettyClip = (n: string) => n.replace(/^primary_/, "").replace(/_/g, " ");

/** Full-window 3D view of the model, drawn roughly as the game does. */
export function ModelPreview({ projectId, heroName, onClose }: { projectId: string; heroName: string; onClose: () => void }) {
  const host = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<PreviewScene | null>(null);
  const originalRef = useRef<{ gltf: GLTF; model: LoadedModel | null } | null>(null);
  const [info, setInfo] = useState<PreviewInfo | null>(null);
  const [model, setModel] = useState<LoadedModel | null>(null);
  const [clips, setClips] = useState<THREE.AnimationClip[]>([]);
  const [clip, setClip] = useState("");
  const [status, setStatus] = useState("Loading the model…");
  const [error, setError] = useState<string | null>(null);
  const [playing, setPlaying] = useState(true);
  const [outline, setOutline] = useState(true);
  const [seeThrough, setSeeThrough] = useState(false);
  const [compare, setCompare] = useState(false);
  const [light, setLight] = useState(35);

  useEffect(() => {
    const k = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  }, [onClose]);

  useEffect(() => {
    if (!host.current) return;
    let cancelled = false;
    const scene = new PreviewScene(host.current, (p) => api.modelPreviewFile(projectId, p));
    sceneRef.current = scene;
    (async () => {
      try {
        const i = await api.modelPreviewInfo(projectId);
        if (cancelled) return;
        setInfo(i);
        setStatus(i.sent ? "Loading your model from Blender…" : "Loading the original model…");
        const mine = await scene.loadGltf(i.model);
        if (cancelled) return;
        const m = await scene.addModel(mine, i.materials);
        setModel(m);
        scene.frameAll();
        let anims = mine.animations;
        if (i.sent) {
          setStatus("Loading animations…");
          const orig = await scene.loadGltf(i.original);
          if (cancelled) return;
          originalRef.current = { gltf: orig, model: null };
          anims = orig.animations;
        }
        const sorted = [...anims].sort((a, b) => a.name.localeCompare(b.name));
        setClips(sorted);
        const idle = sorted.find((c) => /stand_idle/.test(c.name)) ?? sorted[0];
        if (idle) {
          setClip(idle.name);
          scene.play(idle);
        }
        setStatus("");
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
      scene.dispose();
      sceneRef.current = null;
    };
  }, [projectId]);

  const s = sceneRef.current;
  const current = clips.find((c) => c.name === clip) ?? null;

  useEffect(() => {
    sceneRef.current?.setOutline(outline);
  }, [outline, model, compare]);
  useEffect(() => {
    sceneRef.current?.setSeeThrough(seeThrough);
  }, [seeThrough, model]);
  useEffect(() => {
    sceneRef.current?.setLightAngle(light);
  }, [light]);
  useEffect(() => {
    if (sceneRef.current) sceneRef.current.playing = playing;
  }, [playing]);

  const toggleCompare = async () => {
    const o = originalRef.current;
    if (!s || !o || !info) return;
    if (o.model) {
      s.removeModel(o.model.root);
      o.model = null;
      setCompare(false);
    } else {
      o.model = await s.addModel(o.gltf, info.materials, 1.4, true);
      setCompare(true);
      s.setOutline(outline);
    }
    s.play(current);
    s.frameAll();
  };

  const pick = (name: string) => {
    setClip(name);
    sceneRef.current?.play(clips.find((c) => c.name === name) ?? null);
  };

  return (
    <div className="preview-overlay" role="dialog" aria-modal="true" aria-label="Model preview">
      <header className="preview-head">
        <div>
          <h2>Preview: {heroName}</h2>
          <span className="hint">
            {info?.sent ? `Your model, sent from Blender ${new Date(info.sent).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}` : "The original model (nothing sent from Blender yet)"}
            {model ? ` · ${model.triangles.toLocaleString()} triangles · ${model.materials.length} materials` : ""}
          </span>
        </div>
        <button className="icon-btn" onClick={onClose} aria-label="Close">
          <Icon name="close" />
        </button>
      </header>
      <div className="preview-main">
        <div className="preview-canvas" ref={host}>
          {(status || error) && (
            <div className="preview-status">
              {error ? (
                <>
                  <strong>Couldn't show the model</strong>
                  <span>{error}</span>
                </>
              ) : (
                <span>{status}</span>
              )}
            </div>
          )}
          {compare && (
            <div className="preview-labels">
              <span>Yours</span>
              <span>Original</span>
            </div>
          )}
        </div>
        <aside className="preview-side">
          <section>
            <h3>Animation</h3>
            <select value={clip} onChange={(e) => pick(e.target.value)} disabled={!clips.length}>
              {clips.map((c) => (
                <option key={c.name} value={c.name}>
                  {prettyClip(c.name)}
                </option>
              ))}
            </select>
            <div className="row">
              <button className="btn" onClick={() => setPlaying(!playing)} disabled={!clips.length}>
                {playing ? "Pause" : "Play"}
              </button>
              <button className="btn" onClick={() => pick("")}>
                Rest pose
              </button>
            </div>
          </section>
          <section>
            <h3>View</h3>
            <label className="check">
              <input type="checkbox" checked={outline} onChange={(e) => setOutline(e.target.checked)} /> Outline
            </label>
            <label className="check">
              <input type="checkbox" checked={seeThrough} onChange={(e) => setSeeThrough(e.target.checked)} /> Show see-through spots
            </label>
            {seeThrough && <p className="hint">Red is where the game shows nothing: the inside of a face, or a face pointing the wrong way. Some red inside sleeves and collars is normal.</p>}
            <label className="check">
              <input type="checkbox" checked={compare} disabled={!info?.sent || !!error} onChange={toggleCompare} /> Original beside it
            </label>
            <label className="slider">
              Light
              <input type="range" min={-180} max={180} value={light} onChange={(e) => setLight(Number(e.target.value))} />
            </label>
            <button className="btn" onClick={() => sceneRef.current?.frameAll()}>
              Reset camera
            </button>
          </section>
          {model && model.unknown.length > 0 && (
            <section>
              <h3>Materials</h3>
              <p className="hint">No game settings found for {model.unknown.join(", ")}: shown plain.</p>
            </section>
          )}
          <p className="hint preview-note">Drag to turn, scroll to zoom, right-drag to move. Close to the game's look, not exact: there's no map lighting, fog or post-processing, so give it a final look in game.</p>
        </aside>
      </div>
    </div>
  );
}
