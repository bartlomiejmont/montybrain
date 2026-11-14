// Small shared building blocks.

import {
  forwardRef,
  useEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
} from "react";

import { clearAfterSubmit } from "../lib/form";
import { ADOPT_ATTR, useDraft, useSubmitting, type DraftHandle } from "../store/drafts";

export function ProjectDot({ color, size = 8, title }: { color: string | null | undefined; size?: number; title?: string }) {
  return (
    <span
      className="dot"
      title={title}
      aria-hidden={title ? undefined : true}
      style={{ width: size, height: size, background: color ?? "var(--text-4)" }}
    />
  );
}

/**
 * Subtle note under a field whose stored value changed while the user was editing it.
 * Pressing the button must not count as leaving the field: the mouse press keeps
 * the focus in the field (no blur, so no autosave of the draft first), and a field
 * reached from here by keyboard skips its blur save (see `focusStaysWith`). When
 * focus then moves on from the button to anywhere else, `onLeave` (the field's
 * own leave/save handler) runs instead.
 */
export function DraftHint({
  draft,
  what,
  onLeave,
  fields = [],
}: {
  draft: Pick<DraftHandle, "key" | "elsewhere" | "adoptStored">;
  /** Field label, e.g. "Tytuł". */
  what?: string;
  onLeave?: () => void;
  /** Ids of the field(s) this hint belongs to (focus moving back there is not leaving). */
  fields?: string[];
}) {
  if (!draft.elsewhere) return null;
  return (
    <p className="draft-hint" role="status">
      {what ? `Wartość pola „${what}” zmieniła się` : "Wartość tego pola zmieniła się"} poza aplikacją. Zapis zastosuje
      Twoją wersję.
      <button
        type="button"
        className="draft-hint-use"
        {...{ [ADOPT_ATTR]: draft.key }}
        onMouseDown={(e) => e.preventDefault()}
        onClick={(e) => draft.adoptStored(document.activeElement !== e.currentTarget)}
        onBlur={(e) => {
          const to = e.relatedTarget as HTMLElement | null;
          if (to && fields.includes(to.id)) return;
          onLeave?.();
        }}
      >
        Użyj wersji z pliku
      </button>
    </p>
  );
}

type BtnProps = ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "ghost" | "primary" | "danger" | "quiet" };

export const Button = forwardRef<HTMLButtonElement, BtnProps>(function Button(
  { variant = "ghost", className, type = "button", ...rest },
  ref,
) {
  return <button ref={ref} type={type} className={`btn btn-${variant} ${className ?? ""}`} {...rest} />;
});

export const IconButton = forwardRef<HTMLButtonElement, ButtonHTMLAttributes<HTMLButtonElement> & { label: string }>(
  function IconButton({ label, className, type = "button", ...rest }, ref) {
    return <button ref={ref} type={type} aria-label={label} title={label} className={`icon-btn ${className ?? ""}`} {...rest} />;
  },
);

/** "Zapisz" / "Anuluj" of an explicit-save editor; both wait while a save is in flight. */
export function SaveActions({ busy, onCancel }: { busy: boolean; onCancel: () => void }) {
  return (
    <div className="editor-actions">
      <Button type="submit" variant="primary" disabled={busy} aria-busy={busy || undefined}>
        {busy ? "Zapisywanie…" : "Zapisz"}
      </Button>
      <Button variant="quiet" disabled={busy} onClick={onCancel}>
        Anuluj
      </Button>
    </div>
  );
}

