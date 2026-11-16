import { t as tr } from "../i18n";
import { lockInert } from "../lib/inert";
import { ArrowLeft, ExternalLink, X } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";

import type { Task, Topic } from "../api";
import { resolveDetail, topicTarget, type DetailTarget } from "../lib/details";
import { journalOf, tasksAt } from "../lib/group";
import { taskHref, topicHref } from "../lib/nav";
import { links, navigate, useRoute } from "../router/router";
import { DetailContext } from "../store/details";
import { useDrift } from "../store/signals";
import { useUpdates } from "../store/updates";
import { useVault, useView } from "../store/vault";
import { useCopyContext } from "./TopicItem";
import { askAboutTopic } from "../store/session";
import { Review } from "../views/project/UpdateStrip";
import { useJournalAgent } from "./JournalAgent";
import { JournalForm, JournalList } from "./Journal";
import { BranchTriage } from "./BranchTriage";
import { Menu } from "./Menu";
import { TaskDetail } from "./TaskDetail";
import { TaskList } from "./TaskList";
import { TopicSummary } from "./TopicSummary";
import { WorkspaceOverview, type WorkspaceOverviewKind } from "./WorkspaceOverview";
import { useMoveItems } from "./TaskRow";
import { TopicEditor, TopicStatusPill, isWaiting } from "./TopicItem";
import { WaitingStep, openWaitStep, useWaitStepOpen } from "./WaitingStep";
import { Button, ConfirmButton, ProjectName } from "./ui";
import "./details.css";

export function DetailWorkspace({ enabled, overview, children }: { enabled: boolean; overview?: WorkspaceOverviewKind; children: ReactNode }) {
  const [target, setTarget] = useState<DetailTarget | null>(null);
  const [overlay, setOverlay] = useState(false);
  const workspace = useRef<HTMLDivElement>(null);
  const origin = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (!workspace.current) return;
    const observer = new ResizeObserver(([entry]) => setOverlay(entry!.contentRect.width < 1000));
    observer.observe(workspace.current);
    return () => observer.disconnect();
  }, [enabled]);
  useEffect(() => {
    if (!target || !overlay) return;
    const elements = [workspace.current?.querySelector<HTMLElement>(".main-scroll"), document.querySelector<HTMLElement>(".sidebar")].filter((el): el is HTMLElement => !!el);
    return lockInert(elements);
  }, [target, overlay]);
  const open = (next: DetailTarget) => {
    if (!target) origin.current = document.activeElement as HTMLElement | null;
    setTarget(next);
  };
  const close = () => {
    setTarget(null);
    if (origin.current?.isConnected) origin.current.focus({ preventScroll: true });
  };
  if (!enabled) return children;
  return (
    <DetailContext.Provider value={open}>
      <div ref={workspace} className={`detail-workspace${target ? " has-detail" : overview ? " has-overview" : ""}`}>
        {children}
        {target && <div className="detail-backdrop" aria-hidden onClick={close} />}
        {target ? <DetailPanel target={target} overlay={overlay} onSelect={open} onClose={close} /> : overview && <WorkspaceOverview context={overview} />}
      </div>
    </DetailContext.Provider>
  );
}

