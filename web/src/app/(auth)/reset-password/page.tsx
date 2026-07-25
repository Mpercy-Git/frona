"use client";

import { Suspense, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import Link from "next/link";
import { api } from "@/lib/api-client";
import { Logo } from "@/components/logo";

export default function ResetPasswordPage() {
  return (
    <Suspense>
      <ResetPasswordContent />
    </Suspense>
  );
}

function ResetPasswordContent() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const token = searchParams.get("token") ?? "";

  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState("");

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (password !== confirm) {
      setError("Passwords don't match.");
      return;
    }
    setError("");
    setSubmitting(true);
    try {
      await api.post("/api/auth/reset-password", { token, new_password: password });
      setDone(true);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not reset your password");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="flex min-h-screen items-center justify-center px-4">
      <div className="w-full max-w-sm space-y-6">
        <div className="flex items-center justify-center gap-2">
          <Logo size={80} animate />
          <span className="text-3xl font-bold text-text-primary tracking-wide" style={{ fontFamily: "var(--font-brand)" }}>FRONA</span>
        </div>

        {!token ? (
          <div className="space-y-4">
            <div className="rounded-lg bg-error-bg p-3 text-sm text-error-text">
              This reset link is missing its token. Request a new one.
            </div>
            <p className="text-center text-sm text-text-secondary">
              <Link href="/forgot-password" className="font-medium text-text-primary hover:underline">
                Request a reset link
              </Link>
            </p>
          </div>
        ) : done ? (
          <div className="space-y-4">
            <div className="rounded-lg bg-surface-secondary p-4 text-sm text-text-secondary">
              Your password has been changed, and every existing session was signed out.
            </div>
            <button
              type="button"
              onClick={() => router.replace("/login")}
              className="w-full rounded-lg bg-accent px-4 py-2 text-sm font-medium text-surface hover:bg-accent-hover transition"
            >
              Sign in
            </button>
          </div>
        ) : (
          <>
            <div className="space-y-1">
              <h1 className="text-lg font-semibold text-text-primary text-center">
                Choose a new password
              </h1>
            </div>
            {error && (
              <div className="rounded-lg bg-error-bg p-3 text-sm text-error-text">{error}</div>
            )}
            <form onSubmit={handleSubmit} className="space-y-4">
              <div>
                <label htmlFor="password" className="block text-sm font-medium text-text-secondary">
                  New password
                </label>
                <input
                  id="password"
                  type="password"
                  required
                  minLength={8}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="mt-1 block w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-text-secondary focus:outline-none"
                />
              </div>
              <div>
                <label htmlFor="confirm" className="block text-sm font-medium text-text-secondary">
                  Confirm new password
                </label>
                <input
                  id="confirm"
                  type="password"
                  required
                  minLength={8}
                  value={confirm}
                  onChange={(e) => setConfirm(e.target.value)}
                  className="mt-1 block w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-text-secondary focus:outline-none"
                />
              </div>
              <button
                type="submit"
                disabled={submitting}
                className="w-full rounded-lg bg-accent px-4 py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
              >
                {submitting ? "Saving..." : "Set new password"}
              </button>
            </form>
            <p className="text-center text-sm text-text-secondary">
              <Link href="/login" className="font-medium text-text-primary hover:underline">
                Back to sign in
              </Link>
            </p>
          </>
        )}
      </div>
    </div>
  );
}