/** Two-step destructive action: first click asks inline, second confirms. */
export function ConfirmButton({
  children,
  question,
  confirmLabel,
  onConfirm,
  autoOpen = false,
  onCancel,
}: {
  children: ReactNode;
  question: string;
  confirmLabel: string;
  onConfirm: () => void;
  autoOpen?: boolean;
  onCancel?: () => void;
}) {
  const [asking, setAsking] = useState(autoOpen);
  const confirmRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (asking) confirmRef.current?.focus();
  }, [asking]);
  useEffect(() => setAsking(autoOpen), [autoOpen]);
  if (!asking) {
    return (
      <Button variant="quiet" onClick={() => setAsking(true)}>
        {children}
      </Button>
    );
  }
  return (
    <span className="confirm" role="group" aria-label={question}>
      <span className="confirm-q">{question}</span>
      <Button ref={confirmRef} variant="danger" onClick={onConfirm}>
        {confirmLabel}
      </Button>
      <Button
        variant="quiet"
        onClick={() => {
          setAsking(false);
          onCancel?.();
        }}
        onKeyDown={(e) => e.key === "Escape" && setAsking(false)}
      >
        Anuluj
      </Button>
    </span>
  );
}

/**
 * Text input that submits on Enter and clears itself; Escape blurs.
 * `onSubmit` may return false to keep the text (e.g. on error).
 * While a submission is in flight the input is read-only (also after it is
 * re-created), and only the submitted text is cleared afterwards.
 */
export function SubmitInput({
  onSubmit,
  className,
  autoFocus,
  onEscape,
  draftKey,
  ...rest
}: Omit<InputHTMLAttributes<HTMLInputElement>, "onSubmit"> & {
  onSubmit: (value: string) => Promise<boolean | void> | boolean | void;
  onEscape?: () => void;
  /**
   * Identifies the field by what it adds to (a list, a task), so the typed text
   * survives the component being re-created (e.g. its list moves).
   */
  draftKey: string;
}) {
  const key = `si:${draftKey}`;
  const { draft: value, setDraft: setValue, state } = useDraft(key, "");
  const submitting = useSubmitting(key);
  return (
    <input
      {...rest}
      autoFocus={autoFocus}
      className={`field ${className ?? ""}`}
      value={value}
      disabled={rest.disabled || submitting.busy}
      onChange={(e) => setValue(e.target.value)}
      onKeyDown={async (e) => {
        if (e.key === "Escape") {
          setValue("");
          onEscape?.();
          e.currentTarget.blur();
          return;
        }
        if (e.key !== "Enter" || e.nativeEvent.isComposing) return;
        e.preventDefault();
        const v = value.trim();
        if (!v || !submitting.begin()) return;
        const input = e.currentTarget;
        let ok = false;
        try {
          ok = (await onSubmit(v)) !== false;
        } finally {
          submitting.end();
        }
        const now = state().draft;
        const next = clearAfterSubmit(ok, now, value);
        if (next !== now) setValue(next);
        requestAnimationFrame(() => input.isConnected && input.focus());
      }}
    />
  );
}

/** Segmented single-choice control. */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
  disabled,
}: {
  value: T;
  options: { value: T; label: string; hint?: string; disabled?: boolean }[];
  onChange: (v: T) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <div className="segmented" role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          title={o.hint}
          disabled={disabled || o.disabled}
          className={o.value === value ? "on" : undefined}
          onClick={() => o.value !== value && onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Calls `fn` on pointerdown outside `ref` and on Escape. */
export function useDismiss(ref: React.RefObject<HTMLElement | null>, open: boolean, fn: () => void) {
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) fn();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        fn();
      }
    };
    document.addEventListener("pointerdown", onDown, true);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("pointerdown", onDown, true);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [ref, open, fn]);
}

/** localStorage-backed boolean (a per-viewer convenience; falls back to memory). */
export function useStoredFlag(key: string, initial: boolean): [boolean, (v: boolean) => void] {
  const [v, setV] = useState<boolean>(() => {
    try {
      const raw = window.localStorage.getItem(key);
      return raw === null ? initial : raw === "1";
    } catch {
      return initial;
    }
  });
  const set = (next: boolean) => {
    setV(next);
    try {
      window.localStorage.setItem(key, next ? "1" : "0");
    } catch {
      /* storage unavailable */
    }
  };
  return [v, set];
}
