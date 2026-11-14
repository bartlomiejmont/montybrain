import { ChevronDown, ChevronRight, Copy, FileText, GitBranch, MoreHorizontal } from "lucide-react";
import { useState } from "react";

import type { Project, Topic, TopicStatus } from "../api";
import { buildTopicContext, copyText } from "../lib/context";
import { buildGraph, neighbours, topicKey as refTopicKey } from "../lib/refs";
import { firstSentence } from "../lib/today";
import { links, navigate } from "../router/router";
import { formatAge } from "../lib/dates";
import { countTasks, journalOf, tasksAt, topicKey } from "../lib/group";
import { TOPIC_STATUSES, TOPIC_STATUS_LABEL } from "../lib/labels";
import { closeAfterSave, once, submitAction, topicPatch } from "../lib/form";
import { openCount, statusChoices, statusPillLabel } from "../lib/topicStatus";
import { dropDrafts, useDraft, useSubmitting } from "../store/drafts";
import { useDrift } from "../store/signals";
import { useTaskFlag } from "../store/ui";
import { useVault, useView } from "../store/vault";
import { useActions } from "./actions";
import { JournalForm, JournalList } from "./Journal";
import { Menu } from "./Menu";
import { TaskList } from "./TaskList";
import { WaitingStep, openWaitStep, useWaitStepOpen } from "./WaitingStep";
import { Button, ConfirmButton, DraftHint, SaveActions, Segmented } from "./ui";

export function TopicStatusBadge({ status }: { status: TopicStatus }) {
  return (
    <span className={`badge topic-${status}`}>
      <span className="badge-mark" aria-hidden />
      {TOPIC_STATUS_LABEL[status]}
    </span>
  );
}

export const isWaiting = (s: TopicStatus) => s === "waiting" || s === "blocked";

/**
 * The topic's status as a control: opens the five statuses; choosing one saves
 * it at once through the same topic update as the editor (no journal entry).
 * `onChosen` hears the saved status (the waiting step follows Czeka / Zablokowany).
 */
export function TopicStatusPill({ project, topic, onChosen }: { project: Pick<Project, "slug">; topic: Topic; onChosen?: (s: TopicStatus) => void }) {
  const { run } = useVault();
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const [saving, setSaving] = useState(false);
  const choose = async (status: TopicStatus) => {
    if (status === topic.status) return;
    setSaving(true);
    const ok = (await run((a) => a.updateTopic(project.slug, topic.slug, { status }))) !== undefined;
    setSaving(false);
    if (ok) onChosen?.(status);
  };
  return (
    <>
      <button
        type="button"
        className={`badge badge-btn topic-${topic.status}`}
        aria-haspopup="menu"
        aria-expanded={anchor !== null}
        aria-label={statusPillLabel(topic.title, topic.status)}
        aria-busy={saving || undefined}
        title="Zmień status tematu"
        onClick={(e) => setAnchor(anchor ? null : e.currentTarget)}
      >
        <span className="badge-mark" aria-hidden />
        {TOPIC_STATUS_LABEL[topic.status]}
        <ChevronDown size={11} className="badge-chev" aria-hidden />
      </button>
      {anchor && (
        <Menu
          anchor={anchor}
          label={`Status tematu ${topic.title}`}
          minWidth={150}
          onClose={() => setAnchor(null)}
          items={statusChoices(topic.status).map((c) => ({
            key: c.status,
            label: c.label,
            checked: c.checked,
            onSelect: () => void choose(c.status),
          }))}
        />
      )}
    </>
  );
}

