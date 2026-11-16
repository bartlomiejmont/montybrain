import { t as tr } from "../i18n";
import { SessionIndicator } from "./AgentSession";
import { ProviderSummary } from "./ProviderConnections";
import { ChevronRight, Columns3, Inbox, Network, Play, Search, Settings2, Sun } from "lucide-react";
import { useEffect, useState, type CSSProperties } from "react";

import { countTasks, partitionProjects } from "../lib/group";
import { PROJECT_COLORS } from "../lib/labels";
import { boardHref } from "../lib/nav";
import { plural } from "../lib/updates";
import type { Route } from "../router/registry";
import { links, navigate } from "../router/router";
import { useRuns } from "../store/runs";
import { buildToday, attentionCount } from "../lib/today";
import { driftStore, useDrift, useWaiting, waitingStore } from "../store/signals";
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
  color,
  pending,
  countLabel,
  countSuffix,
  countTone,
}: {
  href: string;
  active: boolean;
  icon: React.ReactNode;
  label: string;
  count?: number;
  dim?: boolean;
  color?: string | null;
  /** A subtle marker with this accessible text (e.g. proposals waiting). */
  mark?: string;
  pending?: number;
  countLabel?: string;
  countSuffix?: string;
  countTone?: "running";
}) {
  const defaultTitle = count ? plural(count, tr("otwarte zadanie"), tr("otwarte zadania"), tr("otwartych zadań")) : undefined;
  return (
    <a href={href} className={`nav-item${active ? " active" : ""}${dim ? " dim" : ""}${color ? " project-nav" : ""}`} style={color ? { "--project-color": color } as CSSProperties : undefined} aria-current={active ? "page" : undefined}>
      <span className="nav-icon">{icon}</span>
      <span className="nav-label">{label}</span>
      {mark && (
        <span className="nav-mark" title={mark} aria-label={mark}>
          {pending} <span aria-hidden>{tr("zm.")}</span>
        </span>
      )}
      {count ? (
        <span
          className={`nav-count${countTone ? ` ${countTone}` : ""}`}
          title={countLabel ?? defaultTitle}
          aria-label={countLabel ?? defaultTitle}
        >
          {count}
          {countSuffix ?? ""}
        </span>
      ) : null}
    </a>
  );
}

function pendingMark(n: number): string | undefined {
  if (n <= 0) return undefined;
  return plural(n, tr("propozycja zmian czeka"), tr("propozycje zmian czekają"), tr("propozycji zmian czeka"));
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
  const drift = useDrift();
  const waiting = useWaiting();
  const attention = attentionCount(buildToday({ snapshot: view.snapshot, today: view.today, waiting: waiting.list, drift: drift.list, updates: Object.values(updates.byProject) }));
  useEffect(() => {
    void updateStore.start().catch(() => {});
    void driftStore.start().catch(() => {});
    void waitingStore.start().catch(() => {});
  }, []);

  const projectItem = (p: (typeof visible)[number]) => (
    <NavItem
      key={p.slug}
      href={links.project(p.slug)}
      active={activeProject === p.slug}
      icon={<ProjectDot color={p.color} />}
      label={p.name}
      color={p.color}
      count={counts.project.get(p.slug)?.open}
      dim={p.state !== "active"}
      mark={pendingMark(updates.byProject[p.slug]?.pending ?? 0)}
      pending={updates.byProject[p.slug]?.pending}
    />
  );

  return (
    <nav className="sidebar" aria-label={tr("Nawigacja")}>
      <div className="drag-strip" data-tauri-drag-region />
      <div className="nav-scroll">
        <button type="button" className="nav-item nav-search" onClick={() => window.dispatchEvent(new Event("montybrain:search"))}>
          <span className="nav-icon"><Search size={14} /></span>
          <span className="nav-label">{tr("Szukaj")}</span><kbd>{/Mac/.test(navigator.platform) ? "⌘K" : "Ctrl K"}</kbd>
        </button>
        <NavItem
          href={links.today()}
          active={route.view === "today"}
          icon={<Sun size={14} />}
          label={tr("Dziś")}
          count={attention}
          countLabel={plural(attention, tr("sprawa wymagająca uwagi"), tr("sprawy wymagające uwagi"), tr("spraw wymagających uwagi"))}
        />
        <NavItem
          href={links.inbox()}
          active={route.view === "inbox"}
          icon={<Inbox size={14} />}
          label={tr("Skrzynka")}
          count={counts.inbox.open}
        />
        <NavItem href={boardHref()} active={route.view === "board"} icon={<Columns3 size={14} />} label={tr("Tablica")} />
        <NavItem href={links.map()} active={route.view === "map"} icon={<Network size={14} />} label={tr("Mapa")} />
        <NavItem
          href="#/akcje"
          active={route.view === "actions"}
          icon={<Play size={14} />}
          label={tr("Akcje")}
          count={runningActions}
          countTone={runningActions > 0 ? "running" : undefined}
          countLabel={runningActions > 0 ? plural(runningActions, tr("akcja w toku"), tr("akcje w toku"), tr("akcji w toku")) : undefined}
        />

        <div className="nav-section">{tr("Projekty")}</div>
        {visible.map(projectItem)}
        {visible.length === 0 && <p className="nav-empty">{tr("Nie masz jeszcze projektów.")}</p>}

        {adding ? (
          <div className="nav-add">
            <SubmitInput
              autoFocus
              draftKey="new-project"
              className="field-inline"
              placeholder={tr("Nazwa projektu")}
              aria-label={tr("Nazwa nowego projektu")}
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
            <span className="nav-label">{tr("Projekt")}</span>
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
              {tr("Archiwum")} <span className="count">{archived.length}</span>
            </button>
            {showArchived && archived.map(projectItem)}
          </>
        )}
      </div>
      <div className="nav-settings"><NavItem href="#/ustawienia" active={route.view === "settings"} icon={<Settings2 size={14} />} label={tr("Ustawienia")} /></div>
      <div className="sidebar-foot">
        <ProviderSummary />
        <SessionIndicator />
        <WarningsIndicator />
      </div>
    </nav>
  );
}
