// The audio editor: trim, cut, volume line and fades on my file for one
// game sound. Edits are a recipe saved on the project item; the file itself
// never changes. Playback renders through the same code as the build, so
// what you hear here is what plays in game.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, isTauri } from "./api";
import type { AudioEdit, Level, SoundJob, Waveform } from "./types";
import { Icon, Modal } from "./ui";

const EMPTY: AudioEdit = { start: 0, end: null, cuts: [], volume: [], fadeIn: 0, fadeOut: 0 };
const SILENT_DB = -60;
const TOP_DB = 12;
const LOW_DB = -36;
const H = 200;

type Act = (fn: () => Promise<unknown>, ok?: string) => Promise<boolean>;

/** Parts of the file kept: [start, end] minus the cuts (as the build does). */
export function keptParts(e: AudioEdit, duration: number): [number, number][] {
  const a = Math.min(Math.max(e.start, 0), duration);
  const b = Math.min(Math.max(e.end ?? duration, a), duration);
  const cuts = e.cuts.map(([x, y]) => [Math.min(x, y), Math.max(x, y)] as [number, number]).filter((c) => c[1] > c[0]).sort((p, q) => p[0] - q[0]);
  const out: [number, number][] = [];
  let at = a;
  for (const c of cuts) {
    if (c[1] <= at || c[0] >= b) continue;
    if (c[0] > at) out.push([at, Math.min(c[0], b)]);
    at = Math.max(at, c[1]);
  }
  if (at < b) out.push([at, b]);
  return out.filter((r) => r[1] - r[0] > 1e-4);
}

function gainDbAt(e: AudioEdit, t: number): number {
  const p = [...e.volume].sort((x, y) => x[0] - y[0]);
  if (!p.length) return 0;
  if (t <= p[0][0]) return p[0][1];
  if (t >= p[p.length - 1][0]) return p[p.length - 1][1];
  let i = 0;
  while (i + 1 < p.length && p[i + 1][0] <= t) i++;
  const [a, b] = [p[i], p[i + 1]];
  const f = b[0] > a[0] ? (t - a[0]) / (b[0] - a[0]) : 0;
  return a[1] + (b[1] - a[1]) * f;
}

const dbToY = (db: number) => (db <= SILENT_DB ? H - 2 : Math.max(2, Math.min(H - 12, ((TOP_DB - Math.max(db, LOW_DB)) / (TOP_DB - LOW_DB)) * (H - 14) + 2)));
const yToDb = (y: number) => (y >= H - 8 ? SILENT_DB : Math.round((TOP_DB - ((y - 2) / (H - 14)) * (TOP_DB - LOW_DB)) * 2) / 2);
const fmt = (s: number) => {
  const m = Math.floor(s / 60);
  const r = s - m * 60;
  return m ? `${m}:${r.toFixed(2).padStart(5, "0")}` : `${r.toFixed(2)} s`;
};

