import { t as tr } from "../i18n";
import { askAboutTopic } from "../store/session";
// „Dziś": the map of topics across active projects, not a list of tasks.
// Sections (each hidden when empty): Wymaga uwagi, W toku, Czeka, and the few
// urgent tasks outside topics (collapsed). A topic row expands in place to its
// open tasks. The logic is lib/today.ts; waiting items and drift come from
// store/signals.ts, proposals from store/updates.ts.

import { ChevronRight, MoreHorizontal } from "lucide-react";
import { useEffect, useState } from "react";

import { JournalList } from "../components/Journal";
import { BranchTriage } from "../components/BranchTriage";
import { ProjectName } from "../components/ui";
import { Menu } from "../components/Menu";
import { TaskList } from "../components/TaskList";
import { TopicStatusPill, isWaiting, useCopyContext } from "../components/TopicItem";
import { WaitingStep, openWaitStep, useWaitStepOpen } from "../components/WaitingStep";
import { formatAge, formatLongDate, isOverdue } from "../lib/dates";
import { describeError } from "../lib/errors";
import { tasksAt } from "../lib/group";
import { plural } from "../lib/text";
import { topicTarget } from "../lib/details";
import { useOpenDetail } from "../store/details";
import { buildToday, type ProjectRow, type Reason, type Row, type TaskRow, type TopicRow } from "../lib/today";
import { links, navigate } from "../router/router";
import { driftStore, useDrift, useWaiting, waitingStore } from "../store/signals";
import { updateStore, useUpdates } from "../store/updates";
import { useVault, useView } from "../store/vault";

function Reasons({ reasons }: { reasons: Reason[] }) {
  return (
    <span className="today-reasons">
      {reasons.map((r, i) => (
        <span key={i} className={`today-reason r-${r.kind}`}>
          {r.text}
        </span>
      ))}
    </span>
  );
}

/** "Nadal czeka" and "Zmień powód" for an item whose check is due or that waits. */
function WaitActions({ row, onReason }: { row: TopicRow | TaskRow; onReason: () => void }) {
  const { api, notify } = useVault();
  if (!row.wait) return null;
  const key = row.wait.key;
  return (
    <span className="today-wait-actions">
      {row.wait.due && (
        <button
          type="button"
          className="today-link"
          onClick={async () => {
            try {
              waitingStore.set(await api.confirmWaiting(key));
              notify(tr("Przypomnę znowu za {0}.", row.wait!.interval_days === 1 ? tr("1 dzień") : tr("{0} dni", row.wait!.interval_days)));
            } catch (e) {
              notify(describeError(e));
            }
          }}
        >
          {tr("Nadal czeka")}</button>
      )}
      <button type="button" className="today-link" onClick={onReason}>
        {row.wait.reason ? tr("Zmień powód") : tr("Na co czeka?")}
      </button>
    </span>
  );
}

