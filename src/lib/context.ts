// "Kopiuj kontekst": a Markdown block for starting an agent session on one
// topic. Deterministic (same input, same text) so it can be tested.

import type { DriftBranch, JournalEntry, Project, Task, Topic } from "../api/types";
import { TASK_STATUS_LABEL, TOPIC_STATUS_LABEL } from "./labels";

export interface ContextInput {
  project: Project;
  topic: Topic;
  /** Newest first (as `journalOf` returns them); the first five are used. */
  journal: JournalEntry[];
  /** The topic's tasks; open ones are listed. */
  tasks: Task[];
  /** Branches matched or linked to the topic (from the drift detector). */
  branches: DriftBranch[];
  /** One-hop neighbours on the map (kind label, title, project name, first sentence). */
  neighbours?: { kind: string; title: string; project: string; sentence: string }[];
}

const one = (s: string) => s.replace(/\s+/g, " ").trim();

export function buildTopicContext({ project, topic, journal, tasks, branches, neighbours = [] }: ContextInput): string {
  const out: string[] = [];
  out.push(`# ${project.name} / ${topic.title}`, "");
  out.push(`- Projekt: ${project.name} (\`${project.slug}\`)`);
  out.push(`- Temat: ${topic.title} (\`${topic.slug}\`)`);
  out.push(`- Status: ${TOPIC_STATUS_LABEL[topic.status]}`);
  if (topic.repo) out.push(`- Główne repozytorium: ${topic.repo}`);
  out.push("", "## Stan obecny", "", topic.summary.trim() || "(brak opisu stanu)");

  out.push("", "## Ostatnie wpisy w dzienniku", "");
  const entries = journal.slice(0, 5);
  if (entries.length === 0) out.push("(brak wpisów)");
  for (const j of entries) out.push(`- ${j.at.replace("T", " ")} · ${j.author}: ${one(j.text)}`);

  out.push("", "## Otwarte zadania", "");
  const open = tasks.filter((t) => t.status !== "done");
  if (open.length === 0) out.push("(brak otwartych zadań)");
  for (const t of open) {
    const due = t.due ? ` · termin ${t.due.replace("T", " ")}` : "";
    out.push(`- ^${t.id ?? "?"} ${t.title} (${TASK_STATUS_LABEL[t.status].toLowerCase()}${due})`);
  }

  out.push("", "## Repozytoria", "");
  if (project.repos.length === 0) out.push("(projekt nie ma repozytoriów)");
  for (const r of project.repos) out.push(`- ${r.name}: ${r.path}`);

  const sorted = [...branches].sort((a, b) => a.repo.localeCompare(b.repo) || a.branch.localeCompare(b.branch));
  if (sorted.length) {
    out.push("", "## Gałęzie tematu", "");
    for (const b of sorted) {
      const wt = b.worktree ? `, drzewo robocze ${b.worktree}` : "";
      const seen = b.seen ? `, widziana ${b.seen.replace("T", " ")}` : "";
      out.push(`- ${b.repo}: ${b.branch} (ostatni commit ${b.last_commit}${wt}${seen})`);
    }
  }

  if (neighbours.length) {
    out.push("", "## Połączone tematy", "");
    for (const n of neighbours) out.push(`- ${n.kind}: ${n.project} / ${n.title}${n.sentence ? ` — ${n.sentence}` : ""}`);
  }

  out.push(
    "",
    "---",
    `Zapisuj postęp w MontyBrain dla tego tematu (projekt \`${project.slug}\`, temat \`${topic.slug}\`): wpisy przez \`log_update\`, statusy zadań przez \`update_task\`, nowe zadania przez \`create_task\` (serwer MCP \`montybrain\`).`,
  );
  return `${out.join("\n")}\n`;
}

/** Copies `text`: the async clipboard API (works in the Tauri webview on a click), else a hidden textarea. */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // fall through
  }
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    return ok;
  } catch {
    return false;
  }
}
