"use client";

import { useState, useEffect, useMemo, useCallback, useId } from "react";
import { VaultItemPicker, type CredentialOption } from "@/components/vault-item-picker";
import { api } from "@/lib/api-client";
import { takeoverHref } from "@/lib/browser-live";
import type { CredentialRequestItem, CredentialTarget, GrantDuration, HitlResponse, SkillCandidate, ToolCall, VaultField } from "@/lib/types";
import { ApprovalButtons } from "./approval-parts";

function Label({ children }: { children: React.ReactNode }) {
  return <label className="block text-sm font-medium text-text-tertiary mb-1">{children}</label>;
}

export interface ToolContentProps {
  te: ToolCall;
  chatId: string;
  /**
   * Called when the user produces a response. The wizard submits all
   * collected responses in a single batch via the unified resolve endpoint.
   * `displayText` is what we show in the wizard chip for "selected answer".
   */
  onResolve: (response: HitlResponse, displayText: string) => void;
}

export function QuestionContent({ te, onResolve, selectedAnswer }: ToolContentProps & { selectedAnswer?: string }) {
  const hitl = te.hitl;
  if (!hitl || hitl.request.type !== "Question") return null;
  const question = hitl.prompt;
  const options = hitl.request.data.options;

  return (
    <div className="space-y-2">
      <p className="text-sm text-text-primary">{question}</p>
      {options.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {options.map((option) => (
            <button
              key={option}
              onClick={() => onResolve({ type: "Choice", data: option }, option)}
              className={`rounded-lg border px-2.5 py-1 text-xs font-medium transition ${
                selectedAnswer === option
                  ? "border-accent bg-accent/10 text-accent"
                  : "border-border text-text-secondary hover:border-accent hover:text-accent"
              }`}
            >
              {option}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function TakeoverContent({ te, onResolve }: ToolContentProps) {
  const hitl = te.hitl;
  if (!hitl || hitl.request.type !== "Takeover") return null;
  const { reason, debugger_url } = hitl.request.data;

  return (
    <div className="space-y-2">
      <p className="text-sm text-text-primary">{reason}</p>
      <div className="flex flex-wrap gap-1.5">
        <a
          href={takeoverHref(debugger_url)}
          target="_blank"
          rel="noopener noreferrer"
          className="rounded-lg border border-border px-2.5 py-1 text-xs font-medium text-text-secondary hover:border-accent hover:text-accent transition"
        >
          Open Live Browser
        </a>
        <button
          onClick={() => onResolve({ type: "Choice", data: "Done" }, "Done")}
          className="rounded-lg border border-border px-2.5 py-1 text-xs font-medium text-text-secondary hover:border-accent hover:text-accent transition"
        >
          Resume Agent
        </button>
      </div>
    </div>
  );
}

interface VaultItem {
  id: string;
  name: string;
  username?: string;
}

interface VaultConnection {
  id: string;
  name: string;
  provider: string;
  enabled: boolean;
}

function defaultPrefix(query: string): string {
  return query.toUpperCase().replace(/[^A-Z0-9]+/g, "_").replace(/^_+|_+$/g, "");
}

interface SlotGrant {
  connection_id: string;
  vault_item_id: string;
  target: CredentialTarget;
}

/**
 * One credential in a (possibly batched) request: pick the vault, find the
 * item, and choose how it's exposed as env vars. Reports the built grant (or
 * `null` while incomplete) to the parent, which collects one per slot and
 * submits them together.
 */
function CredentialSlot({
  item,
  index,
  showHeader,
  connections,
  onChange,
}: {
  item: CredentialRequestItem;
  index: number;
  showHeader: boolean;
  connections: VaultConnection[];
  onChange: (index: number, grant: SlotGrant | null) => void;
}) {
  const [selection, setSelection] = useState<CredentialOption | null>(null);
  const [bindingMode, setBindingMode] = useState<"prefix" | "single">("prefix");
  const [envVarPrefix, setEnvVarPrefix] = useState(defaultPrefix(item.query));
  const [envVar, setEnvVar] = useState<string | null>(null);
  const [fields, setFields] = useState<string[]>([]);
  const [selectedField, setSelectedField] = useState("");
  const [loadingFields, setLoadingFields] = useState(false);
  const [fieldError, setFieldError] = useState(false);
  const [fieldVersion, setFieldVersion] = useState(0);
  const inputId = useId();
  const selectedConnectionId = selection?.connection_id;
  const selectedItemId = selection?.id;

  useEffect(() => {
    setFields([]);
    setSelectedField("");
    setFieldError(false);
    setLoadingFields(!!selectedItemId);
    if (!selectedConnectionId || !selectedItemId) return;
    const controller = new AbortController();
    api.get<string[]>(`/api/vaults/${encodeURIComponent(selectedConnectionId)}/items/${encodeURIComponent(selectedItemId)}/fields`)
      .then((availableFields) => {
        if (controller.signal.aborted) return;
        setFields(availableFields);
        setSelectedField(availableFields.includes("PASSWORD") ? "PASSWORD" : availableFields[0] ?? "");
      })
      .catch(() => { if (!controller.signal.aborted) setFieldError(true); })
      .finally(() => { if (!controller.signal.aborted) setLoadingFields(false); });
    return () => controller.abort();
  }, [selectedConnectionId, selectedItemId, fieldVersion]);

  const prefix = envVarPrefix.trim();
  const variableName = envVar ?? (selectedField ? `${prefix ? `${prefix}_` : ""}${selectedField}` : "");
  const fieldsReady = !!selection && !loadingFields && !fieldError && fields.length > 0;

  const target = useMemo<CredentialTarget | null>(() => {
    if (bindingMode === "prefix") {
      if (!prefix) return null;
      return { Prefix: { env_var_prefix: prefix } };
    }
    const name = variableName.trim();
    if (!name || !selectedField) return null;
    const field: VaultField = selectedField === "PASSWORD" ? "Password"
      : selectedField === "USERNAME" ? "Username"
      : { Custom: { name: selectedField } };
    return { Single: { env_var: name, field } };
  }, [bindingMode, prefix, variableName, selectedField]);

  useEffect(() => {
    if (!selection || !fieldsReady || !target) {
      onChange(index, null);
    } else {
      onChange(index, { connection_id: selection.connection_id, vault_item_id: selection.id, target });
    }
  }, [selection, fieldsReady, target, index, onChange]);

  return (
    <div className={showHeader ? "space-y-3 rounded-lg border border-border p-3" : "space-y-3"}>
      {showHeader && (
        <p className="text-sm font-medium text-text-primary">
          {index + 1}. {item.label ?? item.query}
        </p>
      )}

      <VaultItemPicker
        connections={connections}
        selection={selection}
        onSelect={(picked) => {
          setSelection(picked);
          setFields([]);
          setSelectedField("");
          setLoadingFields(!!picked);
          setFieldVersion((version) => version + 1);
          if (picked && !envVarPrefix) setEnvVarPrefix(defaultPrefix(item.query) || defaultPrefix(picked.name) || "CREDENTIAL");
        }}
        initialQuery={item.query}
      />

      {selection && <fieldset className="space-y-2">
        <legend className="block text-sm font-medium text-text-tertiary mb-2">What should the agent use?</legend>
        <div className="flex gap-1.5">
          {([
            { value: "prefix", label: "Entire credential" },
            { value: "single", label: "A specific field" },
          ] as const).map((mode) => (
            <button
              key={mode.value}
              type="button"
              aria-pressed={bindingMode === mode.value}
              onClick={() => setBindingMode(mode.value)}
              className={`flex-1 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition ${
                bindingMode === mode.value
                  ? "border-accent bg-accent/10 text-accent"
                  : "border-border text-text-secondary hover:border-accent"
              }`}
            >
              {mode.label}
            </button>
          ))}
        </div>
        {loadingFields ? (
          <p role="status" className="text-xs text-text-tertiary">Loading fields...</p>
        ) : fieldError ? (
          <p role="alert" className="text-xs text-danger">
            Could not load credential fields.{" "}
            <button type="button" className="underline" onClick={() => setFieldVersion((version) => version + 1)}>Retry</button>
          </p>
        ) : fields.length === 0 ? (
          <p className="text-xs text-text-tertiary">This credential has no available fields.</p>
        ) : bindingMode === "single" ? (
          <div className="space-y-2">
            <select
              id={`${inputId}-field`}
              aria-label="Field to share"
              value={selectedField}
              onChange={(e) => setSelectedField(e.target.value)}
              className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary"
            >
              {fields.map((field) => (
                <option key={field} value={field}>
                  {field === "PASSWORD" ? "Password" : field === "USERNAME" ? "Username" : field === "API_KEY" ? "API key" : field}
                </option>
              ))}
            </select>
          </div>
        ) : null}

        {fieldsReady && target && (
          <div className="flex flex-wrap gap-1.5" aria-label="Environment variable preview" aria-live="polite">
            {(bindingMode === "prefix" ? fields.map((field) => `${prefix}_${field}`) : [variableName.trim()]).map((name) => (
              <code key={name} className="rounded bg-surface-tertiary px-2 py-1 text-xs text-text-primary">{name}</code>
            ))}
          </div>
        )}

        <details className="text-xs text-text-tertiary">
          <summary className="cursor-pointer py-1">Advanced</summary>
          <div className="space-y-2 pt-2">
            {bindingMode === "prefix" ? (<>
              <label htmlFor={`${inputId}-prefix`} className="block font-medium">Variable name prefix</label>
              <input
                id={`${inputId}-prefix`}
                value={envVarPrefix}
                onChange={(e) => setEnvVarPrefix(e.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, ""))}
                placeholder="DB"
                className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm font-mono text-text-primary placeholder:text-text-tertiary"
              />
            </>) : (<>
              <label htmlFor={`${inputId}-variable`} className="block font-medium">Environment variable name</label>
              <input
                id={`${inputId}-variable`}
                value={variableName}
                onChange={(e) => setEnvVar(e.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, ""))}
                placeholder="DB_PASSWORD"
                className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm font-mono text-text-primary placeholder:text-text-tertiary"
              />
            </>)}
          </div>
        </details>
      </fieldset>}
    </div>
  );
}

/**
 * Credential approval. Handles both a single `Credential` request and a
 * batched `Credentials` request — the batch renders one slot per key so the
 * user provides every secret an API needs (app key, user key, …) in one go
 * and a single Duration + Approve resolves them all together.
 */
export function CredentialContent({ te, onResolve }: ToolContentProps) {
  const hitl = te.hitl;

  const items = useMemo<CredentialRequestItem[]>(() => {
    if (hitl?.request.type === "Credential") return [{ query: hitl.request.data.query }];
    if (hitl?.request.type === "Credentials") return hitl.request.data.items;
    return [];
  }, [hitl]);

  const reason =
    hitl?.request.type === "Credential"
      ? hitl.request.data.reason
      : hitl?.request.type === "Credentials"
        ? hitl.request.data.reason
        : "";

  const [connections, setConnections] = useState<VaultConnection[]>([]);
  const [loadingConnections, setLoadingConnections] = useState(true);
  const [connectionError, setConnectionError] = useState(false);
  const [connectionVersion, setConnectionVersion] = useState(0);
  const [duration, setDuration] = useState<GrantDuration>("once");
  const [grants, setGrants] = useState<(SlotGrant | null)[]>(() => items.map(() => null));

  useEffect(() => {
    const controller = new AbortController();
    setLoadingConnections(true);
    setConnectionError(false);
    api.get<VaultConnection[]>("/api/vaults")
      .then((conns) => { if (!controller.signal.aborted) setConnections(conns.filter((c) => c.enabled)); })
      .catch(() => { if (!controller.signal.aborted) setConnectionError(true); })
      .finally(() => { if (!controller.signal.aborted) setLoadingConnections(false); });
    return () => controller.abort();
  }, [connectionVersion]);

  // Keep the grants array aligned with the requested items.
  useEffect(() => {
    setGrants((prev) => items.map((_, i) => prev[i] ?? null));
  }, [items]);

  const handleSlotChange = useCallback((index: number, grant: SlotGrant | null) => {
    setGrants((prev) => {
      const next = prev.slice();
      next[index] = grant;
      return next;
    });
  }, []);

  if (!hitl || (hitl.request.type !== "Credential" && hitl.request.type !== "Credentials")) {
    return null;
  }

  const multiple = items.length > 1;
  const allReady = grants.length === items.length && grants.every((g) => g !== null);

  const handleApprove = () => {
    if (!allReady) return;
    const built = items.map((item, i) => {
      const g = grants[i] as SlotGrant;
      return {
        query: item.query,
        connection_id: g.connection_id,
        vault_item_id: g.vault_item_id,
        grant_duration: duration,
        target: g.target,
      };
    });
    onResolve(
      { type: "Vault", data: { type: "GrantedMany", data: { grants: built } } },
      multiple ? `Granted ${built.length}` : "Approved",
    );
  };

  const handleDeny = () => {
    onResolve({ type: "Vault", data: { type: "Denied" } }, "Denied");
  };

  const durationValue = typeof duration === "string" ? duration : "hours" in duration ? "hours" : "days";

  return (
    <div className="space-y-3">
      <p className="text-sm text-text-tertiary">{reason}</p>

      {loadingConnections ? (
        <p role="status" className="text-xs text-text-tertiary">Loading vaults...</p>
      ) : connectionError ? (
        <p role="alert" className="text-xs text-danger">
          Could not load vaults.{" "}
          <button type="button" onClick={() => setConnectionVersion((version) => version + 1)} className="underline">Retry</button>
        </p>
      ) : items.map((item, i) => (
        <CredentialSlot
          key={`${i}-${item.query}`}
          item={item}
          index={i}
          showHeader={multiple}
          connections={connections}
          onChange={handleSlotChange}
        />
      ))}

      <div>
        <Label>Duration</Label>
        <select
          value={durationValue}
          onChange={(e) => {
            const v = e.target.value;
            if (v === "once") setDuration("once");
            else if (v === "permanent") setDuration("permanent");
            else if (v === "hours") setDuration({ hours: 24 });
            else if (v === "days") setDuration({ days: 7 });
          }}
          className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary"
        >
          <option value="once">Allow once</option>
          <option value="hours">Allow for 24 hours</option>
          <option value="days">Allow for 7 days</option>
          <option value="permanent">Allow permanently</option>
        </select>
      </div>

      <ApprovalButtons loading={false} onApprove={handleApprove} onDeny={handleDeny} approveDisabled={!allReady} />
    </div>
  );
}

export function AppContent({ te, onResolve }: ToolContentProps) {
  const hitl = te.hitl;
  if (!hitl || hitl.request.type !== "App") return null;
  const { action, manifest } = hitl.request.data;
  const name = String(manifest?.name || manifest?.id || "Unknown service");
  const description = manifest?.description ? String(manifest.description) : null;
  const command = manifest?.command ? String(manifest.command) : null;

  const handleApprove = () => {
    onResolve({ type: "Approval", data: true }, "Approved");
  };

  const handleDeny = () => {
    onResolve({ type: "Approval", data: false }, "Denied");
  };

  return (
    <div className="space-y-3">
      <div>
        <p className="text-sm font-medium text-text-primary capitalize">{action} service: {name}</p>
        {description && <p className="text-xs text-text-tertiary mt-1">{description}</p>}
      </div>
      {command && (
        <div>
          <Label>Command</Label>
          <code className="block rounded-lg border border-border bg-surface-secondary px-3 py-2 text-xs font-mono text-text-secondary overflow-x-auto">
            {command}
          </code>
        </div>
      )}
      <ApprovalButtons loading={false} onApprove={handleApprove} onDeny={handleDeny} />
    </div>
  );
}

/**
 * Skill-install approval. The agent found skills it doesn't have; nothing is
 * written until this is approved, so the list has to say plainly what would be
 * added, from where, and who ends up with it.
 */
export function SkillsContent({ te, onResolve }: ToolContentProps) {
  const hitl = te.hitl;
  if (!hitl || hitl.request.type !== "Skills") return null;
  const { items, scope, reason } = hitl.request.data;

  const approveLabel = items.length === 1 ? `Install ${items[0].name}` : `Install ${items.length} skills`;

  return (
    <div className="space-y-3">
      <div>
        <p className="text-sm font-medium text-text-primary">{approveLabel}</p>
        <p className="text-xs text-text-tertiary mt-1">{reason}</p>
      </div>
      <div>
        <Label>{scope === "user" ? "Available to all your agents" : "Available to this agent only"}</Label>
        <ul className="space-y-1.5">
          {items.map((item: SkillCandidate) => (
            <li
              key={`${item.repo}/${item.name}`}
              className="rounded-lg border border-border bg-surface-secondary px-3 py-2"
            >
              <div className="flex flex-wrap items-baseline gap-x-2">
                <span className="text-sm font-medium text-text-primary">{item.name}</span>
                <a
                  href={`https://github.com/${item.repo}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-xs font-mono text-text-tertiary hover:text-accent"
                >
                  {item.repo}
                </a>
              </div>
              {item.description && (
                <p className="mt-1 text-xs text-text-secondary">{item.description}</p>
              )}
            </li>
          ))}
        </ul>
      </div>
      <ApprovalButtons
        loading={false}
        onApprove={() => onResolve({ type: "Approval", data: true }, "Approved")}
        onDeny={() => onResolve({ type: "Approval", data: false }, "Denied")}
      />
    </div>
  );
}

export function ToolContentDispatch(props: ToolContentProps & { selectedAnswer?: string }) {
  switch (props.te.hitl?.request.type) {
    case "Question":
      return <QuestionContent {...props} />;
    case "Takeover":
      return <TakeoverContent {...props} />;
    case "Credential":
    case "Credentials":
      return <CredentialContent {...props} />;
    case "App":
      return <AppContent {...props} />;
    case "Skills":
      return <SkillsContent {...props} />;
    default:
      return null;
  }
}