function TopicRowView({ row, today }: { row: TopicRow; today: string }) {
  const openDetail = useOpenDetail();
  const view = useView();
  const copy = useCopyContext();
  const [open, setOpen] = useState(false);
  const [menu, setMenu] = useState<HTMLElement | null>(null);
  const t = row.topic;
  const waitKey = `topic:${row.project}/${t.slug}`;
  const step = useWaitStepOpen(waitKey);
  const setStep = (on: boolean) => openWaitStep(on ? waitKey : null);
  const loc = { kind: "topic", project: row.project, topic: t.slug } as const;
  const panel = `today-${row.project}-${t.slug}`;
  return (
    <li className={`today-row${open ? " open" : ""}`} data-status={t.status}>
      <div className="today-head">
        <button type="button" className="today-toggle today-expand" aria-label={tr("Zadania tematu {0} ({1})", t.title, row.openTasks)} title={tr("Rozwiń zadania tematu")} aria-expanded={open} aria-controls={panel} onClick={() => setOpen(!open)}>
          <ChevronRight size={14} className="chev" aria-hidden />
        </button>
        <span className="today-kind">{tr("Temat")}</span>
        <button type="button" className="today-title today-title-button" title={tr("Otwórz szczegóły tematu")} onClick={() => openDetail?.(topicTarget(t))}>{t.title}</button>
        <TopicStatusPill project={{ slug: row.project }} topic={t} onChosen={(s) => isWaiting(s) && setStep(true)} />
        <button type="button" className="today-project" onClick={() => openDetail?.({ kind: "project", project: row.project })}>
          <ProjectName name={row.projectName} color={view.snapshot.projects.find((p) => p.slug === row.project)?.color} />
        </button>
        <span className="today-task-count" title={tr("Otwarte zadania w tym temacie")}>{row.openTasks} {plural(row.openTasks, tr("zadanie"), tr("zadania"), tr("zadań"))}</span>
        <span className="today-meta">
          {row.last && (
            <span className="age" title={tr("Ostatni wpis {0} · {1}", row.last.at.replace("T", " "), row.last.author)}>
              {formatAge(row.last.at, today)} · {row.last.author}
            </span>
          )}
        </span>
        <button
          type="button"
          className="icon-btn today-more"
          aria-label={tr("Więcej: {0}", t.title)}
          title={tr("Więcej")}
          aria-haspopup="menu"
          aria-expanded={menu !== null}
          onClick={(e) => setMenu(menu ? null : e.currentTarget)}
        >
          <MoreHorizontal size={14} />
        </button>
        {menu && (
          <Menu
            anchor={menu}
            label={tr("Temat {0}", t.title)}
            minWidth={200}
            onClose={() => setMenu(null)}
            items={[
              { key: "details", label: tr("Szczegóły tematu"), onSelect: () => openDetail?.(topicTarget(t)) },
              { key: "open", label: tr("Otwórz w projekcie"), onSelect: () => navigate(links.project(row.project, t.slug)) },
              { key: "agent", label: tr("Zapytaj agenta…"), onSelect: () => askAboutTopic(t) },
              { key: "copy", label: tr("Kopiuj kontekst"), onSelect: () => void copy(row.project, t) },
              { key: "map", label: tr("Pokaż na mapie"), onSelect: () => navigate(links.map(`${row.project}/${t.slug}`)) },
            ]}
          />
        )}
      </div>
      {row.sentence && <p className="today-sentence">{row.sentence}</p>}
      <div className="today-why">
        <Reasons reasons={row.reasons} />
        <WaitActions row={row} onReason={() => setStep(true)} />
      </div>
      {step && (
        <WaitingStep
          targets={[{ key: waitKey, kind: "topic", status: t.status === "blocked" ? "blocked" : "waiting", project: row.project, topic: t.slug }]}
          onDone={() => setStep(false)}
        />
      )}
      {open && (
        <div className="today-tasks" id={panel}>
          <TaskList listId={`today:${row.project}/${t.slug}`} tasks={tasksAt(view.snapshot, loc).filter((x) => x.status !== "done")} addTo={loc} addPlaceholder="Dodaj zadanie" />
        </div>
      )}
    </li>
  );
}

function ProjectRowView({ row }: { row: ProjectRow }) {
  const view = useView();
  const project = view.snapshot.projects.find((p) => p.slug === row.project);
  const openDetail = useOpenDetail();
  const open = () => openDetail?.({ kind: "project", project: row.project });
  return (
    <li className="today-row today-project-row">
      <div className="today-head">
        <span className="today-kind">{tr("Projekt")}</span>
        <button type="button" className="today-title-link" onClick={open}>
          <ProjectName name={row.projectName} color={project?.color} />
        </button>
        <Reasons reasons={row.reasons} />
        <span className="today-meta">
          <button type="button" className="today-link" onClick={open}>
            {tr("Zobacz szczegóły →")}</button>
        </span>
      </div>
      {row.drift && project && <details className="today-branch-actions"><summary>{tr("Uporządkuj gałęzie — przypisz do tematu lub ignoruj")}</summary><BranchTriage project={project} /></details>}
    </li>
  );
}

function TaskRowView({ row }: { row: TaskRow }) {
  const [step, setStep] = useState(false);
  return (
    <li className="today-row today-task-row">
      <span className="today-kind">{tr("Zadanie poza tematem")}</span>
      <TaskList listId={`today:task:${row.key}`} tasks={[row.task]} showContext />
      <div className="today-why">
        <Reasons reasons={row.reasons} />
        <WaitActions row={row} onReason={() => setStep(true)} />
      </div>
      {step && row.wait && <WaitingStep targets={[{ key: row.wait.key, kind: "task", status: "blocked", task: row.task }]} onDone={() => setStep(false)} />}
    </li>
  );
}

function RowView({ row, today }: { row: Row; today: string }) {
  if (row.kind === "project") return <ProjectRowView row={row} />;
  if (row.kind === "task") return <TaskRowView row={row} />;
  return <TopicRowView row={row} today={today} />;
}

