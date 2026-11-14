import { TriangleAlert } from "lucide-react";
import { useCallback, useRef, useState } from "react";

import { plural } from "../lib/text";
import { useView } from "../store/vault";
import { useActions } from "./actions";
import { useDismiss } from "./ui";

/** Small footer indicator for core load warnings and reload errors. */
export function WarningsIndicator() {
  const view = useView();
  const { reveal } = useActions();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const close = useCallback(() => setOpen(false), []);
  useDismiss(ref, open, close);

  const warnings = view.snapshot.warnings;
  const n = warnings.length + (view.load_error ? 1 : 0);

  return (
    <div className="warnings" ref={ref}>
      {n > 0 ? (
        <button type="button" className="warn-btn" aria-expanded={open} onClick={() => setOpen(!open)}>
          <TriangleAlert size={12} />
          {n} {plural(n, "ostrzeżenie", "ostrzeżenia", "ostrzeżeń")}
        </button>
      ) : (
        <span className="vault-path" title={view.snapshot.root}>
          {view.snapshot.root.replace(/^\/Users\/[^/]+/, "~")}
        </span>
      )}
      {open && (
        <div className="warn-panel" role="dialog" aria-label="Ostrzeżenia przy wczytywaniu">
          <div className="warn-title">
            Pliki wczytane z problemami
            <span className="vault-path">{view.snapshot.root.replace(/^\/Users\/[^/]+/, "~")}</span>
          </div>
          {view.load_error && <p className="warn-error">Nie udało się wczytać danych: {view.load_error}</p>}
          <ul>
            {warnings.map((w, i) => (
              <li key={i}>
                <button type="button" className="warn-item" title="Pokaż plik w Finderze" onClick={() => void reveal(w.path)}>
                  <span className="warn-loc">
                    {w.path}
                    {w.line != null && <span className="warn-line">:{w.line}</span>}
                  </span>
                  <span className="warn-msg">{w.message}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
