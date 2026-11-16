import { t as tr } from "../i18n";
import { TriangleAlert, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { plural } from "../lib/text";
import { explainWarning } from "../lib/warnings";
import { useView } from "../store/vault";
import { useActions } from "./actions";
import { useDismiss } from "./ui";

/** Small footer indicator for core load warnings and reload errors. */
export function WarningsIndicator() {
  const view = useView();
  const { reveal } = useActions();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const btnRef = useRef<HTMLButtonElement>(null);
  const closeBtnRef = useRef<HTMLButtonElement>(null);
  const close = useCallback((restoreFocus = false) => {
    setOpen(false);
    if (restoreFocus) requestAnimationFrame(() => btnRef.current?.focus());
  }, []);
  useDismiss(ref, open, close);

  useEffect(() => {
    if (open) closeBtnRef.current?.focus();
  }, [open]);

  const warnings = view.snapshot.warnings;
  const n = warnings.length + (view.load_error ? 1 : 0);

  return (
    <div className="warnings" ref={ref}>
      {n > 0 ? (
        <button ref={btnRef} type="button" className="btn btn-quiet warn-btn" aria-expanded={open} onClick={() => setOpen(!open)}>
          <TriangleAlert size={12} />
          {n} {plural(n, tr("ostrzeżenie"), tr("ostrzeżenia"), tr("ostrzeżeń"))}
        </button>
      ) : (
        <span className="vault-path" title={view.snapshot.root}>
          {view.snapshot.root.replace(/^\/Users\/[^/]+/, "~")}
        </span>
      )}
      {open && (
        <div
          className="warn-panel"
          role="dialog"
          aria-label={tr("Ostrzeżenia przy wczytywaniu")}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              close(true);
            }
          }}
        >
          <div className="warn-head">
            <div className="warn-title">
              {tr("Pliki wczytane z problemami")}<span className="vault-path">{view.snapshot.root.replace(/^\/Users\/[^/]+/, "~")}</span>
            </div>
            <button ref={closeBtnRef} type="button" className="icon-btn warn-close" aria-label={tr("Zamknij ostrzeżenia")} onClick={() => close(true)}>
              <X size={14} />
            </button>
          </div>
          {view.load_error && <p className="warn-error">{tr("Nie udało się wczytać danych:")} {view.load_error}</p>}
          <ul>
            {warnings.map((w, i) => {
              const info = explainWarning(w.message);
              return (
                <li key={i} className="warn-entry">
                  <button type="button" className="warn-item" title={tr("Pokaż plik w Finderze")} onClick={() => void reveal(w.path)}>
                    <span className="warn-loc">
                      {w.path}
                      {w.line != null && <span className="warn-line">:{w.line}</span>}
                    </span>
                    <span className="warn-msg">{info.summary}</span>
                  </button>
                  <details className="warn-tech">
                    <summary>{tr("Szczegóły techniczne")}</summary>
                    <code>{info.raw}</code>
                  </details>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}
