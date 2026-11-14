import { describe, expect, it } from "vitest";

import { ackSave, cancelEdit, freshDraft, leaveDraft, requestSave, startSave, takeQueued, type DraftState } from "./draft";
import { clearAfterSubmit, closeAfterSave, projectPatch, submitAction, titleCommit, topicPatch } from "./form";

const typed = (s: DraftState, v: string): DraftState => ({ ...s, draft: v });
const same = (v: string) => ({ edited: null, stored: v });
const edit = (v: string, stored: string) => ({ edited: v, stored });

/** A field of an explicit form sent as operation `op` with value `v`. */
const sending = (base: string, v: string, op = 1) => startSave(requestSave(typed(freshDraft(base), v), v).state, op, v);

// N1: "Zapisz" in the topic / project editors.
describe("explicit-save form: submit", () => {
  it("ignores a second submit while the first is in flight", () => {
    // the first request is pending; nothing about the drafts may be decided now
    expect(submitAction(true, { title: "B" })).toBe("ignore");
    expect(submitAction(true, {})).toBe("ignore"); // in particular: no close with an empty patch
  });

  it("closes without a request when nothing changed, sends otherwise", () => {
    expect(submitAction(false, {})).toBe("close");
    expect(submitAction(false, { title: "B" })).toBe("send");
  });
});

describe("explicit-save form: close after the request", () => {
  it("closes when it succeeded and nothing was typed meanwhile", () => {
    const title = ackSave(sending("A", "B"), 1, "B");
    const summary = freshDraft("s");
    expect(closeAfterSave(true, [title, summary])).toBe(true);
  });

  it("stays open, keeping the text, when the request failed", () => {
    const title = ackSave(sending("A", "B"), 1, undefined);
    expect(title.draft).toBe("B");
    expect(closeAfterSave(false, [title])).toBe(false);
  });

  it("stays open when the user typed into a saved field while it was saving", () => {
    const title = ackSave(typed(sending("A", "B"), "C"), 1, "B");
    expect(title).toMatchObject({ base: "B", draft: "C" });
    expect(closeAfterSave(true, [title])).toBe(false);
  });

  it("stays open when the user typed into another field while saving", () => {
    const title = ackSave(sending("A", "B"), 1, "B");
    expect(closeAfterSave(true, [title, typed(freshDraft("s"), "s2")])).toBe(false);
  });

  it("closes when the saved value was normalised by the backend", () => {
    expect(closeAfterSave(true, [ackSave(sending("A", "B  "), 1, "B")])).toBe(true);
  });
});

describe("topic patch", () => {
  const unchanged = { title: same("T"), status: same("active"), repo: same(""), summary: same("S") };

  it("is empty when nothing changed", () => {
    expect(topicPatch(unchanged)).toEqual({});
  });

  it("sends only the fields the user changed, compared with what will be stored", () => {
    expect(topicPatch({ ...unchanged, title: edit("B", "T"), repo: edit("", "api") })).toEqual({ title: "B", repo: null });
    // whitespace-only changes are not changes
    expect(topicPatch({ ...unchanged, summary: edit("S  ", "S") })).toEqual({});
  });

  it("sends a reversal to the value the snapshot still shows", () => {
    // B was stored by the previous submit; the snapshot may still say A.
    expect(topicPatch({ ...unchanged, title: edit("A", "B") })).toEqual({ title: "A" });
  });
});

