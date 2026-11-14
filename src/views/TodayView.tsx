// „Dziś": the map of topics across active projects, not a list of tasks.
// Sections (each hidden when empty): Wymaga uwagi, W toku, Czeka, and the few
// urgent tasks outside topics (collapsed). A topic row expands in place to its
// open tasks. The logic is lib/today.ts; waiting items and drift come from
// store/signals.ts, proposals from store/updates.ts.

import { ChevronRight, MoreHorizontal } from "lucide-react";
import { useEffect, useState } from "react";

import { JournalList } from "../components/Journal";
import { Menu } from "../components/Menu";
import { TaskList } from "../components/TaskList";
import { TopicStatusPill, isWaiting, useCopyContext } from "../components/TopicItem";
import { WaitingStep, openWaitStep, useWaitStepOpen } from "../components/WaitingStep";
import { formatAge, formatLongDate } from "../lib/dates";
import { describeError } from "../lib/errors";
import { tasksAt } from "../lib/group";
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
              notify(`Przypomnę znowu za ${row.wait!.interval_days === 1 ? "1 dzień" : `${row.wait!.interval_days} dni`}.`);
            } catch (e) {
              notify(describeError(e));
            }
          }}
        >
          Nadal czeka
        </button>
      )}
      <button type="button" className="today-link" onClick={onReason}>
        {row.wait.reason ? "Zmień powód" : "Na co czeka?"}
      </button>
    </span>
  );
}

function TopicRowView({ row, today }: { row: TopicRow; today: string }) {
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
        <button type="button" className="today-toggle" aria-expanded={open} aria-controls={panel} onClick={() => setOpen(!open)}>
          <ChevronRight size={14} className="chev" aria-hidden />
          <span className="today-title" title={t.title}>
            {t.title}
          </span>
        </button>
        <TopicStatusPill project={{ slug: row.project }} topic={t} onChosen={(s) => isWaiting(s) && setStep(true)} />
        <a className="today-project" href={links.project(row.project, t.slug)}>
          {row.projectName}
        </a>
        <span className="today-meta">
          {row.last && (
            <span className="age" title={`Ostatni wpis ${row.last.at.replace("T", " ")} · ${row.last.author}`}>
              {formatAge(row.last.at, today)} · {row.last.author}
            </span>
          )}
        </span>
        <button
          type="button"
          className="icon-btn today-more"
          aria-label={`Więcej: ${t.title}`}
          title="Więcej"
          aria-haspopup="menu"
          aria-expanded={menu !== null}
          onClick={(e) => setMenu(menu ? null : e.currentTarget)}
        >
          <MoreHorizontal size={14} />
        </button>
        {menu && (
          <Menu
            anchor={menu}
            label={`Temat ${t.title}`}
            minWidth={200}
            onClose={() => setMenu(null)}
            items={[
              { key: "open", label: "Otwórz w projekcie", onSelect: () => navigate(links.project(row.project, t.slug)) },
              { key: "copy", label: "Kopiuj kontekst", onSelect: () => void copy(row.project, t) },
              { key: "map", label: "Pokaż na mapie", onSelect: () => navigate(links.map(`${row.project}/${t.slug}`)) },
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
  return (
    <li className="today-row today-project-row">
      <div className="today-head">
        <a className="today-title-link" href={`${links.project(row.project)}${row.drift && !row.pending ? "?drift=1" : ""}`}>
          {row.projectName}
        </a>
        <Reasons reasons={row.reasons} />
        <span className="today-meta">
          <a className="today-link" href={`${links.project(row.project)}${row.drift && !row.pending ? "?drift=1" : ""}`}>
            {row.pending ? "Przejrzyj propozycje →" : "Zobacz rozjazd →"}
          </a>
        </span>
      </div>
    </li>
  );
}

function TaskRowView({ row }: { row: TaskRow }) {
  const [step, setStep] = useState(false);
  return (
    <li className="today-row today-task-row">
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
  const [showLoose, setShowLoose] = useState(false);
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
        <h1 className="view-title">Dziś</h1>
        <p className="view-sub">{formatLongDate(view.today)}</p>
      </header>

      {nothing ? (
        <div className="empty">
          <p className="empty-title">Nic pilnego.</p>
          <p className="empty-text">Tu trafiają tematy w toku, te, które czekają, i to, co wymaga uwagi — ze wszystkich aktywnych projektów.</p>
        </div>
      ) : (
        <>
          {section("attention", "Wymaga uwagi", s.attention)}
          {section("active", "W toku", s.active)}
          {section("waiting", "Czeka", s.waiting, "przypomnę, gdy przyjdzie czas sprawdzić")}
          {s.loose.length > 0 && (
            <section className="group today-group g-loose" aria-labelledby="h-loose">
              <h2 className="group-head" id="h-loose">
                <button type="button" className="done-toggle today-loose-toggle" aria-expanded={showLoose} aria-controls="today-loose" onClick={() => setShowLoose(!showLoose)}>
                  <ChevronRight size={12} className="chev" aria-hidden />
                  Zadania poza tematami <span className="count">{s.loose.length}</span>
                </button>
              </h2>
              {showLoose && (
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
          Ostatnie aktualizacje
        </h2>
        {view.recent_journal.length === 0 ? (
          <p className="empty-text">Wpisy z dzienników tematów — Twoje i agentów — pojawią się tutaj. Dodasz je w widoku projektu.</p>
        ) : (
          <JournalList entries={view.recent_journal} showTopic />
        )}
      </section>
    </div>
  );
}
