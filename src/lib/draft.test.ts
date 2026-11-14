import { describe, expect, it } from "vitest";

import {
  ackSave,
  adoptStored,
  changedElsewhere,
  discardDraft,
  freshDraft,
  isDirty,
  leaveDraft,
  showsConflict,
  requestSave,
  startSave,
  syncDraft,
  takeQueued,
  target,
  withFocus,
  type DraftState,
} from "./draft";

const typed = (s: DraftState, v: string): DraftState => ({ ...s, draft: v });

describe("draft preservation", () => {
  it("follows the file while the draft is clean and unfocused", () => {
    const s = syncDraft(freshDraft("a"), "b");
    expect(s.draft).toBe("b");
    expect(s.base).toBe("b");
  });

  it("keeps a dirty draft when the file changes and flags it", () => {
    const s = syncDraft(typed(freshDraft("notatki"), "notatki moje"), "notatki agenta");
    expect(s.draft).toBe("notatki moje");
    expect(showsConflict(s, "notatki agenta")).toBe(true);
    // the hint goes away when the file returns to what the user started from
    expect(changedElsewhere(s, "notatki")).toBe(false);
  });

  it("never overwrites a focused field, even before typing", () => {
    const focused = withFocus(freshDraft("a"), true, "a");
    const s = syncDraft(focused, "b");
    expect(s.draft).toBe("a");
    expect(showsConflict(s, "b")).toBe(true);
  });

  it("keeps further typing made after a save was sent", () => {
    const sent = startSave(freshDraft("a"), 1, "b");
    const s = ackSave(typed(sent, "b dalej"), 1, "b");
    expect(s.draft).toBe("b dalej");
    expect(s.base).toBe("b");
    expect(s.pending).toBeNull();
  });

  it("returns the same object when nothing changed", () => {
    const s = freshDraft("a");
    expect(syncDraft(s, "a")).toBe(s);
  });

  it("lets the user take the file's version", () => {
    expect(discardDraft("z").draft).toBe("z");
  });
});

// F1: leaving a field must save only what the user changed, judged against the
// value as reconciled at the moment of leaving (not a value captured earlier).
describe("leaving a field", () => {
  it("does not write back an untouched value over an external edit", () => {
    // focus notes containing A, an agent writes B, the user clicks elsewhere
    const focused = withFocus(freshDraft("A"), true, "A");
    const during = syncDraft(focused, "B");
    expect(during.draft).toBe("A"); // focused: shown text is kept
    const { state, edited } = leaveDraft(during, "B");
    expect(edited).toBeNull();
    expect(state.draft).toBe("B");
    expect(state.focused).toBe(false);
  });

  it("returns the user's text when they changed it, even after an external edit", () => {
    const s = syncDraft(typed(withFocus(freshDraft("A"), true, "A"), "A moje"), "B");
    expect(leaveDraft(s, "B").edited).toBe("A moje");
  });

  it("treats text changed back to the original as untouched", () => {
    const s = typed(typed(withFocus(freshDraft("A"), true, "A"), "Ax"), "A");
    expect(leaveDraft(s, "B").edited).toBeNull();
    expect(leaveDraft(s, "B").state.draft).toBe("B");
  });

  it("does not send the same text again while it is being saved", () => {
    const sent = startSave(typed(withFocus(freshDraft("A"), true, "A"), "B"), 1, "B");
    expect(leaveDraft(sent, "A").edited).toBeNull();
    expect(leaveDraft(typed(sent, "BC"), "A").edited).toBe("BC");
  });
});

