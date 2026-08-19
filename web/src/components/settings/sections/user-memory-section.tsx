"use client";

import { useCallback, useEffect, useState } from "react";
import { CircleStackIcon, TrashIcon } from "@heroicons/react/24/outline";
import { DeleteConfirmDialog } from "@/components/nav/delete-confirm-dialog";
import { SectionHeader, SectionPanel } from "@/components/settings/field";
import {
  getPkmStatus,
  requestPkmReset,
  type PkmResetStatus,
} from "@/lib/api-client";

export function UserMemorySection() {
  const [available, setAvailable] = useState<boolean | null>(null);
  const [reset, setReset] = useState<PkmResetStatus | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadStatus = useCallback(async () => {
    try {
      const response = await getPkmStatus();
      setAvailable(response.available);
      setReset(response.reset);
      if (response.reset?.status !== "failed") setError(null);
    } catch (statusError) {
      setAvailable(false);
      setError(statusError instanceof Error ? statusError.message : "Memory status is not available");
    }
  }, []);

  useEffect(() => {
    void loadStatus();
  }, [loadStatus]);

  useEffect(() => {
    if (reset?.status !== "pending" && reset?.status !== "running") return;
    const timer = window.setInterval(() => void loadStatus(), 1500);
    return () => window.clearInterval(timer);
  }, [loadStatus, reset?.status]);

  async function submitReset() {
    setSubmitting(true);
    setError(null);
    try {
      const accepted = await requestPkmReset();
      setReset({
        requestId: accepted.requestId,
        status: accepted.status,
        requestedAt: new Date().toISOString(),
        startedAt: null,
        error: null,
      });
      setDialogOpen(false);
    } catch (resetError) {
      setError(resetError instanceof Error ? resetError.message : "Memory reset failed");
    } finally {
      setSubmitting(false);
    }
  }

  const active = reset?.status === "pending" || reset?.status === "running";
  const failed = reset?.status === "failed";

  return (
    <div className="space-y-6">
      <SectionHeader
        title="Memory"
        description="Manage your personal knowledge memory"
        icon={CircleStackIcon}
      />

      {available === false ? (
        <SectionPanel>
          <p className="text-sm text-text-secondary">
            Personal knowledge memory is not active on this server.
          </p>
        </SectionPanel>
      ) : (
        <SectionPanel title="Danger zone">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <div className="text-sm font-medium text-text-primary">Reset memory</div>
              <p className="mt-1 text-sm text-text-secondary">
                Delete your derived memory and let the normal schedule build it again from saved chats.
              </p>
            </div>
            <button
              type="button"
              onClick={() => setDialogOpen(true)}
              disabled={active || available !== true}
              className="inline-flex shrink-0 items-center justify-center gap-2 rounded-lg border border-danger px-3 py-2 text-sm font-medium text-danger transition hover:bg-danger/10 disabled:cursor-not-allowed disabled:opacity-50"
            >
              <TrashIcon className="h-4 w-4" />
              {active ? "Reset running" : failed ? "Retry reset" : "Reset memory"}
            </button>
          </div>
          {active && (
            <p className="text-sm text-text-secondary" role="status">
              Your memory reset is running in the background.
            </p>
          )}
          {failed && (
            <p className="text-sm text-danger" role="alert">
              The reset failed. {reset.error || "Try the reset again."}
            </p>
          )}
          {error && !failed && <p className="text-sm text-danger" role="alert">{error}</p>}
        </SectionPanel>
      )}

      <DeleteConfirmDialog
        open={dialogOpen}
        onCancel={() => setDialogOpen(false)}
        onConfirm={() => void submitReset()}
        title="Reset your memory?"
        message="This permanently deletes all derived PKM memory for your account and deletes your managed Memory directory from disk. Chats, messages, and short-term memories remain. All short-term memories are marked for processing again. Saved chats are reindexed by the normal consolidation schedule. This action cannot be undone."
        confirmLabel="Reset memory"
        confirming={submitting}
        confirmingLabel="Sending…"
      />
    </div>
  );
}
