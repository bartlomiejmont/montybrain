import { t as tr } from "../i18n";
import { useVault } from "../store/vault";

export function Toasts() {
  const { toasts, dismiss } = useVault();
  if (toasts.length === 0) return null;
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className="toast">
          <span>{t.text}</span>
          {t.action && (
            <button
              type="button"
              className="toast-action"
              onClick={() => {
                dismiss(t.id);
                t.action!.run();
              }}
            >
              {t.action.label}
            </button>
          )}
          <button type="button" className="toast-x" aria-label={tr("Zamknij")} onClick={() => dismiss(t.id)}>
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
