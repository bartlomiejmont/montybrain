// The quick-capture window: the command field of Skrzynka (same reading,
// confirm step and echo, read by the backend), a compact destination picker
// for plain capture, Enter saves, Esc closes. The window is created hidden
// once and re-shown by the global shortcut; every show resets it
// (`onQuickShow`).

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import { api, mockCommandState, MOCK_COMMAND_TEXT } from "../api";
import type { LineWritten, Snapshot, TaskLocation } from "../api/types";
import { CommandField, Echo, type CommandFieldHandle } from "../components/CommandField";
import { describeError } from "../lib/errors";
import { describeUndo } from "../lib/undo";
import { destinations, filterDestinations, resolveDestination, type Destination } from "./destinations";

/** Long enough to read the echo and press "Cofnij". */
const CLOSE_AFTER_SAVE_MS = 3500;

function Dot({ color }: { color: string | null }) {
  return <span className="qc-dot" style={{ background: color ?? "var(--text-4)" }} aria-hidden />;
}

export function QuickCapture() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [saved, setSaved] = useState<TaskLocation | null>(null);
  const [picking, setPicking] = useState(false);
  const [filter, setFilter] = useState("");
  const [cursor, setCursor] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const mock = mockCommandState();
  // `?cmd=echo` (browser mock): the saved state with its echo and "Cofnij".
  const [done, setDone] = useState<{ reading: string; echo: LineWritten[]; undo: number | null; undone?: string; failed?: boolean } | null>(
    mock === "echo"
      ? { reading: "Nowe zadanie „Kupić filtry” w: Skrzynka", echo: [{ file: "skrzynka.md", line: 9, text: "- [ ] Kupić filtry ^m3n4b5" }], undo: 1 }
      : null,
  );
  const [generation, setGeneration] = useState(0);
  const fieldRef = useRef<CommandFieldHandle>(null);

  const boxRef = useRef<HTMLDivElement>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const closeTimer = useRef<number | undefined>(undefined);

  const all = useMemo(() => (snap ? destinations(snap) : []), [snap]);
  const dest = all.length ? resolveDestination(all, saved) : null;
  const shown = useMemo(() => filterDestinations(all, filter), [all, filter]);

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
    setPicking(false);
    setFilter("");
    setError(null);
    if (mockCommandState() !== "echo") setDone(null);
    setGeneration((g) => g + 1);
    void load();
    requestAnimationFrame(() => fieldRef.current?.focus());
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
    requestAnimationFrame(() => fieldRef.current?.focus());
  }

  function choose(d: Destination) {
    setSaved(d.location);
    closePicker();
    void api.updateSettings({ quick_destination: d.location }).catch(() => {});
  }

  function onWritten(w: { reading: string; echo: LineWritten[]; undo: number | null }) {
    setDone(w);
    closeTimer.current = window.setTimeout(close, CLOSE_AFTER_SAVE_MS);
  }

  /** "Cofnij" next to the echo: takes back this write; on success says so and closes,
   *  on failure stays open with "Spróbuj cofnąć jeszcze raz". */
  async function undoDone(token: number | null) {
    if (!done) return;
    window.clearTimeout(closeTimer.current);
    try {
      const r = await api.commandUndo(token, "quick");
      const v = describeUndo(r, token);
      setDone({ ...done, undo: v.retry, undone: v.text, failed: v.failed });
      // Only a complete answer closes the window; a failure stays with a retry.
      if (!v.failed) closeTimer.current = window.setTimeout(close, 1200);
    } catch (e) {
      setDone({ ...done, undone: describeError(e), failed: true });
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
        <>
        <div className="qc-row qc-done" role="status">
          <span className="qc-check" aria-hidden>
            <svg viewBox="0 0 16 16" width="16" height="16">
              <circle cx="8" cy="8" r="7.25" fill="currentColor" />
              <path d="M4.8 8.3l2.1 2.1 4.3-4.6" fill="none" stroke="var(--bg)" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </span>
          <span className="qc-done-text">
            Zapisano <span className="qc-done-where">· {done.reading}</span>
          </span>
        </div>
        <div className="qc-echo">
          {done.undone ? (
            <p className="qc-undone" role="status">
              {done.undone}
              {done.failed && done.undo !== null && (
                <>
                  {" "}
                  <button type="button" className="cf-link" onClick={() => void undoDone(done.undo)}>
                    Spróbuj cofnąć jeszcze raz
                  </button>
                </>
              )}
            </p>
          ) : (
            <Echo echo={done.echo} onUndo={done.undo !== null ? () => void undoDone(done.undo) : undefined} />
          )}
        </div>
        </>
      ) : (
        <div className="qc-row qc-field" data-tauri-drag-region onKeyDown={(e) => e.key === "Tab" && !e.shiftKey && !e.defaultPrevented && (e.preventDefault(), openPicker())}>
          <span className="qc-ring" aria-hidden />
          <CommandField
            key={generation}
            ref={fieldRef}
            variant="quick"
            source="quick"
            location={dest?.location ?? null}
            placeholder="Nowe zadanie albo komenda…"
            autoFocus
            initialText={mock && mock !== "topic" && mock !== "agent" ? MOCK_COMMAND_TEXT[mock].text : ""}
            startConfirmed={mock === "confirm"}
            onEscape={close}
            onWritten={onWritten}
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
            <span className="qc-chips" />
          )}
          <span className="qc-hints" aria-hidden>
            <kbd>↵</kbd> zapisz <kbd>⇥</kbd> miejsce <kbd>&gt;</kbd> komenda <kbd>esc</kbd>
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
