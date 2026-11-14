import { describe, expect, it } from "vitest";

import { freshDraft, type DraftState } from "../lib/draft";
import { REMOUNT_WINDOW_MS, createDraftStore } from "./drafts";

const dirty = (v: string) => ({ ...freshDraft(""), draft: v });

describe("draft store cleanup (F6)", () => {
  it("frees a clean draft as soon as its field goes away", () => {
    const store = createDraftStore(() => 0);
    store.put("a", freshDraft("x"));
    store.left("a");
    expect(store.size()).toBe(0);
  });

  it("keeps an abandoned dirty draft for a remount, then sweeps it", () => {
    let now = 1000;
    const store = createDraftStore(() => now);
    store.put("a", dirty("typed"));
    store.left("a");
    expect(store.get("a")?.draft).toBe("typed");
    now += REMOUNT_WINDOW_MS + 1;
    expect(store.get("a")).toBeUndefined(); // not reused
    store.sweep();
    expect(store.size()).toBe(0);
    expect(store.leftCount()).toBe(0);
  });

  it("does not sweep a draft whose field came back", () => {
    let now = 0;
    const store = createDraftStore(() => now);
    store.put("a", dirty("typed"));
    store.left("a");
    store.entered("a");
    now += REMOUNT_WINDOW_MS * 10;
    store.sweep();
    expect(store.get("a")?.draft).toBe("typed");
  });

  it("dropDrafts clears both the drafts and their departure stamps", () => {
    const store = createDraftStore(() => 0);
    store.put("te:p/t:title", dirty("a"));
    store.put("te:p/t:summary", dirty("b"));
    store.put("other", dirty("c"));
    store.left("te:p/t:title");
    store.left("other");
    store.drop("te:p/t:");
    expect(store.size()).toBe(1);
    expect(store.leftCount()).toBe(1);
  });

  it("keeps a draft while another field with the same key is still mounted", () => {
    // the task title is edited both in the row (rename) and in the detail panel
    let now = 0;
    const store = createDraftStore(() => now);
    store.entered("task:x:title");
    store.entered("task:x:title");
    store.put("task:x:title", dirty("typed in the row"));
    store.left("task:x:title"); // the detail panel collapses
    now += REMOUNT_WINDOW_MS * 10;
    store.sweep();
    expect(store.get("task:x:title")?.draft).toBe("typed in the row");
    store.left("task:x:title"); // the row goes too: now it is abandoned
    now += REMOUNT_WINDOW_MS + 1;
    store.sweep();
    expect(store.size()).toBe(0);
  });

  it("schedules a sweep when a dirty draft is abandoned", () => {
    let now = 0;
    const timers: (() => void)[] = [];
    const store = createDraftStore(
      () => now,
      (f) => void timers.push(f),
    );
    store.put("a", dirty("typed"));
    store.left("a");
    expect(timers.length).toBe(1);
    now += REMOUNT_WINDOW_MS + 1;
    timers.shift()!();
    expect(store.size()).toBe(0);
    expect(timers.length).toBe(0); // nothing left to watch
  });
});

/** A backend call held open until the test settles it. */
function deferred() {
  const calls: { value: string; resolve: (w: string | undefined) => void; reject: (e: unknown) => void }[] = [];
  const send = (value: string) => () =>
    new Promise<string | undefined>((resolve, reject) => calls.push({ value, resolve, reject }));
  const flush = () => new Promise((r) => setTimeout(r, 0));
  return { calls, send, flush };
}

const editing = (base: string, draft: string): DraftState => ({ ...freshDraft(base), draft, focused: true });