/** "Kopiuj kontekst": the topic's Markdown block for an agent session. */
export function useCopyContext() {
  const view = useView();
  const drift = useDrift();
  const { notify } = useVault();
  return async (projectSlug: string, topic: Topic) => {
    const project = view.snapshot.projects.find((p) => p.slug === projectSlug);
    if (!project) return;
    const branches = drift.list.find((d) => d.project === projectSlug)?.branches.filter((b) => b.topic === topic.slug) ?? [];
    // One-hop neighbours on the map (topics only), with the first sentence of each.
    const graph = buildGraph(view.snapshot);
    const neighbourList = neighbours(graph, refTopicKey(projectSlug, topic.slug)).flatMap((n) => {
      const other = view.snapshot.topics.find((t) => refTopicKey(t.project, t.slug) === n.other);
      if (!other) return [];
      const kind = n.edge.kind === "waits" ? (n.outgoing ? "Czeka na" : "Czeka na ten temat") : "Powiązane";
      const name = view.snapshot.projects.find((p) => p.slug === other.project)?.name ?? other.project;
      const prose = other.summary.split("\n").filter((l) => !/^(Czeka na|Powiązane):/.test(l.trim())).join(" ");
      return [{ kind, title: other.title, project: name, sentence: firstSentence(prose) }];
    });
    const text = buildTopicContext({
      project,
      topic,
      journal: journalOf(view.snapshot, projectSlug, topic.slug),
      tasks: tasksAt(view.snapshot, { kind: "topic", project: projectSlug, topic: topic.slug }),
      branches,
      neighbours: neighbourList,
    });
    notify((await copyText(text)) ? "Skopiowano kontekst tematu." : "Nie udało się skopiować do schowka.");
  };
}

function TopicEditor({ project, topic, onDone }: { project: Project; topic: Topic; onDone: (status?: TopicStatus) => void }) {
  const { run } = useVault();
  // Drafts live in the draft store: they survive file changes and the topic
  // moving between lists. Saving applies the user's version.
  const dk = `te:${project.slug}/${topic.slug}:`;
  const titleD = useDraft(`${dk}title`, topic.title);
  const statusD = useDraft(`${dk}status`, topic.status);
  const repoD = useDraft(`${dk}repo`, topic.repo ?? "");
  const summaryD = useDraft(`${dk}summary`, topic.summary);
  const status = statusD.draft as TopicStatus;
  const repo = repoD.draft;

  const fields = [titleD, statusD, repoD, summaryD];
  const submitting = useSubmitting(dk);

  const finish = () => {
    dropDrafts(dk);
    onDone();
  };

  // Only fields the user changed are sent, compared with what will be stored
  // (lib/form.ts): a field left untouched while an agent rewrote it must not be
  // written back with the value the editor opened with. While the request is in
  // flight the fields stay editable but "Zapisz" is ignored; the editor closes
  // only if the request succeeded and nothing was typed meanwhile.
  const save = async () => {
    const patch = topicPatch({
      title: { edited: titleD.edited(), stored: titleD.stored() },
      status: { edited: statusD.edited(), stored: statusD.stored() },
      repo: { edited: repoD.edited(), stored: repoD.stored() },
      summary: { edited: summaryD.edited(), stored: summaryD.stored() },
    });
    const action = submitAction(submitting.isBusy(), patch);
    if (action === "ignore") return;
    if (action === "close") return finish();
    if (!submitting.begin()) return;
    try {
      const call = once(() => run((a) => a.updateTopic(project.slug, topic.slug, patch)));
      // Acknowledged by the result: no "changed elsewhere" flash while it is in flight.
      const acked = [
        patch.title !== undefined && titleD.save(patch.title, call, (t) => t.title),
        patch.status !== undefined && statusD.save(patch.status, call, (t) => t.status),
        patch.repo !== undefined && repoD.save(patch.repo ?? "", call, (t) => t.repo ?? ""),
        patch.summary !== undefined && summaryD.save(patch.summary, call, (t) => t.summary),
      ];
      const ok = (await call()) !== undefined;
      await Promise.all(acked); // the fields' own results are applied
      if (closeAfterSave(ok, fields.map((f) => f.state()))) {
        dropDrafts(dk);
        onDone(ok && patch.status && patch.status !== topic.status ? patch.status : undefined);
      }
    } finally {
      submitting.end();
    }
  };
  const cancel = () => !submitting.isBusy() && finish();

  return (
    <form
      className="editor"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
      onKeyDown={(e) => e.key === "Escape" && cancel()}
    >
      <label className="d-label" htmlFor={`tt-${topic.slug}`}>
        Tytuł
      </label>
      <input
        id={`tt-${topic.slug}`}
        className="field"
        value={titleD.draft}
        autoFocus
        onChange={(e) => titleD.setDraft(e.target.value)}
        onFocus={titleD.onFocus}
        onBlur={titleD.onBlur}
      />
      <DraftHint draft={titleD} what="Tytuł" />
      <span className="d-label">Status</span>
      <Segmented<TopicStatus>
        label="Status tematu"
        value={status}
        onChange={(s) => statusD.setDraft(s)}
        options={TOPIC_STATUSES.map((s) => ({ value: s, label: TOPIC_STATUS_LABEL[s] }))}
      />
      <DraftHint draft={statusD} what="Status" />
      <label className="d-label" htmlFor={`tr-${topic.slug}`}>
        Repozytorium
      </label>
      <select
        id={`tr-${topic.slug}`}
        className="field select"
        value={repo}
        onChange={(e) => repoD.setDraft(e.target.value)}
        onFocus={repoD.onFocus}
        onBlur={repoD.onBlur}
      >
        <option value="">— brak —</option>
        {project.repos.map((r) => (
          <option key={r.name} value={r.name}>
            {r.name}
          </option>
        ))}
      </select>
      <DraftHint draft={repoD} what="Repozytorium" />
      <label className="d-label" htmlFor={`ts-${topic.slug}`}>
        Stan obecny
      </label>
      <textarea
        id={`ts-${topic.slug}`}
        className="field"
        rows={4}
        placeholder="Krótko: gdzie jesteśmy, co dalej."
        value={summaryD.draft}
        onChange={(e) => summaryD.setDraft(e.target.value)}
        onFocus={summaryD.onFocus}
        onBlur={summaryD.onBlur}
      />
      <DraftHint draft={summaryD} what="Stan obecny" />
      <SaveActions busy={submitting.busy} onCancel={cancel} />
    </form>
  );
}