export function TodayView() {
  const view = useView();
  const waiting = useWaiting();
  const drift = useDrift();
  const updates = useUpdates();
  const [showLoose, setShowLoose] = useState<boolean | null>(null);
  const [showAttention, setShowAttention] = useState(true);
  useEffect(() => {
    void waitingStore.start().catch(() => {});
    void driftStore.start().catch(() => {});
    void updateStore.start().catch(() => {});
  }, []);

  const s = buildToday({
    snapshot: view.snapshot,
    today: view.today,
    waiting: waiting.list,
    drift: drift.list,
    updates: Object.values(updates.byProject),
  });
  const urgentLoose = s.loose.filter((t) => isOverdue(t.due, view.today) || t.due?.slice(0, 10) === view.today).length;
  const looseOpen = showLoose ?? urgentLoose > 0;
  const nothing = s.attention.length + s.active.length + s.waiting.length + s.loose.length === 0;

  const section = (id: string, title: string, rows: Row[], hint?: string) =>
    rows.length > 0 && (
      <section className={`group today-group g-${id}`} aria-labelledby={`h-${id}`}>
        <h2 className="group-head" id={`h-${id}`}>
          {title} <span className="count">{rows.length}</span>
          {hint && <span className="group-hint">{hint}</span>}
        </h2>
        <ul className="today-list">
          {rows.map((r) => (
            <RowView key={r.key} row={r} today={view.today} />
          ))}
        </ul>
      </section>
    );

  return (
    <div className="view today">
      <header className="view-head">
        <h1 className="view-title">{tr("Dziś")}</h1>
        <p className="view-sub">{formatLongDate(view.today)}</p>
        {!nothing && (
          <p className="today-count-note">
            {[
              s.attention.length > 0 && `${s.attention.length} ${plural(s.attention.length, tr("wymaga uwagi"), tr("wymagają uwagi"), tr("wymaga uwagi"))}`,
              s.active.length > 0 && `${s.active.length} ${plural(s.active.length, tr("temat w toku"), tr("tematy w toku"), tr("tematów w toku"))}`,
              s.waiting.length > 0 && `${s.waiting.length} ${plural(s.waiting.length, tr("czeka"), tr("czekają"), tr("czeka"))}`,
              view.today_items.length > 0 && `${view.today_items.length} ${plural(view.today_items.length, tr("pilne zadanie"), tr("pilne zadania"), tr("pilnych zadań"))}`,
            ]
              .filter(Boolean)
              .join(" · ")}
            <br />{tr("Licznik Dziś wskazuje pozycje w sekcji „Wymaga uwagi”.")}</p>
        )}
      </header>

      {nothing ? (
        <div className="empty">
          <p className="empty-title">{tr("Nic pilnego.")}</p>
          <p className="empty-text">{tr("Tu trafiają tematy w toku, te, które czekają, i to, co wymaga uwagi — ze wszystkich aktywnych projektów.")}</p>
        </div>
      ) : (
        <>
          {s.attention.length > 0 && <section className="group today-group g-attention" aria-labelledby="h-attention">
            <h2 className="group-head" id="h-attention"><button type="button" className="today-section-toggle" aria-expanded={showAttention} aria-controls="today-attention" onClick={() => setShowAttention(!showAttention)}>
              <ChevronRight size={14} className="chev" aria-hidden />  {tr("Wymaga uwagi")} <span className="count">{s.attention.length}</span>
            </button></h2>
            <p className="today-section-note">{tr("To sygnały do sprawdzenia, niezależnie od statusu. Przy każdej pozycji znajdziesz powód.")}</p>
            {showAttention && <div id="today-attention">
              {(["project", "topic", "task"] as const).map((kind) => {
                const rows = s.attention.filter((r) => r.kind === kind);
                if (!rows.length) return null;
                return <div className="today-attention-group" key={kind}>
                  <h3 className="today-subhead">{kind === "project" ? tr("Projekty do przejrzenia") : kind === "topic" ? tr("Tematy do sprawdzenia") : tr("Zadania do sprawdzenia")} <span className="count">{rows.length}</span></h3>
                  <ul className="today-list">{rows.map((r) => <RowView key={r.key} row={r} today={view.today} />)}</ul>
                </div>;
              })}
            </div>}
          </section>}
          {section("active", tr("Pozostałe tematy w toku"), s.active, tr("bez sygnałów wymagających uwagi"))}
          {section("waiting", tr("Czekające tematy i zadania"), s.waiting, tr("oczekują na odpowiedź lub odblokowanie"))}
          {s.loose.length > 0 && (
            <section className="group today-group g-loose" aria-labelledby="h-loose">
              <h2 className="group-head" id="h-loose">
                <button type="button" className="done-toggle today-loose-toggle" aria-expanded={looseOpen} aria-controls="today-loose" onClick={() => setShowLoose(!looseOpen)}>
                  <ChevronRight size={12} className="chev" aria-hidden />
                  {tr("Zadania poza tematami")} <span className="count">{s.loose.length}</span>{urgentLoose > 0 && <span className="due"> · {urgentLoose}  {tr("po terminie lub na dziś")}</span>}
                </button>
              </h2>
              {looseOpen && (
                <div id="today-loose">
                  <TaskList listId="today:loose" tasks={s.loose} showContext />
                </div>
              )}
            </section>
          )}
        </>
      )}

      <section className="group recent" aria-labelledby="h-recent">
        <h2 className="group-head" id="h-recent">
          {tr("Ostatnie aktualizacje")}</h2>
        {view.recent_journal.length === 0 ? (
          <p className="empty-text">{tr("Wpisy z dzienników tematów — Twoje i agentów — pojawią się tutaj. Dodasz je w widoku projektu.")}</p>
        ) : (
          <JournalList entries={view.recent_journal} showTopic />
        )}
      </section>
    </div>
  );
}