describe("project patch", () => {
  const unchanged = {
    name: same("P"),
    color: same("#aabbcc"),
    state: same("active"),
    description: same("d"),
    repos: same(JSON.stringify([{ name: "api", path: "/x" }])),
  };

  it("is empty when nothing changed", () => {
    expect(projectPatch(unchanged)).toEqual({ patch: {} });
  });

  it("validates the colour and repositories", () => {
    expect(projectPatch({ ...unchanged, color: edit("zzz", "#aabbcc") })).toHaveProperty("error");
    expect(projectPatch({ ...unchanged, repos: edit(JSON.stringify([{ name: "", path: "/y" }]), "[]") })).toHaveProperty("error");
  });

  it("sends changed fields only, normalising repositories", () => {
    const repos = edit(JSON.stringify([{ name: " api ", path: " /x " }, { name: "", path: "" }]), unchanged.repos.stored);
    expect(projectPatch({ ...unchanged, repos })).toEqual({ patch: {} });
    expect(projectPatch({ ...unchanged, color: edit("", "#aabbcc"), name: edit("Q", "P") })).toEqual({
      patch: { name: "Q", color: null },
    });
  });
});

// Submit-on-Enter inputs (new task, new topic, link, journal entry): the text is
// cleared after a successful submit only when it is still the submitted text.
describe("ending a title edit", () => {
  it("saves a changed title trimmed, reverts an emptied one", () => {
    expect(titleCommit("  Nowy tytuł ")).toEqual({ kind: "save", value: "Nowy tytuł" });
    expect(titleCommit("   ")).toEqual({ kind: "revert" });
  });

  it("leaves a queued rename alone when the field is confirmed unchanged", () => {
    // A stored, B in flight, C queued; the rename is reopened and confirmed without typing.
    let s = startSave(requestSave(freshDraft("A"), "B").state, 1, "B");
    s = requestSave({ ...s, draft: "C" }, "C").state;
    expect(s).toMatchObject({ pending: { value: "B" }, queued: "C" });

    const left = leaveDraft({ ...s, focused: true }, "A");
    expect(left.edited).toBeNull();
    expect(titleCommit(left.edited)).toEqual({ kind: "keep" });
    // "keep" touches nothing: C is still queued and is what gets written after B.
    const afterB = takeQueued(ackSave(left.state, 1, "B"));
    expect(afterB.send).toBe("C");
  });
});

describe("cancelling a title edit", () => {
  // A stored, B in flight, C queued.
  const queuedC = () => {
    const s = startSave(requestSave(freshDraft("A"), "B").state, 1, "B");
    return requestSave({ ...s, draft: "C" }, "C").state;
  };

  it("Escape keeps a rename submitted earlier", () => {
    const s = cancelEdit({ ...queuedC(), focused: true }, "A", false);
    expect(s).toMatchObject({ draft: "C", pending: { value: "B" }, queued: "C", focused: false });
    expect(takeQueued(ackSave(s, 1, "B")).send).toBe("C");
  });

  it("an emptied title goes back to what will be stored, not to the value in flight", () => {
    const emptied = { ...queuedC(), draft: "   ", focused: true };
    const left = leaveDraft(emptied, "A");
    expect(titleCommit(left.edited)).toEqual({ kind: "revert" });
    const s = cancelEdit(left.state, "A", false);
    expect(s).toMatchObject({ draft: "C", queued: "C" });
    expect(takeQueued(ackSave(s, 1, "B")).send).toBe("C");
  });

  it("drops only the typed text when a single save is in flight", () => {
    const pendingB = startSave(requestSave(freshDraft("A"), "B").state, 1, "B");
    const s = cancelEdit({ ...pendingB, draft: "typed" }, "A", false);
    expect(s).toMatchObject({ draft: "B", pending: { value: "B" }, queued: null });
  });

  it("takes the file's value when nothing is in flight", () => {
    expect(cancelEdit({ ...freshDraft("A"), draft: "typed" }, "Z", false)).toMatchObject({ base: "Z", draft: "Z", pending: null });
  });
});

describe("submit-on-Enter input", () => {
  it("clears the submitted text on success, keeps it on failure", () => {
    expect(clearAfterSubmit(true, "https://a", "https://a")).toBe("");
    expect(clearAfterSubmit(false, "https://a", "https://a")).toBe("https://a");
  });

  it("keeps text typed after the submit (e.g. in a re-created field)", () => {
    expect(clearAfterSubmit(true, "https://b", "https://a")).toBe("https://b");
  });
});