// N2: one write per field in flight; what the user asked for meanwhile goes out after it.
describe("draft store save serialisation (N2)", () => {
  it("saves a reversal made while the previous save is pending", async () => {
    const store = createDraftStore(() => 0);
    const { calls, send, flush } = deferred();
    store.save("f", editing("A", "B"), "B", send("B"));
    expect(calls.map((c) => c.value)).toEqual(["B"]);
    store.save("f", { ...store.get("f")!, draft: "A" }, "A", send("A"));
    expect(calls.map((c) => c.value)).toEqual(["B"]); // not sent concurrently
    calls[0]!.resolve("B");
    await flush();
    expect(calls.map((c) => c.value)).toEqual(["B", "A"]);
    calls[1]!.resolve("A");
    await flush();
    expect(store.get("f")).toMatchObject({ base: "A", draft: "A", pending: null, queued: null });
  });

  it("resolves save() once that write's result is applied (forms decide on it)", async () => {
    const store = createDraftStore(() => 0);
    const { calls, send } = deferred();
    // like useDraft: the field's value is mapped from the call's result
    let result!: Promise<string>;
    const done = store.save("f", editing("A", "B"), "B", () => (result = Promise.resolve("B")).then((w) => w));
    expect(done).toBeInstanceOf(Promise);
    await result; // the caller awaiting the request itself is not enough
    expect(store.get("f")?.pending).not.toBeNull();
    await done;
    expect(store.get("f")).toMatchObject({ base: "B", draft: "B", pending: null });
    // nothing to send: resolves at once
    await store.save("f", store.get("f")!, "B", send("B"));
    expect(calls).toHaveLength(0);
  });

  it("collapses edits made during a pending save to the last one", async () => {
    const store = createDraftStore(() => 0);
    const { calls, send, flush } = deferred();
    store.save("f", editing("A", "B"), "B", send("B"));
    for (const v of ["C", "D", "E"]) store.save("f", { ...store.get("f")!, draft: v }, v, send(v));
    calls[0]!.resolve("B");
    await flush();
    expect(calls.map((c) => c.value)).toEqual(["B", "E"]);
  });

  it("sends the queued value after a failed save, and keeps the draft when nothing is queued", async () => {
    const store = createDraftStore(() => 0);
    const { calls, send, flush } = deferred();
    store.save("f", editing("A", "B"), "B", send("B"));
    calls[0]!.reject(new Error("io"));
    await flush();
    expect(store.get("f")).toMatchObject({ base: "A", draft: "B", pending: null });
    store.save("f", store.get("f")!, "B", send("B"));
    store.save("f", { ...store.get("f")!, draft: "C" }, "C", send("C"));
    calls[1]!.resolve(undefined); // `run` reports a failure as undefined
    await flush();
    expect(calls.map((c) => c.value)).toEqual(["B", "B", "C"]);
  });

  it("forgets the queued write when the editor's drafts are dropped", async () => {
    const store = createDraftStore(() => 0);
    const { calls, send, flush } = deferred();
    store.save("te:x:title", editing("A", "B"), "B", send("B"));
    store.save("te:x:title", { ...store.get("te:x:title")!, draft: "C" }, "C", send("C"));
    store.drop("te:x:");
    calls[0]!.resolve("B");
    await flush();
    expect(calls).toHaveLength(1);
    expect(store.size()).toBe(0);
  });

  it("keeps an abandoned field with a save in flight past the remount window", async () => {
    let now = 0;
    const store = createDraftStore(() => now);
    const { calls, send, flush } = deferred();
    store.save("f", editing("A", "B"), "B", send("B"));
    store.save("f", { ...store.get("f")!, draft: "C" }, "C", send("C"));
    store.left("f");
    now += REMOUNT_WINDOW_MS * 5;
    store.sweep();
    expect(store.get("f")?.pending?.value).toBe("B");
    calls[0]!.resolve("B");
    await flush();
    expect(calls.map((c) => c.value)).toEqual(["B", "C"]); // still written
    calls[1]!.resolve("C");
    await flush();
    store.sweep();
    expect(store.size()).toBe(0); // and freed once nothing is in flight
  });
});

// N1: an explicit-save form submits at most once at a time.
describe("draft store submit guard (N1)", () => {
  it("lets only one submission of a form run at a time", () => {
    const store = createDraftStore(() => 0);
    expect(store.isBusy("te:p/t:")).toBe(false);
    expect(store.begin("te:p/t:")).toBe(true);
    expect(store.begin("te:p/t:")).toBe(false);
    expect(store.isBusy("te:p/t:")).toBe(true);
    store.end("te:p/t:");
    expect(store.begin("te:p/t:")).toBe(true);
  });
});
