import { t as tr } from "../i18n";
import { ChevronRight, Plus } from "lucide-react";
import { useEffect, type CSSProperties } from "react";

import { ProjectHeader } from "../components/ProjectHeader";
import { TaskList } from "../components/TaskList";
import { TopicItem, isWaiting } from "../components/TopicItem";
import { SubmitInput, useStoredFlag } from "../components/ui";
import { taskTarget, topicTarget } from "../lib/details";
import { sortTopics, tasksAt } from "../lib/group";
import { waitText } from "../lib/today";
import type { ViewProps } from "../router/registry";
import { links } from "../router/router";
import { useOpenDetail } from "../store/details";
import { driftStore, useWaiting, waitingStore } from "../store/signals";
import { takeReveal, useReveal } from "../store/ui";
import { useVault, useView } from "../store/vault";
import { UpdateStrip } from "./project/UpdateStrip";

export function ProjectView({ route }: ViewProps) {
  const view = useView();
  const { run } = useVault();
  const openDetail = useOpenDetail();
  const slug = route.params.project ?? "";
  const focusTopic = route.query.get("temat");
  const project = view.snapshot.projects.find((p) => p.slug === slug);
  const [showDone, setShowDone] = useStoredFlag(`montybrain.doneTopics.${slug}`, false);
  const waiting = useWaiting();
  const reveal = useReveal();
  useEffect(() => {
    void waitingStore.start().catch(() => {});
    void driftStore.start().catch(() => {});
  }, []);
  useEffect(() => {
    if (focusTopic) openDetail?.({ kind: "topic", project: slug, topic: focusTopic });
  }, [slug, focusTopic]);
  useEffect(() => {
    if (reveal.project !== slug || !takeReveal(reveal, "project-view")) return;
    const task = reveal.task ? view.snapshot.tasks.find((t) => t.id === reveal.task) : null;
    if (task) openDetail?.(taskTarget(task));
    else if (reveal.topic) openDetail?.({ kind: "topic", project: slug, topic: reveal.topic });
  }, [reveal.nonce, slug]);

  if (!project) return <div className="view"><div className="empty"><p className="empty-title">{tr("Nie ma takiego projektu.")}</p><p className="empty-text">{tr("Mógł zostać usunięty albo przemianowany na dysku.")} <a href={links.today()}>{tr("Wróć do widoku Dziś")}</a>.</p></div></div>;
  const topics = sortTopics(view.snapshot.topics.filter((t) => t.project === slug));
  const active = topics.filter((t) => t.status !== "done" && !isWaiting(t.status));
  const waitingTopics = topics.filter((t) => isWaiting(t.status));
  const done = topics.filter((t) => t.status === "done");
  const projectLoc = { kind: "project", project: slug } as const;
  const renderTopic = (t: (typeof topics)[number]) => {
    const wait = waiting.list.find((w) => w.key === `topic:${slug}/${t.slug}`);
    return <TopicItem key={t.slug} project={project} topic={t} onOpen={() => openDetail?.(topicTarget(t))} waitLine={wait && isWaiting(t.status) ? waitText(wait, view.today) : null} />;
  };
  return <div className="view project-view" style={project.color ? { "--accent": project.color } as CSSProperties : undefined}>
    <ProjectHeader key={project.slug} project={project} />
    <UpdateStrip key={`upd-${project.slug}`} project={project} openAuto={route.query.get("auto") === "1"} openDrift={route.query.get("drift") === "1"} />
    <section className="group" aria-labelledby="h-topics">
      <h2 className="group-head" id="h-topics">{tr("Tematy")} <span className="count">{active.length}</span><span className="group-hint">{tr("Kliknij temat — podgląd i edycja są w panelu po prawej.")}</span></h2>
      {!topics.length && <p className="empty-text">{tr("Temat to jeden wątek pracy. Ma własne zadania i dziennik.")}</p>}
      <div className="topics">{active.map(renderTopic)}</div>
      <div className="add-row topic-add"><span className="add-plus" aria-hidden><Plus size={14} /></span><SubmitInput className="field-inline" placeholder={tr("Nowy temat…")} aria-label={tr("Nowy temat")} draftKey={`new-topic:${slug}`} onSubmit={async (title) => {
        const topic = await run((a) => a.createTopic(slug, { title, status: "planned" }));
        if (!topic) return false;
        openDetail?.(topicTarget(topic));
      }} /></div>
      {waitingTopics.length > 0 && <div className="wait-group" role="group" aria-labelledby="h-waiting-topics"><h3 className="wait-group-head" id="h-waiting-topics">{tr("Czeka")} <span className="count">{waitingTopics.length}</span></h3><div className="topics">{waitingTopics.map(renderTopic)}</div></div>}
      {done.length > 0 && <div className="done-group"><button type="button" className="done-toggle" aria-expanded={showDone} onClick={() => setShowDone(!showDone)}><ChevronRight size={12} className="chev" />{tr("Zakończone tematy")} <span className="count">{done.length}</span></button>{showDone && <div className="topics">{done.map(renderTopic)}</div>}</div>}
    </section>
    <section className="group" aria-labelledby="h-project-tasks"><h2 className="group-head" id="h-project-tasks">{tr("Zadania projektu")}</h2><TaskList listId={`p:${slug}`} tasks={tasksAt(view.snapshot, projectLoc)} addTo={projectLoc} addPlaceholder="Dodaj zadanie ogólne" /></section>
  </div>;
}
