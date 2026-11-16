import { t as tr } from "../i18n";
// The round status control at the start of a task row.

import { forwardRef } from "react";

import type { TaskStatus } from "../api";
import { TASK_STATUS_LABEL } from "../lib/labels";

export const StatusToggle = forwardRef<
  HTMLButtonElement,
  {
    status: TaskStatus;
    onToggle: () => void;
    onMenu: (anchor: HTMLElement) => void;
    disabled?: boolean;
  }
>(function StatusToggle({ status, onToggle, onMenu, disabled }, ref) {
  const label = tr("{0} — kliknij, aby {1}; prawy przycisk: inne statusy", TASK_STATUS_LABEL[status], status === "done" ? tr("przywrócić") : tr("oznaczyć jako zrobione"));
  return (
    <button
      ref={ref}
      type="button"
      className={`status status-${status}`}
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation();
        if (e.altKey) onMenu(e.currentTarget);
        else onToggle();
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onMenu(e.currentTarget);
      }}
    >
      <StatusGlyph status={status} />
    </button>
  );
});

export function StatusGlyph({ status }: { status: TaskStatus }) {
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      {status === "done" ? (
        <>
          <circle cx="8" cy="8" r="7" className="g-fill" />
          <path d="M4.8 8.2 7 10.3l4.2-4.6" className="g-check" />
        </>
      ) : (
        <>
          <circle cx="8" cy="8" r="6.25" className="g-ring" />
          {status === "doing" && <path d="M8 3.25a4.75 4.75 0 0 1 0 9.5z" className="g-half" />}
          {status === "blocked" && <path d="M5.2 8h5.6" className="g-bar" />}
        </>
      )}
    </svg>
  );
}