export function DetailPanel({ target, onSelect, onClose, className = "", children, summaryText, overlay = false }: {
  target: DetailTarget;
  onSelect: (target: DetailTarget) => void;
  onClose: () => void;
  className?: string;
  children?: ReactNode;
  summaryText?: string;
  overlay?: boolean;
}) {
  const view = useView();
  const route = useRoute();
  const { task, topic, project, exists } = resolveDetail(view.snapshot, target);
  const heading = useRef<HTMLHeadingElement>(null);
  const selectionKey = JSON.stringify(target);
  useEffect(() => {
    heading.current?.closest("aside")?.querySelector(".detail-panel-body")?.scrollTo({ top: 0 });
    heading.current?.focus({ preventScroll: true });
  }, [selectionKey]);
  const title = target.kind === "task" ? task?.title : target.kind === "topic" ? topic?.title : project?.name;
  const kind = target.kind === "task" ? tr("Zadanie") : target.kind === "topic" ? tr("Temat") : tr("Projekt");
  const href = task ? taskHref(task) : topic ? topicHref(topic) : project ? links.project(project.slug) : null;
  const atContainer = (route.view === "project" && route.params.project === project?.slug) || (route.view === "inbox" && task?.location.kind === "inbox");
  return (
    <DetailContext.Provider value={onSelect}>
      <aside role={overlay ? "dialog" : undefined} aria-modal={overlay || undefined} style={project?.color ? { "--accent": project.color } as React.CSSProperties : undefined} data-project-color={project?.color ?? undefined} className={`detail-panel ${className}`} aria-label={tr("Szczegóły: {0}", title ?? kind)} onKeyDownCapture={(e) => {
        if (overlay && e.key === "Tab") {
          const focusable = [...e.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), a[href], input:not(:disabled), textarea:not(:disabled), select:not(:disabled), [tabindex="0"]')].filter((el) => el.getClientRects().length > 0);
          const first = focusable[0];
          const last = focusable.at(-1);
          if (e.shiftKey && (document.activeElement === first || document.activeElement === heading.current)) { e.preventDefault(); last?.focus(); }
          else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
        }
        if (e.key === "Escape" && !(e.target as HTMLElement).closest("input, textarea, select, [role='menu'], .wait-step, .map-connect, .confirm, .editor, .branch-new")) {
          e.stopPropagation();
          onClose();
        }
      }}>
        <div className="detail-panel-head">
          <div>
            <span className="detail-kind">{kind}</span>
            <h2 ref={heading} tabIndex={-1} className="detail-panel-title">{target.kind === "project" && project ? <ProjectName name={project.name} color={project.color} /> : title ?? tr("Element niedostępny")}</h2>
          </div>
          <button type="button" className="icon-btn" aria-label={tr("Zamknij panel szczegółów")} onClick={onClose}><X size={16} /></button>
        </div>
        <div className="detail-panel-body">
        {!exists ? <p className="empty-text">{tr("Ten element został usunięty lub przeniesiony. Wybierz inny z listy.")}</p> : (
          <>
            <div className="detail-breadcrumb">
              {project ? <ProjectName name={project.name} color={project.color} /> : <span>{tr("Skrzynka")}</span>}
              {target.kind === "task" && topic && <button type="button" className="detail-link" onClick={() => onSelect(topicTarget(topic))}><ArrowLeft size={12} /> {tr("Temat:")} {topic.title}</button>}
              {target.kind === "task" && !topic && <span>{tr("· zadanie poza tematem")}</span>}
            </div>
            {target.kind === "task" && <p className="detail-save-note">{tr("Zmiany w polach zapisują się po ich opuszczeniu.")}</p>}
            {target.kind === "task" && task && <PanelTask key={selectionKey} task={task} />}
            {target.kind === "topic" && topic && <PanelTopic key={selectionKey} topic={topic} summaryText={summaryText} />}
            {target.kind === "project" && project && <PanelProject project={project} onSelect={onSelect} />}
            {children}
            {href && !atContainer && <div className="detail-panel-footer"><Button variant="quiet" onClick={() => navigate(href)}><ExternalLink size={12} /> {task?.location.kind === "inbox" ? tr("Otwórz w skrzynce") : tr("Otwórz w projekcie")}</Button></div>}
          </>
        )}
        </div>
      </aside>
    </DetailContext.Provider>
  );
}

function PanelTask({ task }: { task: Task }) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const moveItems = useMoveItems(task);
  return <>
    <TaskDetail task={task} askDelete={false} onAskDeleteDone={() => {}} onMove={setAnchor} />
    {anchor && <Menu anchor={anchor} label={tr("Przenieś zadanie do")} items={moveItems} onClose={() => setAnchor(null)} minWidth={260} />}
  </>;
}