// F2: a save is acknowledged by its own result (operation id), never by "the
// next snapshot that differs".
describe("save acknowledgement", () => {
  it("is acknowledged by the mutation result", () => {
    const sent = startSave(typed(freshDraft("A"), "B"), 1, "B");
    expect(sent.pending).toEqual({ op: 1, value: "B" });
    const s = ackSave(sent, 1, "B");
    expect(s).toMatchObject({ base: "B", draft: "B", pending: null });
    expect(isDirty(s)).toBe(false);
    expect(showsConflict(s, "B")).toBe(false);
  });

  it("does not take an external change during a pending save for its own", () => {
    const sent = startSave(typed(freshDraft("A"), "B"), 1, "B");
    const s = syncDraft(sent, "external C");
    expect(s.draft).toBe("B");
    expect(s.base).toBe("A");
    expect(s.pending).not.toBeNull();
  });

  it("keeps the draft and the hint when the save fails after an external change", () => {
    const sent = startSave(typed(freshDraft("A"), "B"), 1, "B");
    const failed = ackSave(syncDraft(sent, "external C"), 1, undefined);
    const s = syncDraft(failed, "external C");
    expect(s).toMatchObject({ base: "A", draft: "B", pending: null });
    expect(showsConflict(s, "external C")).toBe(true);
    // ... and leaving the field offers it for saving again
    expect(leaveDraft(s, "external C").edited).toBe("B");
  });

  it("follows an external change made after the save landed", () => {
    const sent = startSave(typed(freshDraft("A"), "B"), 1, "B");
    const acked = ackSave(syncDraft(sent, "external C"), 1, "B");
    const s = syncDraft(acked, "external C");
    expect(s.draft).toBe("external C");
    expect(showsConflict(s, "external C")).toBe(false);
  });

  it("accepts the backend normalising the saved value without a conflict", () => {
    const sent = startSave(typed(freshDraft("a"), "b  c"), 1, "b  c");
    // the refreshed snapshot (already normalised) may arrive before the result
    const before = syncDraft(sent, "b c");
    expect(before.draft).toBe("b  c");
    expect(showsConflict(before, "b c")).toBe(false);
    const s = syncDraft(ackSave(before, 1, "b c"), "b c");
    expect(s).toMatchObject({ base: "b c", draft: "b c", pending: null });
    expect(showsConflict(s, "b c")).toBe(false);
  });

  it("ignores the result of an older save once a newer one is pending", () => {
    const one = startSave(typed(freshDraft("A"), "B"), 1, "B");
    const two = startSave(typed(one, "BC"), 2, "BC");
    const afterOld = ackSave(two, 1, "B");
    expect(afterOld.pending).toEqual({ op: 2, value: "BC" });
    expect(afterOld.draft).toBe("BC");
    expect(afterOld.base).toBe("B");
    // a failure of the older save does not cancel the newer one either
    expect(ackSave(two, 1, undefined).pending).toEqual({ op: 2, value: "BC" });
    // and a late result of an older save never moves the base backwards
    const done = ackSave(afterOld, 2, "BC");
    expect(ackSave(done, 1, "B").base).toBe("BC");
  });
});

/**
 * Drives one field the way the store does: `leave` = the user leaves the field
 * (autosave), `ack` = the result of the save in flight. Returns the writes sent.
 */
function field(initial: string) {
  let s = withFocus(freshDraft(initial), true, initial);
  let disk = initial;
  let op = 0;
  const sent: string[] = [];
  const send = (v: string) => {
    s = startSave(s, ++op, v);
    sent.push(v);
  };
  const api = {
    get state() {
      return s;
    },
    sent,
    type(v: string) {
      s = withFocus(typed(s, v), true, disk);
      return api;
    },
    leave() {
      const out = leaveDraft(s, disk);
      s = out.state;
      if (out.edited !== null) {
        const r = requestSave(s, out.edited);
        s = r.state;
        if (r.send) send(out.edited);
      }
      return api;
    },
    /** The save in flight landed (`ok`) or failed. */
    ack(ok = true) {
      const p = s.pending!;
      if (ok) disk = p.value;
      const r = takeQueued(syncDraft(ackSave(s, p.op, ok ? p.value : undefined), disk));
      s = r.state;
      if (r.send !== null) send(r.send);
      s = syncDraft(s, disk);
      return api;
    },
    get disk() {
      return disk;
    },
  };
  return api;
}

