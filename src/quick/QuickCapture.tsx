// The quick-capture window: one input, a compact destination picker, Enter
// saves, Esc closes. Same quick-add tokens as everywhere else (!wysoki, @jutro,
// @2026-11-20T09:00). The window is created hidden once and re-shown by the
// global shortcut; every show resets it (`onQuickShow`).

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import { api } from "../api";
import type { NewTask, Snapshot, TaskLocation } from "../api/types";
import { formatDue, toIsoDate } from "../lib/dates";
import { describeError } from "../lib/errors";
import { parseQuickAdd } from "../lib/quickadd";
import { destinations, filterDestinations, resolveDestination, type Destination } from "./destinations";

const CLOSE_AFTER_SAVE_MS = 850;

function Dot({ color }: { color: string | null }) {
  return <span className="qc-dot" style={{ background: color ?? "var(--text-4)" }} aria-hidden />;
}

function tokenChips(task: NewTask, today: string): string[] {
  const out: string[] = [];
  if (task.due) out.push(formatDue(task.due, today));
  if (task.priority === "high") out.push("wysoki");
  if (task.priority === "low") out.push("niski");
  return out;
}

export function QuickCapture() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [saved, setSaved] = useState<TaskLocation | null>(null);
  const [value, setValue] = useState("");
  const [picking, setPicking] = useState(false);
  const [filter, setFilter] = useState("");
  const [cursor, setCursor] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<{ title: string; where: string } | null>(null);

  const boxRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const closeTimer = useRef<number | undefined>(undefined);

  const all = useMemo(() => (snap ? destinations(snap) : []), [snap]);
  const dest = all.length ? resolveDestination(all, saved) : null;
  const shown = useMemo(() => filterDestinations(all, filter), [all, filter]);
  const today = toIsoDate(new Date());
  const parsed = useMemo(() => parseQuickAdd(value, today), [value, today]);
  const chips = value.trim() ? tokenChips(parsed, today) : [];

  const load = useCallback(async () => {
    try {
      const [view, settings] = await Promise.all([api.getState(), api.getSettings()]);
      setSnap(view.snapshot);
      setSaved(settings.quick_destination);
    } catch (e) {
      setError(describeError(e));
    }
  }, []);

  const reset = useCallback(() => {
    window.clearTimeout(closeTimer.current);
    setValue("");
    setPicking(false);
    setFilter("");
    setError(null);
    setDone(null);
    setBusy(false);
    void load();
    requestAnimationFrame(() => inputRef.current?.focus());
  }, [load]);

  useEffect(() => {
    reset();
    let off: (() => void) | undefined;
    let disposed = false;
    void api.onQuickShow(reset).then((u) => (disposed ? u() : (off = u)));
    return () => {
      disposed = true;
      off?.();
    };
  }, [reset]);

  // Keep the native window as tall as the content.
  useLayoutEffect(() => {
    const el = boxRef.current;
    if (!el) return;
    const fit = () => void api.quickResize(Math.ceil(el.getBoundingClientRect().height));
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    if (picking) requestAnimationFrame(() => filterRef.current?.focus());
  }, [picking]);

  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-index="${cursor}"]`)?.scrollIntoView({ block: "nearest" });
  }, [cursor, picking]);

  const close = useCallback(() => {
    window.clearTimeout(closeTimer.current);
    void api.quickClose();
  }, []);

  function openPicker() {
    setFilter("");
    setCursor(Math.max(0, dest ? all.findIndex((d) => d.key === dest.key) : 0));
    setPicking(true);
  }

  function closePicker() {
    setPicking(false);
    requestAnimationFrame(() => inputRef.current?.focus());
  }

  function choose(d: Destination) {
    setSaved(d.location);
    closePicker();
    void api.updateSettings({ quick_destination: d.location }).catch(() => {});
  }

  async function save() {
    if (busy || done || !dest) return;
    if (!value.trim()) return;
    if (!parsed.title.trim()) {
      setError("Wpisz tytuł zadania.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await api.createTask(dest.location, parsed);
      setDone({ title: parsed.title, where: dest.full });
      setValue("");
      closeTimer.current = window.setTimeout(close, CLOSE_AFTER_SAVE_MS);
    } catch (e) {
      setError(describeError(e));
      requestAnimationFrame(() => inputRef.current?.focus());
    } finally {
      setBusy(false);
    }
  }

  function onInputKey(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.nativeEvent.isComposing) return;
    if (e.key === "Enter") {
      e.preventDefault();
      void save();
    } else if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Tab") {
      e.preventDefault();
      openPicker();
    }
  }

  function onFilterKey(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.nativeEvent.isComposing) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setCursor((c) => Math.min(shown.length - 1, c + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setCursor((c) => Math.max(0, c - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const d = shown[cursor];
      if (d) choose(d);
    } else if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      closePicker();
    }
  }

  return (
    <div className="qc" ref={boxRef} onKeyDown={(e) => done && e.key === "Escape" && close()}>
      {done ? (
        <div className="qc-row qc-done" role="status">
          <span className="qc-check" aria-hidden>
            <svg viewBox="0 0 16 16" width="16" height="16">
              <circle cx="8" cy="8" r="7.25" fill="currentColor" />
              <path d="M4.8 8.3l2.1 2.1 4.3-4.6" fill="none" stroke="var(--bg)" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </span>
          <span className="qc-done-text">
            Dodano <span className="qc-done-where">· {done.where}</span>
          </span>
        </div>
      ) : (
        <div className="qc-row" data-tauri-drag-region>
          <span className="qc-ring" aria-hidden />
          <input
            ref={inputRef}
            className="qc-input"
            value={value}
            placeholder="Nowe zadanie…"
            aria-label="Nowe zadanie"
            spellCheck={false}
            autoComplete="off"
            disabled={busy}
            onChange={(e) => {
              setValue(e.target.value);
              setError(null);
            }}
            onKeyDown={onInputKey}
          />
        </div>
      )}

      {!done && (
        <div className="qc-foot">
          <button
            type="button"
            className="qc-dest"
            aria-haspopup="listbox"
            aria-expanded={picking}
            title="Gdzie zapisać (Tab)"
            onClick={() => (picking ? closePicker() : openPicker())}
          >
            {dest && <Dot color={dest.color} />}
            <span className="qc-dest-label">{dest?.full ?? "Skrzynka"}</span>
            <svg className="qc-chev" viewBox="0 0 10 10" width="9" height="9" aria-hidden>
              <path d="M2 3.5l3 3 3-3" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
            </svg>
          </button>
          {error ? (
            <span className="qc-error" role="alert">
              {error}
            </span>
          ) : (
            <span className="qc-chips" aria-live="polite">
              {chips.map((c) => (
                <span key={c} className="qc-chip">
                  {c}
                </span>
              ))}
            </span>
          )}
          <span className="qc-hints" aria-hidden>
            <kbd>↵</kbd> zapisz <kbd>⇥</kbd> miejsce <kbd>esc</kbd>
          </span>
        </div>
      )}

      {picking && !done && (
        <div className="qc-picker">
          <input
            ref={filterRef}
            className="qc-filter"
            value={filter}
            placeholder="Szukaj projektu lub tematu"
            aria-label="Szukaj miejsca"
            aria-controls="qc-list"
            aria-activedescendant={shown[cursor] ? `qc-opt-${cursor}` : undefined}
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => {
              setFilter(e.target.value);
              setCursor(0);
            }}
            onKeyDown={onFilterKey}
          />
          <ul id="qc-list" className="qc-list" role="listbox" aria-label="Miejsce zapisu" ref={listRef}>
            {shown.map((d, i) => (
              <li
                key={d.key}
                id={`qc-opt-${i}`}
                data-index={i}
                role="option"
                aria-selected={i === cursor}
                className={`qc-opt${d.depth ? " sub" : ""}${i === cursor ? " active" : ""}`}
                onMouseMove={() => setCursor(i)}
                onMouseDown={(e) => {
                  e.preventDefault();
                  choose(d);
                }}
              >
                {d.depth === 0 && <Dot color={d.key === "inbox" ? null : d.color} />}
                <span className="qc-opt-label">{filter && d.depth ? d.full : d.label}</span>
                {dest?.key === d.key && <span className="qc-opt-current">obecnie</span>}
              </li>
            ))}
            {shown.length === 0 && <li className="qc-empty">Nic nie pasuje.</li>}
          </ul>
        </div>
      )}
    </div>
  );
}
