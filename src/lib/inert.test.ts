import { describe, expect, it } from "vitest";
import { lockInert } from "./inert";

describe("overlapping dialog backgrounds", () => {
  it("unlocks only after both dialogs release, in either order", () => {
    for (const reverse of [false, true]) {
      const el = { inert: false } as HTMLElement;
      const releases = [lockInert([el]), lockInert([el])];
      if (reverse) releases.reverse();
      releases[0]!();
      expect(el.inert).toBe(true);
      releases[1]!();
      expect(el.inert).toBe(false);
    }
  });
  it("preserves a background that was already inert", () => {
    const el = { inert: true } as HTMLElement;
    lockInert([el])();
    expect(el.inert).toBe(true);
  });
});
