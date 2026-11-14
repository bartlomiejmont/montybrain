import { describe, expect, it } from "vitest";

import { describeUndo, isUndoKey } from "./undo";

const report = (statuses: ("undone" | "changed" | "failed")[], retry: number | null = null) => ({
  reading: "Nowe zadanie „x”",
  source: "quick",
  items: statuses.map((status) => ({ status, message: "" })),
  retry,
});

describe("undo", () => {
  it("a failed undo stays open and offers a retry with the token", () => {
    const v = describeUndo(report(["undone", "failed"], 7), 3);
    expect(v).toMatchObject({ failed: true, retry: 7 });
    // Without a retry token from the backend, the original one.
    expect(describeUndo(report(["failed"]), 3).retry).toBe(3);
  });

  it("a complete or changed undo closes", () => {
    expect(describeUndo(report(["undone"]), 3)).toEqual({ text: "Cofnięto: Nowe zadanie „x”", retry: null, failed: false });
    expect(describeUndo(report(["undone", "changed"]), 3)).toMatchObject({ failed: false, retry: null });
  });

  it("`u` works at the view level, not while typing or with a dialog open", () => {
    const k = { key: "u", metaKey: false, ctrlKey: false, altKey: false, defaultPrevented: false };
    expect(isUndoKey(k, false, false)).toBe(true);
    expect(isUndoKey(k, true, false)).toBe(false);
    expect(isUndoKey(k, false, true)).toBe(false);
    expect(isUndoKey({ ...k, metaKey: true }, false, false)).toBe(false);
    expect(isUndoKey({ ...k, key: "U" }, false, false)).toBe(false);
  });
});
