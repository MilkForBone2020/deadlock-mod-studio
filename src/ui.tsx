import { useEffect, type ReactNode } from "react";

// Small shared pieces: icons, toggle switch, toasts, modal frame.

const paths: Record<string, string> = {
  library: "M4 5h4v14H4zM10 5h4v14h-4zM16.5 5.5l3.5 1-3.5 13-3.5-1z",
  profiles: "M4 7h16M4 12h16M4 17h10",
  activity: "M3 12h4l3-7 4 14 3-7h4",
  settings:
    "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zm8 3l2-1-1-3-2 .5-1.5-1.5.5-2-3-1-1 2h-2l-1-2-3 1 .5 2L6 8.5 4 8l-1 3 2 1v1l-2 1 1 3 2-.5 1.5 1.5-.5 2 3 1 1-2h2l1 2 3-1-.5-2 1.5-1.5 2 .5 1-3-2-1z",
  plus: "M12 5v14M5 12h14",
  grip: "M9 6h.01M15 6h.01M9 12h.01M15 12h.01M9 18h.01M15 18h.01",
  warn: "M12 3l10 18H2zM12 10v5M12 18h.01",
  check: "M5 12l5 5L20 7",
  close: "M6 6l12 12M18 6L6 18",
  folder: "M3 6h6l2 2h10v11H3z",
  play: "M7 5l12 7-12 7z",
  shield: "M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z",
  up: "M12 19V5M6 11l6-6 6 6",
  down: "M12 5v14M6 13l6 6 6-6",
  search: "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4",
  swap: "M7 7h11l-3-3M17 17H6l3 3",
  trash: "M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13",
  layers: "M12 3l9 5-9 5-9-5zM3 13l9 5 9-5",
  wave: "M3 12h2M7 8v8M11 5v14M15 9v6M19 11v2M21 12h0",
  cube: "M12 3l8 4.5v9L12 21l-8-4.5v-9zM12 12l8-4.5M12 12v9M12 12L4 7.5",
};

export function Icon({ name, size = 16 }: { name: keyof typeof paths | string; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={name === "grip" ? 3 : 1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={paths[name] ?? ""} />
    </svg>
  );
}

export function Toggle({
  on,
  onChange,
  label,
  disabled,
}: {
  on: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled}
      className={`toggle ${on ? "on" : ""}`}
      onClick={(e) => {
        e.stopPropagation();
        onChange(!on);
      }}
    >
      <span className="knob" />
    </button>
  );
}

export interface Toast {
  id: number;
  kind: "ok" | "error" | "info";
  text: string;
}

export function Toasts({ items, dismiss }: { items: Toast[]; dismiss: (id: number) => void }) {
  return (
    <div className="toasts" aria-live="polite">
      {items.map((t) => (
        <ToastItem key={t.id} t={t} dismiss={dismiss} />
      ))}
    </div>
  );
}

function ToastItem({ t, dismiss }: { t: Toast; dismiss: (id: number) => void }) {
  useEffect(() => {
    const ms = t.kind === "error" ? 9000 : 4000;
    const h = setTimeout(() => dismiss(t.id), ms);
    return () => clearTimeout(h);
  }, [t, dismiss]);
  return (
    <div className={`toast ${t.kind}`} role={t.kind === "error" ? "alert" : "status"}>
      <span className="toast-text">{t.text}</span>
      <button className="icon-btn" onClick={() => dismiss(t.id)} aria-label="Dismiss">
        <Icon name="close" size={14} />
      </button>
    </div>
  );
}

export function Modal({
  title,
  children,
  onClose,
  footer,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  footer?: ReactNode;
}) {
  useEffect(() => {
    const k = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  }, [onClose]);
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" role="dialog" aria-modal="true" aria-label={title} onClick={(e) => e.stopPropagation()}>
        <header className="modal-head">
          <h2>{title}</h2>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            <Icon name="close" />
          </button>
        </header>
        <div className="modal-body">{children}</div>
        {footer && <footer className="modal-foot">{footer}</footer>}
      </div>
    </div>
  );
}

export const kindLabel: Record<string, string> = {
  skin: "Skin",
  sound: "Sound",
  ui: "UI",
  other: "Other",
};

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function formatDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}
