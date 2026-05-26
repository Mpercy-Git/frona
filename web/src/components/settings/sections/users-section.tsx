"use client";

import { useCallback, useEffect, useState } from "react";
import { UsersIcon, UserCircleIcon, TrashIcon, ExclamationTriangleIcon } from "@heroicons/react/24/outline";
import { api } from "@/lib/api-client";
import { SectionHeader } from "@/components/settings/field";

interface AdminUser {
  id: string;
  handle: string;
  email: string;
  name: string;
  groups: string[];
  deactivated_at: string | null;
  created_at: string;
}

interface PendingConfirm {
  title: string;
  body: string;
  confirmLabel: string;
  danger?: boolean;
  context: ActionContext;
  run: () => Promise<void>;
}

const ADMINS = "admins";

type ActionContext = "demote" | "deactivate" | "delete" | "generic";

/**
 * Admin endpoint failures arrive as `ApiError` where `error.message` is the
 * JSON body emitted by the backend's `translate_invariant_violation` (e.g.
 * `{"reason":"last_admin"}` or `{"reason":"owned_resources","chat":N,"agent":M}`).
 * This converts that into a readable sentence the operator can act on.
 */
function humanizeAdminError(err: unknown, ctx: ActionContext): string {
  const raw = err instanceof Error ? err.message : String(err);
  let body: unknown;
  try {
    body = JSON.parse(raw);
  } catch {
    return raw;
  }
  if (!body || typeof body !== "object" || !("reason" in body)) return raw;
  const reason = (body as { reason: string }).reason;

  if (reason === "last_admin") {
    switch (ctx) {
      case "demote":
        return "Can't demote the last admin. Promote another user first.";
      case "deactivate":
        return "Can't deactivate the last admin. Promote another user first.";
      case "delete":
        return "Can't delete the last admin. Promote another user first.";
      default:
        return "At least one active admin must exist.";
    }
  }

  if (reason === "owned_resources") {
    const counts = body as Record<string, unknown>;
    const parts: string[] = [];
    for (const [key, value] of Object.entries(counts)) {
      if (key === "reason") continue;
      const n = typeof value === "number" ? value : 0;
      if (n > 0) parts.push(`${n} ${key}${n === 1 ? "" : "s"}`);
    }
    const owned = parts.length > 0 ? parts.join(" and ") : "owned resources";
    return `Can't delete this user — they still own ${owned}. Reassign or remove those first.`;
  }

  return raw;
}

