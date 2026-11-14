import { useEffect } from "react";

import { AgentSessionPanel } from "./components/AgentSession";
import { Palette } from "./components/Palette";
import { Sidebar } from "./components/Sidebar";
import { Toasts } from "./components/Toasts";
import { getView } from "./router/registry";
import { HOME, navigate, useRoute } from "./router/router";
import { useVault } from "./store/vault";

export function App() {
  const { view, fatal, api } = useVault();
  const route = useRoute();
  const def = getView(route.view);

  useEffect(() => {
    if (!def) navigate(HOME);
  }, [def]);

  useEffect(() => {
    document.documentElement.dataset.mode = api.mode;
  }, [api.mode]);

  if (fatal) {
    return (
      <div className="boot">
        <div className="drag-strip" data-tauri-drag-region />
        <p className="empty-title">Nie udało się otworzyć danych.</p>
        <p className="empty-text">{fatal}</p>
      </div>
    );
  }
  if (!view || !def) {
    return (
      <div className="boot">
        <div className="drag-strip" data-tauri-drag-region />
      </div>
    );
  }

  const View = def.component;
  return (
    <div className="app">
      <Sidebar route={route} />
      <main className="main">
        <div className="drag-strip" data-tauri-drag-region />
        <div className="main-scroll">
          <View key={route.path} route={route} />
        </div>
      </main>
      <Toasts />
      <AgentSessionPanel />
      <Palette />
    </div>
  );
}
