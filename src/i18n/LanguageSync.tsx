import { useEffect } from "react";

import { api } from "../api";
import { setLanguage } from ".";

/** Both desktop windows follow the same persisted native setting. */
export function LanguageSync() {
  useEffect(() => {
    let disposed = false;
    let off: (() => void) | undefined;
    let received = false;
    void api.onSettingsChange((settings) => {
      received = true;
      if (!disposed) setLanguage(settings.language);
    }).then((unsubscribe) => { if (disposed) unsubscribe(); else off = unsubscribe; });
    void api.getSettings().then((settings) => {
      if (!disposed && !received) setLanguage(settings.language);
    }).catch(() => {});
    return () => { disposed = true; off?.(); };
  }, []);
  return null;
}