// N2: intent is judged against what will be on disk once the pending write
// lands, and a change made while a save is in flight is saved after it.
describe("changes made while a save is in flight", () => {
  it("saves a reversal to the original value made before the acknowledgement", () => {
    const f = field("A").type("B").leave();
    expect(f.sent).toEqual(["B"]);
    f.type("A").leave(); // B still pending: A must not count as "unchanged"
    expect(f.sent).toEqual(["B"]); // at most one write in flight
    expect(target(f.state)).toBe("A");
    f.ack();
    expect(f.sent).toEqual(["B", "A"]);
    f.ack();
    expect(f.disk).toBe("A");
    expect(f.state).toMatchObject({ base: "A", draft: "A", pending: null, queued: null });
    expect(isDirty(f.state)).toBe(false);
  });

  it("judges leaving against the pending value, not the stored base", () => {
    const pendingB = startSave(typed(freshDraft("A"), "B"), 1, "B");
    expect(leaveDraft(typed(pendingB, "A"), "A").edited).toBe("A");
    expect(leaveDraft(typed(pendingB, "A"), "B").edited).toBe("A");
  });

  it("writes text typed during a pending save after it, when the user leaves before the result", () => {
    const f = field("A").type("B").leave().type("C").leave();
    expect(f.sent).toEqual(["B"]);
    f.ack();
    expect(f.sent).toEqual(["B", "C"]);
    f.ack();
    expect(f.disk).toBe("C");
    expect(f.state.draft).toBe("C");
  });

  it("keeps text typed during a pending save and saves it when the user leaves after the result", () => {
    const f = field("A").type("B").leave().type("C");
    f.ack();
    expect(f.state).toMatchObject({ base: "B", draft: "C", pending: null });
    expect(f.sent).toEqual(["B"]);
    f.leave();
    expect(f.sent).toEqual(["B", "C"]);
    f.ack();
    expect(f.disk).toBe("C");
  });

  it("does not overwrite typing that continues after a queued value was taken", () => {
    const f = field("A").type("B").leave().type("C").leave().type("C2"); // focused again, still typing
    f.ack(); // B landed, C goes out
    expect(f.sent).toEqual(["B", "C"]);
    expect(f.state.draft).toBe("C2");
    f.ack();
    expect(f.state).toMatchObject({ base: "C", draft: "C2" });
    expect(isDirty(f.state)).toBe(true);
  });

  it("collapses rapid successive edits to the last value", () => {
    const f = field("A").type("B").leave().type("C").leave().type("D").leave().type("E").leave();
    expect(f.sent).toEqual(["B"]);
    f.ack();
    expect(f.sent).toEqual(["B", "E"]);
    f.ack();
    expect(f.disk).toBe("E");
  });

  it("drops a queued value when the user returns to the pending one", () => {
    const f = field("A").type("B").leave().type("C").leave().type("B").leave();
    expect(f.state.queued).toBeNull();
    f.ack();
    expect(f.sent).toEqual(["B"]);
    expect(f.state).toMatchObject({ base: "B", draft: "B", pending: null });
  });

  it("keeps the draft (dirty, no hint) when the pending save fails", () => {
    const f = field("A").type("B").leave();
    f.ack(false);
    expect(f.disk).toBe("A");
    expect(f.state).toMatchObject({ base: "A", draft: "B", pending: null, queued: null });
    expect(isDirty(f.state)).toBe(true);
    expect(showsConflict(f.state, "A")).toBe(false); // nothing changed elsewhere
    // leaving again offers it for saving again
    f.leave();
    expect(f.sent).toEqual(["B", "B"]);
  });

  it("still writes the latest value when the save before it fails", () => {
    const f = field("A").type("B").leave().type("C").leave();
    f.ack(false);
    expect(f.sent).toEqual(["B", "C"]);
    f.ack();
    expect(f.disk).toBe("C");
  });

  it("does not write a queued reversal that the failed save made unnecessary", () => {
    const f = field("A").type("B").leave().type("A").leave();
    f.ack(false); // disk still A, the queued A is already there
    expect(f.sent).toEqual(["B"]);
    expect(f.state).toMatchObject({ base: "A", draft: "A", pending: null, queued: null });
  });

  it("requestSave does not send a value that is already what will be stored", () => {
    const s = typed(freshDraft("A"), "A ");
    const r = requestSave(s, "A"); // normalised by the caller (trim)
    expect(r.send).toBe(false);
    expect(r.state.draft).toBe("A");
    expect(requestSave(typed(freshDraft("A"), "B"), "B").send).toBe(true);
  });

  it("taking the stored value during a pending save takes the pending value", () => {
    const f = field("A").type("B").leave().type("C").leave();
    const s = adoptStored(f.state, "A", false);
    expect(s).toMatchObject({ draft: "B", queued: null, pending: { value: "B" } });
    // without a save in flight it is the file's version
    expect(adoptStored(typed(freshDraft("A"), "x"), "Z", true)).toMatchObject({ base: "Z", draft: "Z", focused: true });
  });
});
