"use client";

import { Component, type ErrorInfo, type ReactNode } from "react";

interface ErrorBoundaryProps {
  children: ReactNode;
  /**
   * Rendered when a descendant throws. `reset` clears the error so the subtree
   * re-mounts and re-renders — useful once the offending data has changed
   * (e.g. streaming finished, message replaced by the final version).
   */
  fallback: (error: Error, reset: () => void) => ReactNode;
  /** Label included in the console error so crashes are traceable in prod. */
  label?: string;
  /** Optional hook for external logging/telemetry. */
  onError?: (error: Error, info: ErrorInfo) => void;
  /**
   * When any value here changes *after* an error, the boundary clears the error
   * and retries. Healthy renders never remount — pass the live inputs (e.g. the
   * streaming text) so a transient throw on partial data self-heals as more
   * arrives, without thrashing the subtree while it's working.
   */
  resetKeys?: readonly unknown[];
}

interface ErrorBoundaryState {
  error: Error | null;
}

function keysChanged(a?: readonly unknown[], b?: readonly unknown[]): boolean {
  if (a === b) return false;
  if (!a || !b || a.length !== b.length) return true;
  return a.some((v, i) => !Object.is(v, b[i]));
}

/**
 * Generic client-side error boundary. React has no hook equivalent, so this
 * stays a class component. Isolating a subtree here means a throw inside it
 * degrades to `fallback` instead of unmounting the whole app — which, with no
 * boundary, forces the user to reload the page. Used most heavily around
 * live-updating surfaces (streaming messages, tool-call views) where a single
 * malformed/partial payload shouldn't take down the conversation.
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`[error-boundary${this.props.label ? `:${this.props.label}` : ""}]`, error, info);
    this.props.onError?.(error, info);
  }

  componentDidUpdate(prev: ErrorBoundaryProps) {
    if (this.state.error && keysChanged(prev.resetKeys, this.props.resetKeys)) {
      this.reset();
    }
  }

  reset = () => this.setState({ error: null });

  render() {
    if (this.state.error) {
      return this.props.fallback(this.state.error, this.reset);
    }
    return this.props.children;
  }
}
