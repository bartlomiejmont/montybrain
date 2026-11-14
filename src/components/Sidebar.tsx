import { ChevronRight, Columns3, Inbox, Network, Play, Settings2, Sun } from "lucide-react";
import { useEffect, useState } from "react";

import { countTasks, partitionProjects } from "../lib/group";
import { PROJECT_COLORS } from "../lib/labels";
import { boardHref } from "../lib/nav";
import { plural } from "../lib/updates";
import type { Route } from "../router/registry";
import { links, navigate } from "../router/router";
import { useRuns } from "../store/runs";
import { updateStore, useUpdates } from "../store/updates";
import { useVault, useView } from "../store/vault";
import { ProjectDot, SubmitInput, useStoredFlag } from "./ui";
import { WarningsIndicator } from "./Warnings";

function NavItem({
  href,
  active,
  icon,
  label,
  count,
  dim,
  mark,
}: {
  href: string;
  active: boolean;
  icon: React.ReactNode;
  label: string;
  count?: number;
  dim?: boolean;
  /** A subtle marker with this accessible text (e.g. proposals waiting). */
  mark?: string;
}) {
  return (
    <a href={href} className={`nav-item${active ? " active" : ""}${dim ? " dim" : ""}`} aria-current={active ? "page" : undefined}>
      <span className="nav-icon">{icon}</span>
      <span className="nav-label">{label}</span>
      {mark && (
        <span className="nav-mark" title={mark}>
          <span className="sr-only">{mark}</span>
        </span>
      )}
      {count ? <span className="nav-count">{count}</span> : null}
    </a>
  );
}

function pendingMark(n: number): string | undefined {
  if (n <= 0) return undefined;
  return plural(n, "propozycja zmian czeka", "propozycje zmian czekają", "propozycji zmian czeka");
}

export function Sidebar({ route }: { route: Route }) {
  const view = useView();
  const { run } = useVault();
  const counts = countTasks(view.snapshot);
  const runningActions = useRuns().list.filter((r) => r.mode === "background" && r.status === "running").length;
  const { visible, archived } = partitionProjects(view.snapshot.projects);
  const [showArchived, setShowArchived] = useStoredFlag("montybrain.sidebar.archived", false);
  const [adding, setAdding] = useState(false);
  const activeProject = route.view === "project" ? route.params.project : null;
  const updates = useUpdates();
  useEffect(() => {
    void updateStore.start().catch(() => {});
  }, []);

  const projectItem = (p: (typeof visible)[number]) => (
    <NavItem
      key={p.slug}
      href={links.project(p.slug)}
      active={activeProject === p.slug}
      icon={<ProjectDot color={p.color} />}
      label={p.name}
      count={counts.project.get(p.slug)?.open}
      dim={p.state !== "active"}
      mark={pendingMark(updates.byProject[p.slug]?.pending ?? 0)}
    />
  );

  return (
    <nav className="sidebar" aria-label="Nawigacja">
      <div className="drag-strip" data-tauri-drag-region />
      <div className="nav-scroll">
        <NavItem
          href={links.today()}
          active={route.view === "today"}
          icon={<Sun size={14} />}
          label="Dziś"
          count={view.today_items.length}
        />
        <NavItem
          href={links.inbox()}
          active={route.view === "inbox"}
          icon={<Inbox size={14} />}
          label="Skrzynka"
          count={counts.inbox.open}
        />
        <NavItem href={boardHref()} active={route.view === "board"} icon={<Columns3 size={14} />} label="Tablica" />
        <NavItem href={links.map()} active={route.view === "map"} icon={<Network size={14} />} label="Mapa" />
        <NavItem href="#/akcje" active={route.view === "actions"} icon={<Play size={14} />} label="Akcje" count={runningActions} />

        <div className="nav-section">Projekty</div>
        {visible.map(projectItem)}
        {visible.length === 0 && <p className="nav-empty">Nie masz jeszcze projektów.</p>}

        {adding ? (
          <div className="nav-add">
            <SubmitInput
              autoFocus
              draftKey="new-project"
              className="field-inline"
              placeholder="Nazwa projektu"
              aria-label="Nazwa nowego projektu"
              onEscape={() => setAdding(false)}
              onBlur={() => setAdding(false)}
              onSubmit={async (name) => {
                const color = PROJECT_COLORS[(view.snapshot.projects.length * 3) % PROJECT_COLORS.length] ?? null;
                const p = await run((a) => a.createProject({ name, color }));
                if (!p) return false;
                setAdding(false);
                navigate(links.project(p.slug));
              }}
            />
          </div>
        ) : (
          <button type="button" className="nav-item nav-new" onClick={() => setAdding(true)}>
            <span className="nav-icon">+</span>
            <span className="nav-label">Projekt</span>
          </button>
        )}

        {archived.length > 0 && (
          <>
            <button
              type="button"
              className="nav-archive-toggle"
              aria-expanded={showArchived}
              onClick={() => setShowArchived(!showArchived)}
            >
              <ChevronRight size={11} className="chev" />
              Archiwum <span className="count">{archived.length}</span>
            </button>
            {showArchived && archived.map(projectItem)}
          </>
        )}
      </div>
      <div className="nav-settings"><NavItem href="#/ustawienia" active={route.view === "settings"} icon={<Settings2 size={14} />} label="Ustawienia" /></div>
      <div className="sidebar-foot">
        <WarningsIndicator />
      </div>
    </nav>
  );
}
