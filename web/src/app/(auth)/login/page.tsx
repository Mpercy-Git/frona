"use client";

import { useState, useEffect, Suspense } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import Link from "next/link";
import { useAuth } from "@/lib/auth";
import { Logo } from "@/components/logo";

export default function LoginPage() {
  return (
    <Suspense>
      <LoginContent />
    </Suspense>
  );
}

function LoginContent() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const redirectTo = searchParams.get("redirect") || "/home";
  const { login, user, revalidate, authConfig, initiateSso } = useAuth();

  useEffect(() => {
    revalidate();
  }, [revalidate]);

  useEffect(() => {
    if (user) {
      if (redirectTo.startsWith("http")) {
        window.location.href = redirectTo;
      } else {
        router.replace(redirectTo);
      }
    }
  }, [user, router, redirectTo]);
  const [identifier, setIdentifier] = useState("");
  const [password, setPassword] = useState("");
  const ssoErrorCode = searchParams.get("sso_error");
  const [error, setError] = useState(() => {
    if (!ssoErrorCode) return "";
    const messages: Record<string, string> = {
      email_not_verified: "Your email has not been verified by the SSO provider.",
      csrf_failed: "SSO session expired. Please try again.",
      token_invalid: "SSO authentication failed. Please try again.",
      token_failed: "SSO provider did not return a valid token.",
      sso_disabled: "SSO is not enabled.",
      server_error: "An unexpected error occurred during SSO login.",
      invalid_credentials: "Invalid credentials.",
    };
    return messages[ssoErrorCode] || "SSO login failed. Please try again.";
  });
  const [submitting, setSubmitting] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    setSubmitting(true);
    try {
      await login({ identifier, password });
      if (redirectTo.startsWith("http")) {
        window.location.href = redirectTo;
      } else {
        router.replace(redirectTo);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Login failed");
    } finally {
      setSubmitting(false);
    }
  };

  const showPasswordForm = !authConfig?.sso.disable_local_auth;
  const showSsoButton = authConfig?.sso.enabled;
  const showRegisterLink = authConfig?.allow_registration ?? true;

  return (
    <div className="flex min-h-screen items-center justify-center px-4">
      <div className="w-full max-w-sm space-y-6">
        <div className="flex items-center justify-center gap-2">
          <Logo size={80} animate />
          <span className="text-3xl font-bold text-text-primary tracking-wide" style={{ fontFamily: "var(--font-brand)" }}>FRONA</span>
        </div>
        {error && (
          <div className="rounded-lg bg-error-bg p-3 text-sm text-error-text">
            {error}
          </div>
        )}
        {showPasswordForm && (
          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label htmlFor="identifier" className="block text-sm font-medium text-text-secondary">
                Username or email
              </label>
              <input
                id="identifier"
                type="text"
                required
                value={identifier}
                onChange={(e) => setIdentifier(e.target.value)}
                className="mt-1 block w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-text-secondary focus:outline-none"
              />
            </div>
            <div>
              <label htmlFor="password" className="block text-sm font-medium text-text-secondary">
                Password
              </label>
              <input
                id="password"
                type="password"
                required
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="mt-1 block w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-text-secondary focus:outline-none"
              />
            </div>
            <button
              type="submit"
              disabled={submitting}
              className="w-full rounded-lg bg-accent px-4 py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
            >
              {submitting ? "Signing in..." : "Sign in"}
            </button>
          </form>
        )}
        {showSsoButton && showPasswordForm && (
          <div className="relative">
            <div className="absolute inset-0 flex items-center">
              <div className="w-full border-t border-border" />
            </div>
            <div className="relative flex justify-center text-sm">
              <span className="bg-background px-2 text-text-secondary">or</span>
            </div>
          </div>
        )}
        {showSsoButton && (
          <button
            type="button"
            onClick={initiateSso}
            className="w-full rounded-lg border border-border bg-surface px-4 py-2 text-sm font-medium text-text-primary hover:bg-surface-hover transition"
          >
            Sign in with SSO
          </button>
        )}
        {showPasswordForm && showRegisterLink && (
          <p className="text-center text-sm text-text-secondary">
            Don&apos;t have an account?{" "}
            <Link href="/register" className="font-medium text-text-primary hover:underline">
              Register
            </Link>
          </p>
        )}
      </div>
    </div>
  );
}
