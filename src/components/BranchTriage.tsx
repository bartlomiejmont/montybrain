import { t as tr } from "../i18n";
import { useEffect, useRef, useState } from "react";

import type { Project, ProjectDrift } from "../api";
import { createAndLinkBranch } from "../lib/branchTriage";
import { describeError } from "../lib/errors";
import { driftStore, useDrift } from "../store/signals";
import { useVault, useView } from "../store/vault";
import { Button } from "./ui";
import "./branches.css";

// A topic created before a failed link remains available after closing the panel.
const pendingTopics = new Map<string, string>();

export function BranchTriage({ project }: { project: Project }) {
  const drift = useDrift();
  const branches = drift.list.find((d) => d.project === project.slug)?.unknown ?? [];
  useEffect(() => { void driftStore.start().catch(() => {}); }, []);
  if (!drift.loaded) return <p className="empty-text">{tr("Wczytuję gałęzie…")}</p>;
  if (!branches.length) return <p className="empty-text">{tr("Wszystkie gałęzie są przypisane lub pominięte.")}</p>;
  return <div className="branch-triage">
    <p className="branch-help">{tr("Przypisz gałąź do tematu, żeby widzieć, z jaką pracą jest związana. Jeśli nie wymaga śledzenia, wybierz „Ignoruj”.")}</p>
    <ul className="branch-list">{branches.map((branch) => <BranchRow key={`${branch.repo}/${branch.branch}`} project={project} branch={branch} />)}</ul>
  </div>;
}

function BranchRow({ project, branch }: { project: Project; branch: ProjectDrift["unknown"][number] }) {
  const { api, notify, refresh } = useVault();
  const view = useView();
  const topics = view.snapshot.topics.filter((t) => t.project === project.slug && t.status !== "done");
  const [topic, setTopic] = useState("");
  const [newTopic, setNewTopic] = useState(false);
  const [title, setTitle] = useState("");
  const branchKey = JSON.stringify([project.slug, branch.repo, branch.branch]);
  const [created, setCreated] = useState<string | null>(() => pendingTopics.get(branchKey) ?? null);
  const createdRef = useRef<string | null>(created);
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const perform = async (action: () => Promise<ProjectDrift[]>, done: string) => {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    try {
      driftStore.set(await action());
      pendingTopics.delete(branchKey);
      await refresh();
      notify(done);
    } catch (error) {
      await refresh();
      notify(describeError(error));
    } finally {
      pending.current = false;
      setBusy(false);
    }
  };
  return <li className="branch-item">
    <strong className="branch-name">{branch.branch}</strong>
    <span className="branch-meta">{branch.repo}  {tr("· ostatni commit")} {branch.last_commit}</span>
    <form className="branch-assign" onSubmit={(e) => {
      e.preventDefault();
      if (topic) void perform(() => api.linkBranch(project.slug, branch.repo, branch.branch, topic), tr("Przypisano gałąź „{0}” do tematu.", branch.branch));
    }}>
      <select className="field select" aria-label={tr("Temat dla gałęzi {0}", branch.branch)} value={topic} disabled={busy} onChange={(e) => setTopic(e.target.value)}>
        <option value="">{tr("Wybierz temat…")}</option>
        {topics.map((t) => <option key={t.slug} value={t.slug}>{t.title}</option>)}
      </select>
      <Button type="submit" variant="ghost" disabled={!topic || busy}>{tr("Przypisz")}</Button>
    </form>
    <div className="branch-actions">
      <Button variant="quiet" disabled={busy} onClick={() => setNewTopic(!newTopic)}>{created ? tr("Dokończ przypisanie…") : tr("Nowy temat dla tej gałęzi…")}</Button>
      <Button variant="quiet" disabled={busy} onClick={() => void perform(() => api.ignoreBranch(project.slug, branch.repo, branch.branch), tr("Gałąź „{0}” nie będzie już wymagać uwagi.", branch.branch))}>{tr("Ignoruj")}</Button>
    </div>
    {newTopic && <form className="branch-new" onKeyDown={(e) => { if (e.key === "Escape" && !busy) { e.stopPropagation(); setNewTopic(false); } }} onSubmit={(e) => {
      e.preventDefault();
      if (!title.trim() && !createdRef.current) return;
      void perform(async () => (await createAndLinkBranch(api, project.slug, branch.repo, branch.branch, title.trim(), createdRef.current, (slug) => {
        createdRef.current = slug;
        pendingTopics.set(branchKey, slug);
        setCreated(slug);
      })).drift, tr("Utworzono temat i przypisano gałąź „{0}”.", branch.branch));
    }}>
      {created ? <p className="branch-help">{tr("Temat został utworzony. Ponów przypisanie gałęzi.")}</p> : <input className="field" autoFocus aria-label={tr("Nazwa nowego tematu dla {0}", branch.branch)} placeholder={tr("Nazwa tematu")} value={title} disabled={busy} onChange={(e) => setTitle(e.target.value)} />}
      <div className="branch-actions"><Button type="submit" variant="ghost" disabled={busy || (!created && !title.trim())}>{created ? tr("Ponów przypisanie") : tr("Utwórz i przypisz")}</Button><Button variant="quiet" disabled={busy} onClick={() => setNewTopic(false)}>{tr("Anuluj")}</Button></div>
    </form>}
  </li>;
}