export function TopicItem({
  project,
  topic,
  expanded,
  onToggle,
  select,
  waitLine,
}: {
  project: Project;
  topic: Topic;
  expanded: boolean;
  onToggle: () => void;
  /** Selection mode of the project view: the row's checkbox. */
  select?: { checked: boolean; onClick: (shift: boolean) => void };
  /** Waiting topics: what it waits for and when it is checked again. */
  waitLine?: string | null;
}) {
  const view = useView();
  const { run } = useVault();
  const { reveal } = useActions();
  const [editing, setEditing] = useTaskFlag("topic-editing", `${project.slug}/${topic.slug}`);
  const loc = { kind: "topic", project: project.slug, topic: topic.slug } as const;
  const counts = countTasks(view.snapshot).topic.get(topicKey(project.slug, topic.slug)) ?? { open: 0, total: 0 };
  const panelId = `topic-${project.slug}-${topic.slug}`;
  const copyContext = useCopyContext();
  const [menu, setMenu] = useState<HTMLElement | null>(null);
  // The waiting step after the status became Czeka / Zablokowany.
  const waitKey = `topic:${project.slug}/${topic.slug}`;
  const waitStep = useWaitStepOpen(waitKey);
  const afterStatus = (s?: TopicStatus) => s && isWaiting(s) && openWaitStep(waitKey);

  return (
    <section className={`topic${expanded ? " expanded" : ""}`} data-status={topic.status}>
      <div className="topic-head">
        {select && (
          <input
            type="checkbox"
            className="topic-check"
            checked={select.checked}
            aria-label={`Zaznacz temat ${topic.title}`}
            onChange={() => {}}
            onClick={(e) => select.onClick(e.shiftKey)}
          />
        )}
        <button type="button" className="topic-toggle" aria-expanded={expanded} aria-controls={panelId} onClick={onToggle}>
          <ChevronRight size={14} className="chev" />
          <span className="topic-title" title={expanded ? undefined : topic.title}>
            {topic.title}
          </span>
        </button>
        <TopicStatusPill project={project} topic={topic} onChosen={afterStatus} />
        <span className="topic-meta">
          {topic.repo && (
            <span className="repo-tag" title={`Repozytorium: ${topic.repo}`}>
              <GitBranch size={11} />
              {topic.repo}
            </span>
          )}
          <span className="count" title={openCount(counts.open, counts.total)} aria-label={openCount(counts.open, counts.total)}>
            {counts.open}/{counts.total}
          </span>
          {topic.updated && (
            <span className="age" title={`Zaktualizowano ${topic.updated.replace("T", " ")}`}>
              {formatAge(topic.updated, view.today)}
            </span>
          )}
        </span>
        <button
          type="button"
          className="icon-btn topic-more"
          aria-label={`Więcej: ${topic.title}`}
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
            label={`Temat ${topic.title}`}
            minWidth={190}
            onClose={() => setMenu(null)}
            items={[
              { key: "copy", label: "Kopiuj kontekst", onSelect: () => void copyContext(project.slug, topic) },
              { key: "map", label: "Pokaż na mapie", onSelect: () => navigate(links.map(`${project.slug}/${topic.slug}`)) },
            ]}
          />
        )}
      </div>
      {waitLine && <p className="topic-wait">{waitLine}</p>}
      {waitStep && (
        <WaitingStep
          targets={[{ key: waitKey, kind: "topic", status: topic.status === "blocked" ? "blocked" : "waiting", project: project.slug, topic: topic.slug }]}
          onDone={() => openWaitStep(null)}
        />
      )}
      {!expanded && topic.summary && (
        <p className="topic-summary clamp" onClick={onToggle}>
          {topic.summary}
        </p>
      )}
      {expanded && (
        <div className="topic-body" id={panelId}>
          {editing ? (
            <TopicEditor
              project={project}
              topic={topic}
              onDone={(s) => {
                setEditing(false);
                afterStatus(s);
              }}
            />
          ) : (
            <>
              {topic.summary ? (
                <p className="topic-summary full">{topic.summary}</p>
              ) : (
                <p className="topic-summary muted">Brak opisu stanu. Dodaj go w edycji tematu.</p>
              )}
              <div className="topic-actions">
                <Button variant="quiet" onClick={() => setEditing(true)}>
                  Edytuj temat
                </Button>
                <Button variant="quiet" onClick={() => void reveal(topic.path)}>
                  <FileText size={13} /> Pokaż plik
                </Button>
                <Button variant="quiet" onClick={() => void copyContext(project.slug, topic)}>
                  <Copy size={13} /> Kopiuj kontekst
                </Button>
                <ConfirmButton
                  question="Usunąć temat razem z zadaniami i dziennikiem?"
                  confirmLabel="Usuń temat"
                  onConfirm={() => void run((a) => a.deleteTopic(project.slug, topic.slug))}
                >
                  Usuń
                </ConfirmButton>
              </div>
            </>
          )}

          <TaskList
            listId={`t:${project.slug}/${topic.slug}`}
            tasks={tasksAt(view.snapshot, loc)}
            addTo={loc}
            addPlaceholder="Dodaj zadanie"
          />

          <div className="subhead">Dziennik</div>
          <JournalForm project={project.slug} topic={topic.slug} status={topic.status} />
          <JournalList entries={journalOf(view.snapshot, project.slug, topic.slug)} />
        </div>
      )}
    </section>
  );
}