export function UsersSection() {
  const [users, setUsers] = useState<AdminUser[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [pending, setPending] = useState<PendingConfirm | null>(null);
  const [running, setRunning] = useState(false);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await api.get<AdminUser[]>("/api/admin/users");
      setUsers(list);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load users");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const displayName = (u: AdminUser) => u.name || u.handle;

  const requestAdminToggle = (u: AdminUser, makeAdmin: boolean) => {
    const groups = makeAdmin
      ? Array.from(new Set([...u.groups, ADMINS]))
      : u.groups.filter((g) => g !== ADMINS);
    setPending({
      title: makeAdmin ? "Grant admin?" : "Revoke admin?",
      body: makeAdmin
        ? `${displayName(u)} will gain access to all admin endpoints (managing users, groups, etc.).`
        : `${displayName(u)} will lose admin privileges. They keep their account and can still sign in.`,
      confirmLabel: makeAdmin ? "Grant admin" : "Revoke admin",
      context: makeAdmin ? "generic" : "demote",
      run: async () => {
        await api.patch(`/api/admin/users/${u.id}`, { groups });
      },
    });
  };

  const requestActiveToggle = (u: AdminUser, makeActive: boolean) => {
    setPending({
      title: makeActive ? "Reactivate user?" : "Deactivate user?",
      body: makeActive
        ? `${displayName(u)} will be able to sign in again.`
        : `${displayName(u)} will be immediately signed out and unable to sign in. This is reversible.`,
      confirmLabel: makeActive ? "Reactivate" : "Deactivate",
      danger: !makeActive,
      context: makeActive ? "generic" : "deactivate",
      run: async () => {
        const path = makeActive ? "reactivate" : "deactivate";
        await api.post(`/api/admin/users/${u.id}/${path}`, {});
      },
    });
  };

  const requestDelete = (u: AdminUser) => {
    setPending({
      title: "Delete user?",
      body: `Permanently delete ${displayName(u)} (@${u.handle}). This cannot be undone. The user must have no chats or agents.`,
      confirmLabel: "Delete",
      danger: true,
      context: "delete",
      run: async () => {
        await api.delete(`/api/admin/users/${u.id}`);
      },
    });
  };

  const runPending = async () => {
    if (!pending) return;
    setRunning(true);
    setError(null);
    try {
      await pending.run();
      setPending(null);
      void reload();
    } catch (e) {
      setError(humanizeAdminError(e, pending.context));
      setPending(null);
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="space-y-4">
      <SectionHeader
        title="Users"
        description="Manage user accounts. Admins can list, create, promote, deactivate, and delete users."
        icon={UsersIcon}
      />

      <div className="flex items-center justify-between min-h-[36px]">
        <h4 className="text-base font-medium text-text-secondary">
          {loading ? "Loading…" : `${users.length} ${users.length === 1 ? "user" : "users"}`}
        </h4>
        <button
          type="button"
          onClick={() => setShowCreate(true)}
          className="inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-surface shadow-sm hover:bg-accent-hover transition"
        >
          Add user
        </button>
      </div>

      {error && (
        <div className="rounded-lg bg-error-bg p-3 text-sm text-error-text break-all">
          {error}
        </div>
      )}

      {users.length === 0 && !loading ? (
        <p className="text-sm text-text-tertiary text-center py-8">No users yet</p>
      ) : (
        <div className="rounded-xl border border-border bg-surface-secondary divide-y divide-border">
          {users.map((u) => {
            const isAdmin = u.groups.includes(ADMINS);
            const isActive = u.deactivated_at === null;
            const otherGroupCount = u.groups.filter((g) => g !== ADMINS).length;
            return (
              <div
                key={u.id}
                className={`px-4 py-3 flex items-center gap-3 ${isActive ? "" : "opacity-60"}`}
              >
                <UserCircleIcon className="h-8 w-8 text-text-tertiary shrink-0" />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium text-text-primary truncate">
                      {displayName(u)}
                    </span>
                    {isAdmin && (
                      <span className="rounded-full bg-accent/15 px-2 py-0.5 text-[11px] font-medium text-accent">
                        Admin
                      </span>
                    )}
                    {!isActive && (
                      <span className="rounded-full bg-warning-bg px-2 py-0.5 text-[11px] font-medium text-warning-text">
                        Deactivated
                      </span>
                    )}
                    {otherGroupCount > 0 && (
                      <span className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] font-medium text-text-secondary">
                        +{otherGroupCount} {otherGroupCount === 1 ? "group" : "groups"}
                      </span>
                    )}
                  </div>
                  <div className="text-xs text-text-tertiary truncate">
                    {u.email} · @{u.handle}
                  </div>
                </div>
                <label className="shrink-0 flex items-center gap-1.5 text-xs text-text-secondary cursor-pointer">
                  <input
                    type="checkbox"
                    checked={isAdmin}
                    onChange={(e) => requestAdminToggle(u, e.target.checked)}
                    className="h-3.5 w-3.5 rounded border-border accent-accent"
                  />
                  Admin
                </label>
                <label className="shrink-0 flex items-center gap-1.5 text-xs text-text-secondary cursor-pointer">
                  <input
                    type="checkbox"
                    checked={isActive}
                    onChange={(e) => requestActiveToggle(u, e.target.checked)}
                    className="h-3.5 w-3.5 rounded border-border accent-accent"
                  />
                  Active
                </label>
                <button
                  type="button"
                  onClick={() => requestDelete(u)}
                  aria-label="Delete user"
                  className="shrink-0 inline-flex items-center rounded-lg border border-border px-2 py-1 text-danger hover:bg-surface-tertiary transition"
                >
                  <TrashIcon className="h-3.5 w-3.5" />
                </button>
              </div>
            );
          })}
        </div>
      )}

      {pending && (
        <ConfirmDialog
          title={pending.title}
          body={pending.body}
          confirmLabel={pending.confirmLabel}
          danger={pending.danger}
          running={running}
          onConfirm={runPending}
          onCancel={() => setPending(null)}
        />
      )}

      {showCreate && (
        <CreateUserModal
          onClose={() => setShowCreate(false)}
          onCreated={() => {
            setShowCreate(false);
            void reload();
          }}
        />
      )}
    </div>
  );
}

function ConfirmDialog({
  title,
  body,
  confirmLabel,
  danger,
  running,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: string;
  confirmLabel: string;
  danger?: boolean;
  running: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0 bg-black/50" onClick={running ? undefined : onCancel} />
      <div className="relative rounded-xl border border-border bg-surface-secondary p-4 space-y-4 max-w-lg w-full mx-4 shadow-xl">
        <div className="mb-1 pb-3 border-b border-border flex items-start justify-between gap-3">
          <h3 className="text-lg font-semibold text-text-primary">{title}</h3>
          <ExclamationTriangleIcon
            className={`h-10 w-10 shrink-0 ${danger ? "text-danger" : "text-yellow-500"}`}
          />
        </div>
        <p className="text-sm text-text-secondary">{body}</p>
        <div className="flex gap-2">
          <button
            onClick={onConfirm}
            disabled={running}
            className={`w-28 inline-flex items-center justify-center rounded-lg py-2 text-sm font-medium shadow-sm transition disabled:opacity-50 ${
              danger
                ? "border border-border text-danger hover:bg-surface-tertiary"
                : "bg-accent text-surface hover:bg-accent-hover"
            }`}
          >
            {running ? "Working…" : confirmLabel}
          </button>
          <button
            onClick={onCancel}
            disabled={running}
            className="w-28 inline-flex items-center justify-center rounded-lg border border-border py-2 text-sm font-medium text-text-secondary hover:bg-surface-tertiary disabled:opacity-50 transition"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

function CreateUserModal({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: () => void;
}) {
  const [handle, setHandle] = useState("");
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [isAdmin, setIsAdmin] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      await api.post("/api/admin/users", {
        handle,
        email,
        name,
        password,
        groups: isAdmin ? [ADMINS] : [],
      });
      onCreated();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to create user");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0 bg-black/50" onClick={onClose} />
      <div className="relative rounded-xl border border-border bg-surface-secondary p-5 space-y-4 max-w-sm w-full mx-4 shadow-xl">
        <div className="pb-3 border-b border-border">
          <h3 className="text-lg font-semibold text-text-primary">Add user</h3>
          <p className="text-sm text-text-tertiary mt-1">
            Create a new account directly. Share the password out of band.
          </p>
        </div>
        {error && (
          <div className="rounded-lg bg-error-bg p-3 text-sm text-error-text break-all">{error}</div>
        )}
        <form onSubmit={submit} className="space-y-3">
          <input
            type="text"
            required
            placeholder="Username"
            value={handle}
            onChange={(e) => setHandle(e.target.value)}
            className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-accent focus:outline-none"
          />
          <input
            type="text"
            required
            placeholder="Name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-accent focus:outline-none"
          />
          <input
            type="email"
            required
            placeholder="Email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-accent focus:outline-none"
          />
          <input
            type="password"
            required
            placeholder="Password (min 8 chars)"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-accent focus:outline-none"
          />
          <label className="flex items-center gap-2 text-sm text-text-secondary cursor-pointer">
            <input
              type="checkbox"
              checked={isAdmin}
              onChange={(e) => setIsAdmin(e.target.checked)}
              className="h-4 w-4 rounded border-border accent-accent"
            />
            Grant admin privileges
          </label>
          <div className="flex gap-2 justify-end pt-2">
            <button
              type="button"
              onClick={onClose}
              className="w-28 inline-flex items-center justify-center rounded-lg border border-border py-2 text-sm font-medium text-text-secondary hover:bg-surface-tertiary transition"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={submitting}
              className="w-28 inline-flex items-center justify-center rounded-lg bg-accent py-2 text-sm font-medium text-surface shadow-sm hover:bg-accent-hover disabled:opacity-50 transition"
            >
              {submitting ? "Creating…" : "Create"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
