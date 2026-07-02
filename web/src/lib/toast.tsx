"use client";

import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  CheckCircleIcon,
  ExclamationTriangleIcon,
  InformationCircleIcon,
  XCircleIcon,
  XMarkIcon,
} from "@heroicons/react/24/outline";

export type ToastType = "success" | "error" | "info" | "warning";

interface Toast {
  id: number;
  type: ToastType;
  message: string;
}

export interface ToastApi {
  /** Show a toast. `durationMs <= 0` keeps it until dismissed. */
  show: (message: string, type?: ToastType, durationMs?: number) => void;
  success: (message: string) => void;
  error: (message: string) => void;
  info: (message: string) => void;
  warning: (message: string) => void;
}

const ToastContext = createContext<ToastApi | null>(null);

/**
 * Access the shared toast API. Safe to call outside a ToastProvider — it
 * degrades to console logging rather than throwing, so a stray call can't
 * crash a page.
 */
export function useToast(): ToastApi {
  const ctx = useContext(ToastContext);
  return ctx ?? FALLBACK_TOAST;
}

const FALLBACK_TOAST: ToastApi = {
  show: (m, type = "info") => console[type === "error" ? "error" : "log"](`[toast] ${m}`),
  success: (m) => console.log(`[toast] ${m}`),
  error: (m) => console.error(`[toast] ${m}`),
  info: (m) => console.log(`[toast] ${m}`),
  warning: (m) => console.warn(`[toast] ${m}`),
};

const DEFAULT_DURATIONS: Record<ToastType, number> = {
  success: 4000,
  info: 4000,
  warning: 6000,
  error: 8000,
};

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const idRef = useRef(0);

  const remove = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const show = useCallback(
    (message: string, type: ToastType = "info", durationMs?: number) => {
      const id = ++idRef.current;
      setToasts((prev) => [...prev, { id, type, message }]);
      const ms = durationMs ?? DEFAULT_DURATIONS[type];
      if (ms > 0) {
        setTimeout(() => remove(id), ms);
      }
    },
    [remove],
  );

  const api = useMemo<ToastApi>(
    () => ({
      show,
      success: (m) => show(m, "success"),
      error: (m) => show(m, "error"),
      info: (m) => show(m, "info"),
      warning: (m) => show(m, "warning"),
    }),
    [show],
  );

  return (
    <ToastContext.Provider value={api}>
      {children}
      <Toaster toasts={toasts} onDismiss={remove} />
    </ToastContext.Provider>
  );
}

const TYPE_STYLES: Record<ToastType, { bg: string; text: string; Icon: typeof CheckCircleIcon }> = {
  success: { bg: "bg-success-bg", text: "text-success-text", Icon: CheckCircleIcon },
  error: { bg: "bg-error-bg", text: "text-error-text", Icon: XCircleIcon },
  warning: { bg: "bg-warning-bg", text: "text-warning-text", Icon: ExclamationTriangleIcon },
  info: { bg: "bg-info-bg", text: "text-info-text", Icon: InformationCircleIcon },
};

function Toaster({ toasts, onDismiss }: { toasts: Toast[]; onDismiss: (id: number) => void }) {
  if (toasts.length === 0) return null;
  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-0 z-[100] flex flex-col items-center gap-2 p-4 pb-safe sm:items-end sm:right-4 sm:left-auto">
      {toasts.map((t) => {
        const { bg, text, Icon } = TYPE_STYLES[t.type];
        return (
          <div
            key={t.id}
            role="status"
            className={`pointer-events-auto flex w-full max-w-sm items-start gap-2 rounded-lg border border-border ${bg} px-3 py-2 shadow-lg`}
          >
            <Icon className={`h-5 w-5 shrink-0 ${text}`} />
            <p className={`flex-1 text-sm ${text}`}>{t.message}</p>
            <button
              onClick={() => onDismiss(t.id)}
              className={`shrink-0 rounded p-0.5 ${text} opacity-70 hover:opacity-100 transition`}
              aria-label="Dismiss"
            >
              <XMarkIcon className="h-4 w-4" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
