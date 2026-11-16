import { useLanguage, t as tr } from "./i18n";
import { useEffect } from "react";

import { AgentSessionPanel } from "./components/AgentSession";
import { DetailWorkspace } from "./components/DetailPanel";
import { Palette } from "./components/Palette";
import { Sidebar } from "./components/Sidebar";
import { Toasts } from "./components/Toasts";
import { getView } from "./router/registry";
import { HOME, navigate, useRoute } from "./router/router";
import { useVault } from "./store/vault";

export function App() {
  useLanguage();
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
        <p className="empty-title">{tr("Nie udało się otworzyć danych.")}</p>
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
        <DetailWorkspace key={route.path} enabled={["today", "board", "inbox", "project"].includes(route.view ?? "")} overview={route.view === "inbox" ? { kind: "inbox" } : route.view === "project" ? { kind: "project", project: route.params.project ?? "" } : undefined}>
          <div className="main-scroll">
            <View key={route.path} route={route} />
          </div>
        </DetailWorkspace>
      </main>
      <Toasts />
      <AgentSessionPanel />
      <Palette />
    </div>
  );
}
