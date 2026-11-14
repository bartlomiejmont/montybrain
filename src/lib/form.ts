// Explicit-save editors (topic, project): what "Zapisz" sends and when the
// editor may close. Pure, so the decisions are testable without a DOM.
//
// Behaviour while a save is in flight: the fields stay editable, a second
// submit is ignored (the button shows "Zapisywanie…"), and the editor closes —
// dropping its drafts — only when the request succeeded and every field is
// settled (nothing typed after the submit, nothing in flight). Otherwise it
// stays open with the user's text.

import type { ProjectPatch, ProjectState, Repo, TopicPatch, TopicStatus } from "../api/types";
import { isSettled, type DraftState } from "./draft";

/** One field: the user's value when they changed it (`DraftHandle.edited`), and what will be stored (`stored`). */
export interface FieldEdit {
  edited: string | null;
  stored: string;
}

const trim = (s: string) => s.trim();

/** The user's value when it differs from what will be stored (compared after `norm`), else null. */
export function changedValue(f: FieldEdit, norm: (s: string) => string = (s) => s): string | null {
  return f.edited !== null && norm(f.edited) !== norm(f.stored) ? f.edited : null;
}

/** `f` called at most once; later calls return the first result (one request shared by several fields). */
export function once<T>(f: () => T): () => T {
  let done = false;
  let out: T;
  return () => {
    if (!done) {
      out = f();
      done = true;
    }
    return out;
  };
}

export type SubmitAction = "ignore" | "close" | "send";

/** Pressing "Zapisz": ignored while a submission is in flight, closes when nothing changed, else sends. */
export function submitAction(busy: boolean, patch: object): SubmitAction {
  if (busy) return "ignore";
  return Object.keys(patch).length === 0 ? "close" : "send";
}

/** After the request settled: close only when it succeeded and nothing is unsaved or in flight. */
export function closeAfterSave(ok: boolean, fields: DraftState[]): boolean {
  return ok && fields.every(isSettled);
}

/**
 * The text of a submit-on-Enter input after its submission settled: cleared on
 * success only while it still is the submitted text (`ok` false keeps it).
 */
export function clearAfterSubmit(ok: boolean, current: string, submitted: string): string {
  return ok && current === submitted ? "" : current;
}

export type TitleCommit = { kind: "keep" } | { kind: "save"; value: string } | { kind: "revert" };

/**
 * Ending a title edit (inline rename or the detail field). `edited` is what
 * `DraftHandle.leave()` returned: null means the text is already what will be
 * stored — the draft and any write in flight or queued must be left alone
 * (reverting here would cancel a queued rename). An emptied title goes back to
 * the stored one; anything else is saved.
 */
export function titleCommit(edited: string | null): TitleCommit {
  if (edited === null) return { kind: "keep" };
  const value = edited.trim();
  return value ? { kind: "save", value } : { kind: "revert" };
}

export function topicPatch(f: { title: FieldEdit; status: FieldEdit; repo: FieldEdit; summary: FieldEdit }): TopicPatch {
  const title = changedValue(f.title, trim);
  const status = changedValue(f.status) as TopicStatus | null;
  const repo = changedValue(f.repo);
  const summary = changedValue(f.summary, trim);
  return {
    ...(title !== null ? { title } : {}),
    ...(status !== null ? { status } : {}),
    ...(repo !== null ? { repo: repo || null } : {}),
    ...(summary !== null ? { summary } : {}),
  };
}

const HEX_RE = /^#[0-9a-f]{6}$/i;

const cleanRepos = (json: string): Repo[] =>
  (JSON.parse(json) as Repo[]).map((r) => ({ name: r.name.trim(), path: r.path.trim() })).filter((r) => r.name || r.path);

export function projectPatch(f: {
  name: FieldEdit;
  color: FieldEdit;
  state: FieldEdit;
  description: FieldEdit;
  repos: FieldEdit;
}): { patch: ProjectPatch } | { error: string } {
  const name = changedValue(f.name, trim);
  const color = changedValue(f.color);
  const state = changedValue(f.state) as ProjectState | null;
  const description = changedValue(f.description, trim);
  if (color && !HEX_RE.test(color)) return { error: "Kolor podaj jako #rrggbb, np. #a3e635" };
  let repos: Repo[] | null = null;
  if (f.repos.edited !== null) {
    repos = cleanRepos(f.repos.edited);
    if (repos.some((r) => !r.name)) return { error: "Każde repozytorium potrzebuje nazwy." };
    if (JSON.stringify(repos) === JSON.stringify(cleanRepos(f.repos.stored))) repos = null;
  }
  return {
    patch: {
      ...(name !== null ? { name } : {}),
      ...(color !== null ? { color: color || null } : {}),
      ...(state !== null ? { state } : {}),
      ...(description !== null ? { description } : {}),
      ...(repos !== null ? { repos } : {}),
    },
  };
}
