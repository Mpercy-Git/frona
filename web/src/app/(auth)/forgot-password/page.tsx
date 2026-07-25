"use client";

import { useState } from "react";
import Link from "next/link";
import { api } from "@/lib/api-client";
import { Logo } from "@/components/logo";

export default function ForgotPasswordPage() {
  const [email, setEmail] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [sent, setSent] = useState(false);
  const [error, setError] = useState("");

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    setSubmitting(true);
    try {
      await api.post("/api/auth/forgot-password", { email });
      setSent(true);
    } catch (err) {
      // Only server-level failures land here — an unknown address still
      // succeeds, so this never reveals whether the account exists.
      setError(err instanceof Error ? err.message : "Something went wrong");
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

        {sent ? (
          <div className="space-y-4">
            <div className="rounded-lg bg-surface-secondary p-4 text-sm text-text-secondary">
              If an account exists for <span className="text-text-primary">{email}</span>, a
              reset link is on its way. The link works once and expires shortly.
            </div>
            <p className="text-center text-sm text-text-secondary">
              <Link href="/login" className="font-medium text-text-primary hover:underline">
                Back to sign in
              </Link>
            </p>
          </div>
        ) : (
          <>
            <div className="space-y-1">
              <h1 className="text-lg font-semibold text-text-primary text-center">
                Reset your password
              </h1>
              <p className="text-sm text-text-secondary text-center">
                Enter your email and we&apos;ll send you a link to choose a new password.
              </p>
            </div>
            {error && (
              <div className="rounded-lg bg-error-bg p-3 text-sm text-error-text">{error}</div>
            )}
            <form onSubmit={handleSubmit} className="space-y-4">
              <div>
                <label htmlFor="email" className="block text-sm font-medium text-text-secondary">
                  Email
                </label>
                <input
                  id="email"
                  type="email"
                  required
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  className="mt-1 block w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-text-secondary focus:outline-none"
                />
              </div>
              <button
                type="submit"
                disabled={submitting}
                className="w-full rounded-lg bg-accent px-4 py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
              >
                {submitting ? "Sending..." : "Send reset link"}
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
