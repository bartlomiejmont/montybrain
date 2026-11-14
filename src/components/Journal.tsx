import { useState } from "react";

import type { JournalEntry, TopicStatus } from "../api";
import { formatStamp } from "../lib/dates";
import { TOPIC_STATUSES, TOPIC_STATUS_LABEL } from "../lib/labels";
import { clearAfterSubmit } from "../lib/form";
import { links } from "../router/router";
import { useDraft, useSubmitting } from "../store/drafts";
import { useVault, useView } from "../store/vault";
import { Button, ProjectDot } from "./ui";

export function JournalList({ entries, showTopic = false }: { entries: JournalEntry[]; showTopic?: boolean }) {
  const view = useView();
  return (
    <ol className="journal">
      {entries.map((e) => {
        const project = view.snapshot.projects.find((p) => p.slug === e.project);
        const topic = view.snapshot.topics.find((t) => t.project === e.project && t.slug === e.topic);
        return (
          <li key={`${e.project}/${e.topic}/${e.at}/${e.line}`} className="j-entry">
            <time className="j-time" dateTime={e.at} title={e.at.replace("T", " ")}>
              {formatStamp(e.at, view.today)}
            </time>
            <div className="j-body">
              <div className="j-head">
                <span className="j-author">{e.author}</span>
                {showTopic && (
                  <a className="j-topic" href={links.project(e.project, e.topic)} title={`${project?.name ?? e.project} · ${topic?.title ?? e.topic}`}>
                    <ProjectDot color={project?.color} size={6} />
                    <span className="ctx-text">{topic?.title ?? e.topic}</span>
                  </a>
                )}
              </div>
              <p className="j-text">{e.text}</p>
            </div>
          </li>
        );
      })}
    </ol>
  );
}

/** Adds a journal entry; can change the topic status in the same write. */
export function JournalForm({ project, topic, status }: { project: string; topic: string; status: TopicStatus }) {
  const { run } = useVault();
  // Kept in the draft store: survives the topic moving between lists (so does
  // the "submitting" flag: a re-created form cannot send the entry twice).
  const key = `journal:${project}/${topic}`;
  const { draft: text, setDraft: setText, state } = useDraft(key, "");
  const [nextStatus, setNextStatus] = useState<TopicStatus | "">("");
  const { busy, begin, end } = useSubmitting(key);

  const submit = async () => {
    const t = text.trim();
    if (!t || !begin()) return;
    let ok = false;
    try {
      ok = (await run((a) => a.addJournalEntry(project, topic, { text: t, author: "ja", status: nextStatus || null }))) !== undefined;
    } finally {
      end();
    }
    const now = state().draft;
    const next = clearAfterSubmit(ok, now, text);
    if (next !== now) setText(next);
    if (ok) setNextStatus("");
  };

  return (
    <form
      className="j-form"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <textarea
        className="field"
        rows={text.includes("\n") ? 3 : 1}
        placeholder="Co się wydarzyło? (⌘↩ zapisuje)"
        aria-label="Nowy wpis w dzienniku"
        value={text}
        disabled={busy}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
            e.preventDefault();
            void submit();
          }
        }}
      />
      <div className="j-form-row">
        <select
          className="field select"
          aria-label="Zmień status tematu"
          value={nextStatus}
          onChange={(e) => setNextStatus(e.target.value as TopicStatus | "")}
        >
          <option value="">Status bez zmian</option>
          {TOPIC_STATUSES.filter((s) => s !== status).map((s) => (
            <option key={s} value={s}>
              Zmień na: {TOPIC_STATUS_LABEL[s]}
            </option>
          ))}
        </select>
        <Button type="submit" variant="primary" disabled={!text.trim() || busy}>
          Dodaj wpis
        </Button>
      </div>
    </form>
  );
}
