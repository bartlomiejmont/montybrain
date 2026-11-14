import { describe, expect, it } from "vitest";

import { deadlineParts, deadlineToSave, fromDue, withDate, withTime } from "./deadline";
import { freshDraft, leaveDraft, showsConflict, syncDraft, withFocus } from "./draft";

// F3: the deadline (date + optional time) is one draft, so it gets the same
// conflict handling and hint as title/notes.
describe("deadline draft", () => {
  it("round-trips stored due values", () => {
    expect(fromDue(null)).toBe("");
    expect(fromDue("2026-11-20")).toBe("2026-11-20");
    expect(deadlineParts("2026-11-20T14:00")).toEqual({ date: "2026-11-20", time: "14:00" });
    expect(deadlineParts("")).toEqual({ date: "", time: "" });
    expect(deadlineToSave("2026-11-20T14:00")).toBe("2026-11-20T14:00");
    expect(deadlineToSave("2026-11-20")).toBe("2026-11-20");
    expect(deadlineToSave("")).toBeNull();
  });

  it("edits date and time as one value and keeps the time while the date is retyped", () => {
    let d = withTime("2026-11-20", "09:30");
    expect(d).toBe("2026-11-20T09:30");
    d = withDate(d, ""); // a date input reports "" while a new date is being typed
    expect(deadlineParts(d).time).toBe("09:30");
    expect(deadlineToSave(d)).toBeNull(); // no date = no deadline
    d = withDate(d, "2026-11-21");
    expect(d).toBe("2026-11-21T09:30");
    expect(withTime(d, "")).toBe("2026-11-21");
  });

  it("does not silently replace an external date when the user leaves the field", () => {
    // user is editing the date, an agent changes the deadline meanwhile
    const editing = { ...withFocus(freshDraft("2026-11-20"), true, "2026-11-20"), draft: "2026-11-22" };
    const s = syncDraft(editing, "2026-12-01T10:00");
    expect(s.draft).toBe("2026-11-22");
    expect(showsConflict(s, "2026-12-01T10:00")).toBe(true); // the hint is shown
    // leaving saves the user's own value (as the hint says), not a silent revert
    expect(leaveDraft(s, "2026-12-01T10:00").edited).toBe("2026-11-22");
  });

  it("does not write an untouched deadline back over an external change", () => {
    const focused = withFocus(freshDraft("2026-11-20"), true, "2026-11-20");
    const s = syncDraft(focused, "2026-12-01T10:00");
    const left = leaveDraft(s, "2026-12-01T10:00");
    expect(left.edited).toBeNull();
    expect(deadlineParts(left.state.draft)).toEqual({ date: "2026-12-01", time: "10:00" });
  });
});
