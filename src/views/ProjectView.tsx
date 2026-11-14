import { ChevronDown, ChevronRight } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties } from "react";

import type { Topic, TopicStatus } from "../api";
import { WaitingStep, type WaitTarget } from "../components/WaitingStep";
import { plural } from "../lib/text";
import { waitText } from "../lib/today";
import { driftStore, useWaiting, waitingStore } from "../store/signals";
import { Menu } from "../components/Menu";
import { ProjectHeader } from "../components/ProjectHeader";
import { TaskList } from "../components/TaskList";
import { TopicItem, isWaiting } from "../components/TopicItem";
import { SubmitInput, useStoredFlag } from "../components/ui";
import { sortTopics, tasksAt } from "../lib/group";
import type { ViewProps } from "../router/registry";
import { links } from "../router/router";
import { takeReveal, useReveal } from "../store/ui";
import { useVault, useView } from "../store/vault";
import { describeError } from "../lib/errors";
import { showStrip } from "../lib/updates";
import { EMPTY_SELECTION, bulkSummary, bulkTargets, clickSelect, pruneSelection, selectionLabel, statusChoices, type BulkResult, type Selection } from "../lib/topicStatus";
import { UpdateStrip } from "./project/UpdateStrip";

export function ProjectView({ route }: ViewProps) {
  const view = useView();
  const { run, notify, refresh, api } = useVault();
  const slug = route.params.project ?? "";
  const focusTopic = route.query.get("temat");
  const project = view.snapshot.projects.find((p) => p.slug === slug);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set(focusTopic ? [focusTopic] : []));
  const [showDoneTopics, setShowDoneTopics] = useStoredFlag(`montybrain.doneTopics.${slug}`, false);
  // Selection mode ("Zaznacz"): one status for many topics, without expanding them.
  const [selecting, setSelecting] = useState(false);
  const [selection, setSelection] = useState<Selection>(EMPTY_SELECTION);
  const [bulkMenu, setBulkMenu] = useState<HTMLElement | null>(null);
  const [bulkBusy, setBulkBusy] = useState(false);
  const [bulkWait, setBulkWait] = useState<WaitTarget[] | null>(null);
  const waiting = useWaiting();
  useEffect(() => {
    void waitingStore.start().catch(() => {});
    void driftStore.start().catch(() => {});
  }, []);
  const selectBtn = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    setSelecting(false);
    setSelection(EMPTY_SELECTION);
  }, [slug]);

  // Following a link to a topic (e.g. from Dziś) opens it and scrolls to it.
  useEffect(() => {
    if (!focusTopic) return;
    setExpanded((s) => new Set(s).add(focusTopic));
    if (view.snapshot.topics.some((t) => t.project === slug && t.slug === focusTopic && t.status === "done")) {
      setShowDoneTopics(true);
    }
    requestAnimationFrame(() =>
      document.getElementById(`topic-${slug}-${focusTopic}`)?.parentElement?.scrollIntoView({ block: "start", behavior: "smooth" }),
    );
  }, [slug, focusTopic]);

  // Jumped here from the board / palette: open the topic (and its done list) and scroll to it.
  const reveal = useReveal();
  useEffect(() => {
    if (reveal.project !== slug || !reveal.topic || !takeReveal(reveal, "project-view")) return;
    const topic = reveal.topic;
    setExpanded((s) => new Set(s).add(topic));
    if (view.snapshot.topics.some((t) => t.project === slug && t.slug === topic && t.status === "done")) {
      setShowDoneTopics(true);
    }
    if (!reveal.task) {
      requestAnimationFrame(() =>
        document.getElementById(`topic-${slug}-${topic}`)?.parentElement?.scrollIntoView({ block: "start", behavior: "smooth" }),
      );
    }
  }, [reveal.nonce, slug]);

  if (!project) {
    return (
      <div className="view">
        <div className="empty">
          <p className="empty-title">Nie ma takiego projektu.</p>
          <p className="empty-text">
            Mógł zostać usunięty albo przemianowany na dysku. <a href={links.today()}>Wróć do widoku Dziś</a>.
          </p>
        </div>
      </div>
    );
  }

  const topics = sortTopics(view.snapshot.topics.filter((t) => t.project === slug));
  // Waiting and blocked topics form their own quiet group between the active and the done ones.
  const active = topics.filter((t) => t.status !== "done" && !isWaiting(t.status));
  const waitingTopics = topics.filter((t) => isWaiting(t.status));
  const done = topics.filter((t) => t.status === "done");
  const waitOf = (t: Topic) => waiting.list.find((w) => w.key === `topic:${slug}/${t.slug}`) ?? null;
  const projectLoc = { kind: "project", project: slug } as const;
  const toggle = (t: string) =>
    setExpanded((s) => {
      const next = new Set(s);
      if (next.has(t)) next.delete(t);
      else next.add(t);
      return next;
    });

  // The rows as shown: active topics, then the done ones while their group is open.
  const shown = [...active, ...waitingTopics, ...(showDoneTopics ? done : [])];
  const order = shown.map((t) => t.slug);
  const sel = pruneSelection(selection, order);
  const stopSelecting = (focus = true) => {
    setSelecting(false);
    setSelection(EMPTY_SELECTION);
    setBulkMenu(null);
    if (focus) requestAnimationFrame(() => selectBtn.current?.focus());
  };
  const setStatusOfSelected = async (status: TopicStatus) => {
    const targets = bulkTargets(shown, sel.selected, status);
    setBulkBusy(true);
    const results: BulkResult[] = [];
    // One after another, through the same topic update as a single change.
    for (const t of targets) {
      try {
        await api.updateTopic(slug, t.slug, { status });
        results.push({ title: t.title, error: null });
      } catch (e) {
        results.push({ title: t.title, error: describeError(e) });
      }
    }
    await refresh(); // once for all of them
    setBulkBusy(false);
    notify(bulkSummary(results, status));
    stopSelecting();
    // The waiting step for everything that now waits (skippable).
    const changed = targets.filter((t) => results.find((r) => r.title === t.title)?.error === null);
    if ((status === "waiting" || status === "blocked") && changed.length) {
      setBulkWait(changed.map((t) => ({ key: `topic:${slug}/${t.slug}`, kind: "topic" as const, status, project: slug, topic: t.slug })));
    }
  };

  const renderTopic = (t: (typeof topics)[number]) => (
    <TopicItem
      key={t.slug}
      project={project}
      topic={t}
      expanded={expanded.has(t.slug)}
      onToggle={() => toggle(t.slug)}
      select={selecting ? { checked: sel.selected.has(t.slug), onClick: (shift) => setSelection(clickSelect(order, sel, t.slug, shift)) } : undefined}
      waitLine={isWaiting(t.status) ? (waitOf(t) ? waitText(waitOf(t)!, view.today) : null) : null}
    />
  );

  return (
    <div className="view" style={project.color ? ({ "--accent": project.color } as CSSProperties) : undefined}>
      <ProjectHeader key={project.slug} project={project} />
      {showStrip(project) && <UpdateStrip key={`upd-${project.slug}`} project={project} openAuto={route.query.get("auto") === "1"} openDrift={route.query.get("drift") === "1"} />}

      <section className="group" aria-labelledby="h-topics">
        <h2 className="group-head" id="h-topics">
          Tematy <span className="count">{active.length}</span>
          {topics.length > 0 && !selecting && (
            <button ref={selectBtn} type="button" className="group-head-action" onClick={() => {
                setSelecting(true);
                requestAnimationFrame(() => document.querySelector<HTMLInputElement>(".topics .topic-check")?.focus());
              }}
              title="Zaznacz tematy, żeby zmienić status wielu naraz">
              Zaznacz
            </button>
          )}
        </h2>
        {selecting && (
          <div className="select-bar" role="toolbar" aria-label="Zaznaczone tematy" onKeyDown={(e) => e.key === "Escape" && !bulkMenu && stopSelecting()}>
            <span className="select-count" aria-live="polite">
              {selectionLabel(sel.selected.size)}
            </span>
            <span className="select-sep" aria-hidden>
              ·
            </span>
            <button
              type="button"
              className="select-btn"
              aria-haspopup="menu"
              aria-expanded={bulkMenu !== null}
              disabled={sel.selected.size === 0 || bulkBusy}
              onClick={(e) => setBulkMenu(bulkMenu ? null : e.currentTarget)}
            >
              {bulkBusy ? "Zapisuję…" : "Ustaw status"}
              <ChevronDown size={12} aria-hidden />
            </button>
            <span className="select-sep" aria-hidden>
              ·
            </span>
            <button type="button" className="select-btn" disabled={bulkBusy} onClick={() => stopSelecting()}>
              Anuluj
            </button>
            <span className="select-hint">Shift: zaznacz zakres</span>
          </div>
        )}
        {bulkMenu && (
          <Menu
            anchor={bulkMenu}
            label="Ustaw status zaznaczonych tematów"
            minWidth={150}
            onClose={() => setBulkMenu(null)}
            items={statusChoices(null).map((c) => ({ key: c.status, label: c.label, onSelect: () => void setStatusOfSelected(c.status) }))}
          />
        )}
        {topics.length === 0 && (
          <p className="empty-text">
            Temat to jeden wątek pracy — funkcja, migracja, research. Ma własne zadania i dziennik.
          </p>
        )}
        {bulkWait && (
          <WaitingStep
            targets={bulkWait}
            title={`${bulkWait.length} ${plural(bulkWait.length, "temat czeka", "tematy czekają", "tematów czeka")}: na co i kiedy przypomnieć?`}
            onDone={() => setBulkWait(null)}
          />
        )}
        <div className="topics">{active.map(renderTopic)}</div>
        <div className="add-row topic-add">
          <span className="add-plus" aria-hidden>
            +
          </span>
          <SubmitInput
            className="field-inline"
            placeholder="Nowy temat…"
            aria-label="Nowy temat"
            draftKey={`new-topic:${slug}`}
            onSubmit={async (title) => {
              const t = await run((a) => a.createTopic(slug, { title, status: "planned" }));
              if (!t) return false;
              setExpanded((s) => new Set(s).add(t.slug));
            }}
          />
        </div>
        {waitingTopics.length > 0 && (
          <div className="wait-group" role="group" aria-labelledby="h-waiting-topics">
            <h3 className="wait-group-head" id="h-waiting-topics">
              Czeka <span className="count">{waitingTopics.length}</span>
            </h3>
            <div className="topics">{waitingTopics.map(renderTopic)}</div>
          </div>
        )}
        {done.length > 0 && (
          <div className="done-group">
            <button
              type="button"
              className="done-toggle"
              aria-expanded={showDoneTopics}
              onClick={() => setShowDoneTopics(!showDoneTopics)}
            >
              <ChevronRight size={12} className="chev" />
              Zakończone tematy <span className="count">{done.length}</span>
            </button>
            {showDoneTopics && <div className="topics">{done.map(renderTopic)}</div>}
          </div>
        )}
      </section>

      <section className="group" aria-labelledby="h-project-tasks">
        <h2 className="group-head" id="h-project-tasks">
          Zadania projektu
        </h2>
        <TaskList
          listId={`p:${slug}`}
          tasks={tasksAt(view.snapshot, projectLoc)}
          addTo={projectLoc}
          addPlaceholder="Dodaj zadanie ogólne"
        />
      </section>
    </div>
  );
}