function PanelTopic({ topic, summaryText }: { topic: Topic; summaryText?: string }) {
  const view = useView();
  const { run } = useVault();
  const [editing, setEditing] = useState(false);
  const copy = useCopyContext();
  const project = view.snapshot.projects.find((p) => p.slug === topic.project);
  const waitKey = `topic:${topic.project}/${topic.slug}`;
  const step = useWaitStepOpen(waitKey);
  const tasks = tasksAt(view.snapshot, { kind: "topic", project: topic.project, topic: topic.slug });
  const entries = journalOf(view.snapshot, topic.project, topic.slug);
  const agent = useJournalAgent(topic.project, topic.slug, entries);
  return <>
    {editing && project ? <TopicEditor project={project} topic={topic} onDone={(status) => {
      setEditing(false);
      if (status && isWaiting(status)) openWaitStep(waitKey);
    }} /> : <>
      <div className="detail-topic-status"><span className="d-label">{tr("Status tematu")}</span><TopicStatusPill project={{ slug: topic.project }} topic={topic} onChosen={(s) => isWaiting(s) && openWaitStep(waitKey)} /></div>
      <div className="detail-description-head"><h3>{tr("Opis tematu")}</h3><Button variant="quiet" onClick={() => setEditing(true)}>{tr("Edytuj temat")}</Button></div>
      <TopicSummary text={summaryText ?? topic.summary} />
    </>}
    {step && <WaitingStep targets={[{ key: waitKey, kind: "topic", status: topic.status === "blocked" ? "blocked" : "waiting", project: topic.project, topic: topic.slug }]} onDone={() => openWaitStep(null)} />}
    <h3 className="detail-section-title">{tr("Zadania w tym temacie")} <span className="count">{tr("Otwarte:")} {tasks.filter((t) => t.status !== "done").length}</span></h3>
    <TaskList listId={`detail:${topic.project}/${topic.slug}`} tasks={tasks} addTo={{ kind: "topic", project: topic.project, topic: topic.slug }} empty={<p className="empty-text">{tr("Brak otwartych zadań w tym temacie.")}</p>} />
    <div className="j-section-head"><h3 className="detail-section-title">{tr("Dziennik tematu")}</h3>{agent.controls}</div>
    <JournalForm project={topic.project} topic={topic.slug} status={topic.status} />
    {agent.result}
    <JournalList entries={entries.slice(0, 5)} />
    <div className="detail-topic-actions">
      <Button variant="quiet" onClick={() => void copy(topic.project, topic)}>{tr("Kopiuj kontekst")}</Button>
      <Button variant="quiet" onClick={() => askAboutTopic(topic)}>{tr("Zapytaj agenta…")}</Button>
      <Button variant="quiet" onClick={() => void run((a) => a.revealPath(topic.path))}>{tr("Pokaż plik")}</Button>
      <ConfirmButton question={tr("Usunąć temat razem z zadaniami i dziennikiem?")} confirmLabel={tr("Usuń temat")} onConfirm={() => void run((a) => a.deleteTopic(topic.project, topic.slug))}>{tr("Usuń temat")}</ConfirmButton>
    </div>
  </>;
}

function PanelProject({ project, onSelect }: { project: NonNullable<ReturnType<typeof resolveDetail>["project"]>; onSelect: (target: DetailTarget) => void }) {
  const view = useView();
  const { refresh } = useVault();
  const drift = useDrift().list.find((d) => d.project === project.slug);
  const update = useUpdates().byProject[project.slug];
  const topics = view.snapshot.topics.filter((t) => t.project === project.slug && t.status !== "done");
  return <>
    {project.description && <p className="detail-prose">{project.description}</p>}
    {!!drift?.unknown.length && <>
      <h3 className="detail-section-title">{tr("Gałęzie bez przypisanego tematu")} <span className="count">{drift.unknown.length}</span></h3>
      <BranchTriage project={project} />
    </>}
    {!!update?.pending && update.proposal && <>
      <h3 className="detail-section-title">{tr("Propozycje zmian")} <span className="count">{update.pending}</span></h3>
      <p className="empty-text">{tr("Agent przygotował zmiany do przejrzenia i zaakceptowania.")}</p>
      <Review key={update.proposal.run_id} project={project.slug} proposal={update.proposal} autoFocus={false} onApplied={() => void refresh()} onDecided={() => void refresh()} onUndone={() => void refresh()} />
    </>}
    <h3 className="detail-section-title">{tr("Tematy projektu")} <span className="count">{topics.length}</span></h3>
    <ul className="detail-topic-list">{topics.map((t) => <li key={t.slug}><button type="button" className="detail-link" onClick={() => onSelect(topicTarget(t))}>{t.title}</button><TopicStatusPill project={project} topic={t} /></li>)}</ul>
  </>;
}
