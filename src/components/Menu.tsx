// Anchored popover menu with keyboard navigation.

import { useCallback, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

import { useDismiss } from "./ui";

export interface MenuItem {
  key: string;
  label: ReactNode;
  /** Right-aligned hint, e.g. a shortcut. */
  hint?: string;
  /** Tooltip (e.g. why an item is disabled). */
  title?: string;
  onSelect?: () => void;
  disabled?: boolean;
  checked?: boolean;
  /** Non-interactive group heading. */
  header?: boolean;
  indent?: boolean;
}

export type MenuAnchor = HTMLElement | { x: number; y: number };

export function Menu({
  anchor,
  items,
  onClose,
  label,
  minWidth = 200,
}: {
  anchor: MenuAnchor;
  items: MenuItem[];
  onClose: () => void;
  label: string;
  minWidth?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const close = useCallback(() => {
    onClose();
    if (anchor instanceof HTMLElement) anchor.focus();
  }, [anchor, onClose]);
  useDismiss(ref, true, close);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r =
      anchor instanceof HTMLElement
        ? anchor.getBoundingClientRect()
        : { left: anchor.x, right: anchor.x, top: anchor.y, bottom: anchor.y };
    const { width, height } = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let top = r.bottom + 4;
    if (top + height > vh - 8) top = Math.max(8, r.top - height - 4);
    let left = r.left;
    // Anchors on the right edge (row actions) open leftwards, aligned to their right side.
    if (left + width > vw - 8) left = Math.max(8, Math.min(r.right, vw - 8) - width);
    setPos({ left, top });
    // The checked item (the current choice) when there is one, else the first.
    const first = el.querySelector<HTMLButtonElement>("button[aria-checked='true']:not(:disabled)") ?? el.querySelector<HTMLButtonElement>("button:not(:disabled)");
    first?.focus();
  }, [anchor]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    const buttons = Array.from(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
    const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = e.key === "ArrowDown" ? (i + 1) % buttons.length : (i - 1 + buttons.length) % buttons.length;
      buttons[next]?.focus();
    } else if (e.key === "Tab") {
      e.preventDefault();
      close();
    }
    e.stopPropagation();
  };

  return createPortal(
    <div
      ref={ref}
      className="menu"
      role="menu"
      aria-label={label}
      style={{ left: pos?.left ?? -9999, top: pos?.top ?? -9999, minWidth }}
      onKeyDown={onKeyDown}
    >
      {items.map((it) =>
        it.header ? (
          <div key={it.key} className="menu-header" role="presentation">
            {it.label}
          </div>
        ) : (
          <button
            key={it.key}
            type="button"
            role={it.checked === undefined ? "menuitem" : "menuitemradio"}
            aria-checked={it.checked}
            disabled={it.disabled}
            title={it.title}
            className={`menu-item${it.indent ? " indent" : ""}${it.checked ? " checked" : ""}`}
            onClick={() => {
              close();
              it.onSelect?.();
            }}
          >
            <span className="menu-label">{it.label}</span>
            {it.hint && <span className="menu-hint">{it.hint}</span>}
          </button>
        ),
      )}
    </div>,
    document.body,
  );
}
