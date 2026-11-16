import { t as tr } from "../i18n";
import { askAboutTopic } from "../store/session";
import { ChevronDown, ChevronRight, GitBranch, MoreHorizontal } from "lucide-react";
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
import { useOpenDetail } from "../store/details";
import { taskTarget } from "../lib/details";
import { useVault, useView } from "../store/vault";
import { Menu } from "./Menu";
import { TopicSummary } from "./TopicSummary";
import { DraftHint, SaveActions, Segmented } from "./ui";

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
        title={tr("Zmień status tematu")}
        onClick={(e) => setAnchor(anchor ? null : e.currentTarget)}
      >
        <span className="badge-mark" aria-hidden />
        {TOPIC_STATUS_LABEL[topic.status]}
        <ChevronDown size={11} className="badge-chev" aria-hidden />
      </button>
      {anchor && (
        <Menu
          anchor={anchor}
          label={tr("Status tematu {0}", topic.title)}
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
      const kind = n.edge.kind === "waits" ? (n.outgoing ? tr("Czeka na") : tr("Czeka na ten temat")) : tr("Powiązane");
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
    notify((await copyText(text)) ? tr("Skopiowano kontekst tematu.") : tr("Nie udało się skopiować do schowka."));
  };
}

export function TopicEditor({ project, topic, onDone }: { project: Project; topic: Topic; onDone: (status?: TopicStatus) => void }) {
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
        {tr("Tytuł")}</label>
      <input
        id={`tt-${topic.slug}`}
        className="field"
        value={titleD.draft}
        autoFocus
        onChange={(e) => titleD.setDraft(e.target.value)}
        onFocus={titleD.onFocus}
        onBlur={titleD.onBlur}
      />
      <DraftHint draft={titleD} what={tr("Tytuł")} />
      <span className="d-label">Status</span>
      <Segmented<TopicStatus>
        label={tr("Status tematu")}
        value={status}
        onChange={(s) => statusD.setDraft(s)}
        options={TOPIC_STATUSES.map((s) => ({ value: s, label: TOPIC_STATUS_LABEL[s] }))}
      />
      <DraftHint draft={statusD} what="Status" />
      <label className="d-label" htmlFor={`tr-${topic.slug}`}>
        {tr("Repozytorium")}</label>
      <select
        id={`tr-${topic.slug}`}
        className="field select"
        value={repo}
        onChange={(e) => repoD.setDraft(e.target.value)}
        onFocus={repoD.onFocus}
        onBlur={repoD.onBlur}
      >
        <option value="">{tr("— brak —")}</option>
        {project.repos.map((r) => (
          <option key={r.name} value={r.name}>
            {r.name}
          </option>
        ))}
      </select>
      <DraftHint draft={repoD} what={tr("Repozytorium")} />
      <label className="d-label" htmlFor={`ts-${topic.slug}`}>
        {tr("Stan obecny")}</label>
      <textarea
        id={`ts-${topic.slug}`}
        className="field"
        rows={4}
        placeholder={tr("Krótko: gdzie jesteśmy, co dalej.")}
        value={summaryD.draft}
        onChange={(e) => summaryD.setDraft(e.target.value)}
        onFocus={summaryD.onFocus}
        onBlur={summaryD.onBlur}
      />
      <DraftHint draft={summaryD} what={tr("Stan obecny")} />
      <SaveActions busy={submitting.busy} onCancel={cancel} />
    </form>
  );
}

/** Project list: a read-only preview. All topic editing lives in the side panel. */
export function TopicItem({ project, topic, onOpen, waitLine }: {
  project: Project;
  topic: Topic;
  onOpen: () => void;
  waitLine?: string | null;
}) {
  const view = useView();
  const openDetail = useOpenDetail();
  const loc = { kind: "topic", project: project.slug, topic: topic.slug } as const;
  const counts = countTasks(view.snapshot).topic.get(topicKey(project.slug, topic.slug)) ?? { open: 0, total: 0 };
  const tasks = tasksAt(view.snapshot, loc);
  const nextTask = tasks.find((t) => t.status === "doing") ?? tasks.find((t) => t.status !== "done");
  const copy = useCopyContext();
  const [menu, setMenu] = useState<HTMLElement | null>(null);
  return <section id={`topic-${project.slug}-${topic.slug}`} className="topic" data-status={topic.status}>
    <div className="topic-head">
      <button type="button" className="topic-toggle" onClick={onOpen} title={tr("Otwórz szczegóły tematu w panelu")}>
        <ChevronRight size={14} className="chev" aria-hidden />
        <span className="topic-title">{topic.title}</span>
      </button>
      <TopicStatusBadge status={topic.status} />
      <span className="topic-meta">
        {topic.repo && <span className="repo-tag" title={tr("Repozytorium: {0}", topic.repo)}><GitBranch size={11} />{topic.repo}</span>}
        <span className="count" title={openCount(counts.open, counts.total)} aria-label={openCount(counts.open, counts.total)}>{tr("Otwarte:")} {counts.open} {tr("z")} {counts.total}</span>
        {topic.updated && <span className="age" title={tr("Zaktualizowano {0}", topic.updated.replace("T", " "))}>{formatAge(topic.updated, view.today)}</span>}
      </span>
      <button type="button" className="icon-btn topic-more" aria-label={tr("Więcej: {0}", topic.title)} aria-haspopup="menu" aria-expanded={menu !== null} onClick={(e) => setMenu(menu ? null : e.currentTarget)}><MoreHorizontal size={14} /></button>
      {menu && <Menu anchor={menu} label={tr("Temat {0}", topic.title)} onClose={() => setMenu(null)} items={[
        { key: "details", label: tr("Otwórz szczegóły"), onSelect: onOpen },
        { key: "agent", label: tr("Zapytaj agenta…"), onSelect: () => askAboutTopic(topic) },
        { key: "copy", label: tr("Kopiuj kontekst"), onSelect: () => void copy(project.slug, topic) },
        { key: "map", label: tr("Pokaż na mapie"), onSelect: () => navigate(links.map(`${project.slug}/${topic.slug}`)) },
      ]} />}
    </div>
    {waitLine && <p className="topic-wait">{waitLine}</p>}
    <div className="topic-preview">
      {topic.summary && <TopicSummary text={topic.summary} compact />}
      {nextTask && <p className="topic-next-step"><span>{tr("Następne zadanie")}</span><button type="button" onClick={() => openDetail?.(taskTarget(nextTask))}>{nextTask.title}</button></p>}
    </div>
  </section>;
}
