import { useCallback, useEffect, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";
import { errorText, isTauri } from "./api";

// Self-update: checks the latest GitHub release on start and every 6 hours.
// Updates are signed; the app rejects anything not signed with this repo's key.

export type UpdateStatus = "idle" | "checking" | "none" | "available" | "downloading" | "error";

export interface UpdaterState {
  status: UpdateStatus;
  current: string | null;
  latest: string | null;
  notes: string | null;
  progress: number | null; // 0..1 while downloading, if the size is known
  error: string | null;
  lastChecked: Date | null;
}

const SIX_HOURS = 6 * 60 * 60 * 1000;

export function useUpdater() {
  const [s, setS] = useState<UpdaterState>({
    status: "idle",
    current: null,
    latest: null,
    notes: null,
    progress: null,
    error: null,
    lastChecked: null,
  });
  const update = useRef<Update | null>(null);

  const runCheck = useCallback(async (manual = false) => {
    if (!isTauri) return;
    setS((x) => ({ ...x, status: x.status === "available" ? "available" : "checking", error: null }));
    try {
      const u = await check();
      update.current = u;
      setS((x) => ({
        ...x,
        status: u ? "available" : "none",
        latest: u?.version ?? null,
        notes: u?.body ?? null,
        lastChecked: new Date(),
      }));
    } catch (e) {
      // Silent on automatic checks (e.g. offline); shown when asked for.
      setS((x) => ({ ...x, status: manual ? "error" : "idle", error: errorText(e), lastChecked: new Date() }));
    }
  }, []);

  const install = useCallback(async () => {
    const u = update.current;
    if (!u) return;
    let total = 0;
    let got = 0;
    setS((x) => ({ ...x, status: "downloading", progress: null, error: null }));
    try {
      await u.downloadAndInstall((ev) => {
        if (ev.event === "Started") total = ev.data.contentLength ?? 0;
        else if (ev.event === "Progress") {
          got += ev.data.chunkLength;
          if (total > 0) setS((x) => ({ ...x, progress: Math.min(1, got / total) }));
        }
      });
      await relaunch();
    } catch (e) {
      setS((x) => ({ ...x, status: "error", error: errorText(e) }));
    }
  }, []);

  useEffect(() => {
    if (!isTauri) {
      setS((x) => ({ ...x, current: "dev" }));
      return;
    }
    getVersion()
      .then((v) => setS((x) => ({ ...x, current: v })))
      .catch(() => undefined);
    void runCheck();
    const h = setInterval(() => void runCheck(), SIX_HOURS);
    return () => clearInterval(h);
  }, [runCheck]);

  return { ...s, check: () => runCheck(true), install };
}

export type Updater = ReturnType<typeof useUpdater>;