export function AudioEditor({ projectId, item, original, onClose, act }: { projectId: string; item: SoundJob; original: Level | null; onClose: () => void; act: Act }) {
  const [wave, setWave] = useState<Waveform | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [edit, setEditRaw] = useState<AudioEdit>(item.edit ?? EMPTY);
  const [undo, setUndo] = useState<AudioEdit[]>([]);
  const [redo, setRedo] = useState<AudioEdit[]>([]);
  const [sel, setSel] = useState<[number, number] | null>(null);
  const [cursor, setCursor] = useState(0);
  const [zoom, setZoom] = useState(1);
  const [tool, setTool] = useState<"select" | "volume">("select");
  const [playing, setPlaying] = useState<"edited" | "original" | null>(null);
  const [rendering, setRendering] = useState(false);
  const [playhead, setPlayhead] = useState<number | null>(null);
  const [width, setWidth] = useState(800);
  const canvas = useRef<HTMLCanvasElement>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const audio = useRef<HTMLAudioElement | null>(null);
  const cache = useRef<{ key: string; url: string } | null>(null);
  const drag = useRef<null | { kind: "sel" | "start" | "end" | "point"; from: number; index?: number; moved: boolean }>(null);

  const dur = wave?.duration ?? 0;
  const kept = useMemo(() => keptParts(edit, dur), [edit, dur]);
  const outLen = kept.reduce((s, r) => s + r[1] - r[0], 0);

  useEffect(() => {
    api.audioPeaks(item.source, 6000).then(setWave).catch((e) => setError(String(e)));
  }, [item.source]);

  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const ob = new ResizeObserver(() => setWidth(el.clientWidth || 800));
    ob.observe(el);
    return () => ob.disconnect();
  }, []);

  const W = Math.round(width * zoom);
  const tToX = useCallback((t: number) => (dur ? (t / dur) * W : 0), [dur, W]);
  const xToT = useCallback((x: number) => (dur ? Math.min(dur, Math.max(0, (x / W) * dur)) : 0), [dur, W]);

  // Source time <-> time in the result.
  const toOut = (t: number) => {
    let acc = 0;
    for (const [a, b] of kept) {
      if (t < a) return acc;
      if (t <= b) return acc + t - a;
      acc += b - a;
    }
    return acc;
  };
  const toSrc = (o: number) => {
    let acc = 0;
    for (const [a, b] of kept) {
      if (o <= acc + (b - a)) return a + (o - acc);
      acc += b - a;
    }
    return kept.length ? kept[kept.length - 1][1] : 0;
  };

  const setEdit = (next: AudioEdit) => {
    setUndo((u) => [...u.slice(-99), edit]);
    setRedo([]);
    setEditRaw(next);
  };

  // ---- drawing
  useEffect(() => {
    const c = canvas.current;
    if (!c || !wave) return;
    const dpr = window.devicePixelRatio || 1;
    c.width = W * dpr;
    c.height = H * dpr;
    c.style.width = `${W}px`;
    c.style.height = `${H}px`;
    const g = c.getContext("2d")!;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.fillStyle = "#0f120f";
    g.fillRect(0, 0, W, H);
    // Removed parts.
    const removed: [number, number][] = [];
    let at = 0;
    for (const [a, b] of kept) {
      if (a > at) removed.push([at, a]);
      at = b;
    }
    if (at < dur) removed.push([at, dur]);
    g.fillStyle = "rgba(221,115,94,0.10)";
    for (const [a, b] of removed) g.fillRect(tToX(a), 0, tToX(b) - tToX(a), H);
    // Waveform: faint as imported, bright as it will sound.
    const mid = (H - 12) / 2;
    const n = wave.peaks.length;
    const keptAt = (t: number) => kept.some(([a, b]) => t >= a && t <= b);
    for (let x = 0; x < W; x++) {
      const i0 = Math.floor((x / W) * n);
      const i1 = Math.max(i0 + 1, Math.floor(((x + 1) / W) * n));
      let lo = 0;
      let hi = 0;
      for (let i = i0; i < Math.min(i1, n); i++) {
        lo = Math.min(lo, wave.peaks[i][0]);
        hi = Math.max(hi, wave.peaks[i][1]);
      }
      const t = xToT(x + 0.5);
      g.fillStyle = "rgba(156,152,132,0.25)";
      g.fillRect(x, mid - hi * mid, 1, Math.max(1, (hi - lo) * mid));
      if (keptAt(t)) {
        const db = gainDbAt(edit, t);
        const k = db <= SILENT_DB ? 0 : Math.pow(10, db / 20);
        g.fillStyle = "#74b8a6";
        g.fillRect(x, mid - Math.min(1, hi * k) * mid, 1, Math.max(1, Math.min(2, (hi - lo) * k) * mid));
      }
    }
    // Fades (in the result's time, shown where they land).
    const fadeIn = Math.min(edit.fadeIn, outLen);
    const fadeOut = Math.min(edit.fadeOut, outLen);
    g.strokeStyle = "rgba(227,172,82,0.8)";
    g.lineWidth = 1.5;
    if (fadeIn > 0 && kept.length) {
      g.beginPath();
      g.moveTo(tToX(toSrc(0)), H - 12);
      g.lineTo(tToX(toSrc(fadeIn)), 2);
      g.stroke();
    }
    if (fadeOut > 0 && kept.length) {
      g.beginPath();
      g.moveTo(tToX(toSrc(outLen - fadeOut)), 2);
      g.lineTo(tToX(toSrc(outLen)), H - 12);
      g.stroke();
    }
    // 0 dB guide and the volume line.
    g.strokeStyle = "rgba(214,182,112,0.18)";
    g.setLineDash([4, 4]);
    g.beginPath();
    g.moveTo(0, dbToY(0));
    g.lineTo(W, dbToY(0));
    g.stroke();
    g.setLineDash([]);
    const pts = [...edit.volume].sort((p, q) => p[0] - q[0]);
    g.strokeStyle = "#d6b670";
    g.lineWidth = 2;
    g.beginPath();
    g.moveTo(0, dbToY(gainDbAt(edit, 0)));
    for (const p of pts) g.lineTo(tToX(p[0]), dbToY(p[1]));
    g.lineTo(W, dbToY(gainDbAt(edit, dur)));
    g.stroke();
    g.fillStyle = "#e8cd8f";
    for (const p of pts) {
      g.beginPath();
      g.arc(tToX(p[0]), dbToY(p[1]), 4.5, 0, Math.PI * 2);
      g.fill();
    }
    // Trim handles.
    g.fillStyle = "#e3ac52";
    for (const t of [edit.start, edit.end ?? dur]) g.fillRect(tToX(t) - 1.5, 0, 3, H);
    // Selection, cursor, playhead.
    if (sel) {
      g.fillStyle = "rgba(116,184,166,0.18)";
      g.fillRect(tToX(sel[0]), 0, tToX(sel[1]) - tToX(sel[0]), H);
    }
    g.fillStyle = "#ece3cc";
    g.fillRect(tToX(cursor), 0, 1, H);
    if (playhead != null) {
      g.fillStyle = "#93c27d";
      g.fillRect(tToX(playhead) - 1, 0, 2, H);
    }
    // Time ruler.
    g.fillStyle = "#6d6b5d";
    g.font = "10px sans-serif";
    const step = [0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30, 60].find((s) => (s / Math.max(dur, 0.001)) * W > 70) ?? 60;
    for (let t = 0; t <= dur; t += step) {
      g.fillRect(tToX(t), H - 10, 1, 10);
      g.fillText(t >= 60 ? `${Math.floor(t / 60)}:${String(Math.round(t % 60)).padStart(2, "0")}` : `${+t.toFixed(2)}s`, tToX(t) + 3, H - 2);
    }
  }, [wave, edit, sel, cursor, playhead, W, kept, outLen, dur, tToX, xToT]);

  // ---- mouse
  const pos = (e: React.MouseEvent) => {
    const r = canvas.current!.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };
  const down = (e: React.MouseEvent) => {
    if (!wave) return;
    const { x, y } = pos(e);
    const t = xToT(x);
    if (tool === "volume") {
      const i = edit.volume.findIndex((p) => Math.hypot(tToX(p[0]) - x, dbToY(p[1]) - y) < 8);
      if (i >= 0) {
        drag.current = { kind: "point", from: t, index: i, moved: false };
      } else {
        const next = { ...edit, volume: [...edit.volume, [t, yToDb(y)] as [number, number]] };
        setEdit(next);
        drag.current = { kind: "point", from: t, index: next.volume.length - 1, moved: true };
      }
      return;
    }
    if (Math.abs(tToX(edit.start) - x) < 6) drag.current = { kind: "start", from: t, moved: false };
    else if (Math.abs(tToX(edit.end ?? dur) - x) < 6) drag.current = { kind: "end", from: t, moved: false };
    else drag.current = { kind: "sel", from: t, moved: false };
  };
  const move = (e: React.MouseEvent) => {
    const d = drag.current;
    if (!d || !wave) return;
    const { x, y } = pos(e);
    const t = xToT(x);
    if (!d.moved && Math.abs(t - d.from) < dur / W / 2 && d.kind !== "point") return;
    if (!d.moved && d.kind !== "point") setUndo((u) => [...u.slice(-99), edit]);
    d.moved = true;
    if (d.kind === "sel") setSel([Math.min(d.from, t), Math.max(d.from, t)]);
    if (d.kind === "start") setEditRaw({ ...edit, start: Math.min(t, (edit.end ?? dur) - 0.01) });
    if (d.kind === "end") setEditRaw({ ...edit, end: Math.max(t, edit.start + 0.01) >= dur - 1e-6 ? null : Math.max(t, edit.start + 0.01) });
    if (d.kind === "point" && d.index != null) {
      const v = [...edit.volume];
      v[d.index] = [t, yToDb(y)];
      setEditRaw({ ...edit, volume: v });
    }
  };
  const up = (e: React.MouseEvent) => {
    const d = drag.current;
    drag.current = null;
    if (!d || !wave) return;
    if (!d.moved && d.kind === "sel") {
      setCursor(xToT(pos(e).x));
      setSel(null);
    }
  };
  const dbl = (e: React.MouseEvent) => {
    if (tool !== "volume") return;
    const { x, y } = pos(e);
    const i = edit.volume.findIndex((p) => Math.hypot(tToX(p[0]) - x, dbToY(p[1]) - y) < 8);
    if (i >= 0) setEdit({ ...edit, volume: edit.volume.filter((_, k) => k !== i) });
  };

  // ---- edits on the selection
  const cutSel = () => sel && (setEdit({ ...edit, cuts: [...edit.cuts, sel] }), setSel(null), setCursor(sel[0]));
  const keepSel = () => sel && (setEdit({ ...edit, start: sel[0], end: sel[1] >= dur - 1e-6 ? null : sel[1], cuts: edit.cuts.filter(([a, b]) => b > sel[0] && a < sel[1]) }), setSel(null), setCursor(sel[0]));
  /** Set the volume line inside the selection (keeping it as it was outside). */
  const shapeSel = (f: (db: number) => number) => {
    if (!sel) return;
    const [a, b] = sel;
    const edge = 0.01;
    const outside = edit.volume.filter((p) => p[0] < a - edge || p[0] > b + edge);
    const inside = edit.volume.filter((p) => p[0] >= a && p[0] <= b).map((p) => [p[0], Math.max(SILENT_DB, Math.min(TOP_DB, f(p[1])))] as [number, number]);
    const before = gainDbAt(edit, a - edge);
    const after = gainDbAt(edit, b + edge);
    const startDb = Math.max(SILENT_DB, Math.min(TOP_DB, f(gainDbAt(edit, a))));
    const endDb = Math.max(SILENT_DB, Math.min(TOP_DB, f(gainDbAt(edit, b))));
    setEdit({ ...edit, volume: [...outside, [Math.max(0, a - edge), before], [a, startDb], ...inside, [b, endDb], [Math.min(dur, b + edge), after]] });
  };
  const doUndo = () => {
    const last = undo[undo.length - 1];
    if (!last) return;
    setRedo((r) => [...r, edit]);
    setUndo((u) => u.slice(0, -1));
    setEditRaw(last);
  };
  const doRedo = () => {
    const last = redo[redo.length - 1];
    if (!last) return;
    setUndo((u) => [...u, edit]);
    setRedo((r) => r.slice(0, -1));
    setEditRaw(last);
  };

  // ---- playback
  const stop = () => {
    audio.current?.pause();
    setPlaying(null);
    setPlayhead(null);
  };
  useEffect(() => () => {
    audio.current?.pause();
    if (cache.current) URL.revokeObjectURL(cache.current.url);
  }, []);
  useEffect(() => {
    if (playing !== "edited") return;
    let raf = 0;
    const tick = () => {
      const a = audio.current;
      if (a) setPlayhead(toSrc(a.currentTime));
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing, kept]);

  const playEdited = async () => {
    if (playing) return stop();
    if (!isTauri) return;
    const key = JSON.stringify([edit, item.volumeDb]);
    try {
      if (cache.current?.key !== key) {
        setRendering(true);
        const buf = await api.renderSoundPreview(item.target, item.source, item.volumeDb ?? 0, edit);
        if (cache.current) URL.revokeObjectURL(cache.current.url);
        cache.current = { key, url: URL.createObjectURL(new Blob([buf], { type: "audio/wav" })) };
      }
    } catch (e) {
      setError(String(e));
      return;
    } finally {
      setRendering(false);
    }
    const a = audio.current ?? new Audio();
    audio.current = a;
    a.src = cache.current!.url;
    a.onended = stop;
    a.currentTime = Math.min(toOut(cursor), Math.max(0, outLen - 0.05));
    setPlaying("edited");
    await a.play();
  };
  const playOriginal = async () => {
    if (playing) return stop();
    if (!isTauri) return;
    const buf = new Uint8Array(await api.audioBytes("game", item.target));
    const a = audio.current ?? new Audio();
    audio.current = a;
    a.src = URL.createObjectURL(new Blob([buf]));
    a.onended = stop;
    setPlaying("original");
    await a.play();
  };

  // Keyboard: space plays, Delete cuts, Ctrl+Z / Ctrl+Y.
  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).tagName === "INPUT") return;
      if (e.code === "Space") {
        e.preventDefault();
        void playEdited();
      } else if ((e.key === "Delete" || e.key === "Backspace") && sel) cutSel();
      else if (e.ctrlKey && e.key.toLowerCase() === "z") doUndo();
      else if (e.ctrlKey && e.key.toLowerCase() === "y") doRedo();
    };
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  });

  const save = async () => {
    const ok = await act(() => api.setItemEdit(projectId, item.target, edit), "Edits saved. Build the mod to hear them in game");
    if (ok) onClose();
  };
  const longer = original && outLen > original.durationSecs * 1.25;

  return (
    <Modal
      title={`Edit: ${item.source.split(/[\\/]/).pop()}`}
      onClose={onClose}
      wide
      footer={
        <>
          <span className="hint">
            Result {fmt(outLen)}
            {original ? ` · original ${fmt(original.durationSecs)}` : ""}
            {longer ? " · much longer than the original" : ""}
          </span>
          <button className="btn ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" onClick={() => void save()}>
            Save edits
          </button>
        </>
      }
    >
      {error && <p className="err">{error}</p>}
      <div className="ae-bar">
        <button className={`btn small ${playing === "edited" ? "on" : ""}`} onClick={() => void playEdited()} disabled={!wave || rendering}>
          <Icon name="play" size={13} /> {rendering ? "Preparing…" : playing === "edited" ? "Stop" : "Play edited"}
        </button>
        <button className={`btn small ghost ${playing === "original" ? "on" : ""}`} onClick={() => void playOriginal()}>
          {playing === "original" ? "Stop" : "Play game's original"}
        </button>
        <span className="ae-sep" />
        <div className="seg">
          <button className={tool === "select" ? "on" : ""} onClick={() => setTool("select")} title="Drag to select; drag the amber lines to trim">
            Select
          </button>
          <button className={tool === "volume" ? "on" : ""} onClick={() => setTool("volume")} title="Click to add a volume point, drag to move, double-click to remove">
            Volume line
          </button>
        </div>
        <span className="ae-sep" />
        <button className="btn small ghost" onClick={doUndo} disabled={!undo.length} title="Undo (Ctrl+Z)">
          Undo
        </button>
        <button className="btn small ghost" onClick={doRedo} disabled={!redo.length} title="Redo (Ctrl+Y)">
          Redo
        </button>
        <button className="btn small ghost" onClick={() => setEdit(EMPTY)} disabled={JSON.stringify(edit) === JSON.stringify(EMPTY)}>
          Reset all
        </button>
        <label className="ae-zoom">
          Zoom
          <input type="range" min={1} max={40} step={0.5} value={zoom} onChange={(e) => setZoom(Number(e.target.value))} />
        </label>
      </div>
      <div className="ae-wave" ref={scroller}>
        {wave ? (
          <canvas ref={canvas} className={tool === "volume" ? "vol" : ""} onMouseDown={down} onMouseMove={move} onMouseUp={up} onMouseLeave={up} onDoubleClick={dbl} />
        ) : (
          <div className="ae-loading">{error ? "Couldn't read the file" : "Reading your file…"}</div>
        )}
      </div>
      <div className="ae-bar">
        <span className="hint">{sel ? `Selection ${fmt(sel[0])} – ${fmt(sel[1])} (${fmt(sel[1] - sel[0])})` : `Cursor ${fmt(cursor)} · drag across the wave to select`}</span>
        <span className="ae-sep" />
        <button className="btn small" disabled={!sel} onClick={cutSel} title="Delete">
          Cut
        </button>
        <button className="btn small" disabled={!sel} onClick={keepSel}>
          Keep only this
        </button>
        <button className="btn small" disabled={!sel} onClick={() => shapeSel((db) => db + 3)}>
          Louder +3 dB
        </button>
        <button className="btn small" disabled={!sel} onClick={() => shapeSel((db) => db - 3)}>
          Quieter −3 dB
        </button>
        <button className="btn small" disabled={!sel} onClick={() => shapeSel(() => SILENT_DB)}>
          Silence
        </button>
      </div>
      <div className="ae-bar">
        <label className="ae-num">
          Fade in
          <input type="number" min={0} step={0.1} value={edit.fadeIn} onChange={(e) => setEdit({ ...edit, fadeIn: Math.max(0, Number(e.target.value) || 0) })} /> s
        </label>
        <label className="ae-num">
          Fade out
          <input type="number" min={0} step={0.1} value={edit.fadeOut} onChange={(e) => setEdit({ ...edit, fadeOut: Math.max(0, Number(e.target.value) || 0) })} /> s
        </label>
        {original && outLen > original.durationSecs + 0.05 && (
          <button className="btn small ghost" onClick={() => setEdit({ ...edit, end: toSrc(original.durationSecs) >= dur - 1e-6 ? null : toSrc(original.durationSecs), fadeOut: Math.max(edit.fadeOut, 1) })} title="Trim the end so the result is as long as the original, with a 1 s fade out">
            Fit to original length
          </button>
        )}
        <span className="hint" style={{ marginLeft: "auto" }}>
          Space play · Delete cut · Ctrl+Z undo. Loudness still matches the original; the volume line shapes it.
        </span>
      </div>
    </Modal>
  );
}
