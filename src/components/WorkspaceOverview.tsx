import { t as tr } from "../i18n";
import { taskTarget, topicTarget } from "../lib/details";
import { compareTasks } from "../lib/board";
import { plural } from "../lib/text";
import { useOpenDetail } from "../store/details";
import { useView } from "../store/vault";
import { JournalList } from "./Journal";
import { ProjectName } from "./ui";
import { TaskList } from "./TaskList";

export type WorkspaceOverviewKind = { kind: "inbox" } | { kind: "project"; project: string };

/** Useful context before a task is selected; selected details replace this pane. */
export function WorkspaceOverview({ context }: { context: WorkspaceOverviewKind }) {
  const view = useView();
  const openDetail = useOpenDetail();
  const snap = view.snapshot;
  if (context.kind === "inbox") {
    const tasks = snap.tasks.filter((t) => t.location.kind === "inbox" && t.status !== "done");
    const topics = snap.topics.filter((t) => t.status !== "done" && snap.projects.some((p) => p.slug === t.project && p.state === "active")).sort((a, b) => (b.updated ?? "").localeCompare(a.updated ?? "")).slice(0, 5);
    return <aside className="detail-panel workspace-overview" aria-label={tr("Jak korzystać ze skrzynki")}>
      <span className="detail-kind">{tr("Skrzynka")}</span>
      <h2 className="detail-panel-title">{tr("Zapisz teraz. Uporządkuj później.")}</h2>
      <p className="overview-intro">{tasks.length ? tr("{0} {1} na przypisanie do projektu lub tematu.", tasks.length, plural(tasks.length, tr("wpis czeka"), tr("wpisy czekają"), tr("wpisów czeka"))) : tr("Masz miejsce na luźne myśli, pomysły i zadania, które jeszcze nie mają swojego projektu.")}</p>
      <ol className="overview-steps">
        <li><strong>{tr("Zapisz myśl")}</strong><span>{tr("Wpisz zwykłe zdanie w polu po lewej i naciśnij Enter.")}</span></li>
        <li><strong>{tr("Otwórz wpis")}</strong><span>{tr("Kliknij jego tytuł. Tutaj zobaczysz opis, status i termin.")}</span></li>
        <li><strong>{tr("Znajdź mu miejsce")}</strong><span>{tr("Użyj „Przenieś do…” w szczegółach albo przyjmij podpowiedź obok wpisu.")}</span></li>
      </ol>
      {tasks.length > 0 && <button type="button" className="overview-action" onClick={() => openDetail?.(taskTarget(tasks[0]!))}>{tr("Otwórz pierwszy wpis →")}</button>}
      {topics.length > 0 && <>
        <h3 className="detail-section-title">{tr("Ostatnio aktualizowane tematy")}</h3>
        <ul className="overview-topics">{topics.map((t) => <li key={`${t.project}/${t.slug}`}><button type="button" className="detail-link" onClick={() => openDetail?.(topicTarget(t))}>{t.title}</button><ProjectName name={snap.projects.find((p) => p.slug === t.project)?.name ?? t.project} color={snap.projects.find((p) => p.slug === t.project)?.color} /></li>)}</ul>
      </>}
    </aside>;
  }
  const project = snap.projects.find((p) => p.slug === context.project);
  if (!project) return null;
  const tasks = snap.tasks.filter((t) => t.location.kind !== "inbox" && t.location.project === project.slug && t.status !== "done").sort((a, b) => Number(b.status === "doing") - Number(a.status === "doing") || compareTasks(a, b));
  const entries = snap.journal.filter((j) => j.project === project.slug).sort((a, b) => b.at.localeCompare(a.at)).slice(0, 3);
  return <aside className="detail-panel workspace-overview" style={project.color ? { "--accent": project.color } as React.CSSProperties : undefined} data-project-color={project.color ?? undefined} aria-label={tr("Bieżąca praca: {0}", project.name)}>
    <div className="detail-kind">{tr("Bieżąca praca ·")} <ProjectName name={project.name} color={project.color} /></div>
    <h2 className="detail-panel-title">{tr("Najbliższe zadania")}</h2>
    <p className="overview-intro">{tr("Najpierw zadania w toku, potem priorytet i termin. Kliknij zadanie, żeby otworzyć szczegóły w tym panelu.")}</p>
    <TaskList listId={`overview:${project.slug}`} tasks={tasks.slice(0, 5)} empty={<p className="empty-text">{tr("Brak otwartych zadań. Nowe zadanie możesz dodać w temacie lub na dole projektu.")}</p>} />
    {tasks.length > 5 && <p className="overview-note">{tr("Pozostałe")} {tasks.length - 5} {plural(tasks.length - 5, tr("zadanie znajdziesz"), tr("zadania znajdziesz"), tr("zadań znajdziesz"))}  {tr("w tematach projektu.")}</p>}
    <h3 className="detail-section-title">{tr("Ostatnie aktualizacje")}</h3>
    {entries.length ? <JournalList entries={entries} showTopic /> : <p className="empty-text">{tr("Wpisy z dzienników tematów pojawią się tutaj.")}</p>}
  </aside>;
}
