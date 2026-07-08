import { CheckCircleIcon, XCircleIcon } from "@heroicons/react/16/solid";
import { CheckIcon, HandRaisedIcon } from "@heroicons/react/16/solid";

export function ApprovalResult({
  denied,
  label,
  children,
}: {
  denied: boolean;
  label: string;
  children?: React.ReactNode;
}) {
  const colorClasses = denied
    ? "border-danger/30 bg-danger-bg text-danger-text"
    : "border-success/30 bg-success-bg text-success-text";
  const Icon = denied ? XCircleIcon : CheckCircleIcon;
  return (
    <div
      className={`inline-block rounded-lg border px-3 py-2 text-base my-2 -order-1 ${colorClasses}`}
    >
      <span className="inline-flex items-center gap-1.5">
        <Icon className="h-4 w-4 shrink-0 inline" />
        {label}
      </span>
      {children && <span className="ml-2">{children}</span>}
    </div>
  );
}

export function ApprovalButtons({
  loading,
  onApprove,
  onDeny,
  approveDisabled,
}: {
  loading: boolean;
  onApprove: () => void;
  onDeny: () => void;
  approveDisabled?: boolean;
}) {
  return (
    <div className="flex gap-2">
      <button
        onClick={onApprove}
        disabled={loading || approveDisabled}
        className="w-28 inline-flex items-center justify-center gap-1.5 rounded-lg bg-accent py-2 text-sm font-medium text-surface shadow-sm hover:bg-accent-hover transition disabled:opacity-50"
      >
        <CheckIcon className="h-4 w-4" />
        {loading ? "Approving..." : "Approve"}
      </button>
      <button
        onClick={onDeny}
        disabled={loading}
        className="w-28 inline-flex items-center justify-center gap-1.5 rounded-lg bg-danger py-2 text-sm font-medium text-surface shadow-sm hover:opacity-90 transition"
      >
        <HandRaisedIcon className="h-4 w-4" />
        Decline
      </button>
    </div>
  );
}
