import { describe, expect, it } from "vitest";

import { createMockApi, MOCK_TAKEN_SHORTCUTS } from "./mock";
import { canonicalShortcut, prettyShortcut, shortcutFromEvent, shortcutKeys } from "./shortcut";

const key = (code: string, mods: Partial<{ ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }> = {}) => ({
  code,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

describe("shortcut strings", () => {
  it("builds from key events", () => {
    expect(shortcutFromEvent(key("Space", { ctrlKey: true, altKey: true }))).toBe("Ctrl+Alt+Space");
    expect(shortcutFromEvent(key("KeyK", { metaKey: true, shiftKey: true }))).toBe("Shift+Cmd+K");
    expect(shortcutFromEvent(key("Digit1", { altKey: true }))).toBe("Alt+1");
    expect(shortcutFromEvent(key("AltLeft", { altKey: true }))).toBeNull();
    expect(shortcutFromEvent(key("MetaRight", { metaKey: true }))).toBeNull();
  });

  it("normalizes like the backend", () => {
    expect(canonicalShortcut("alt+control+space")).toBe("Ctrl+Alt+Space");
    expect(canonicalShortcut("Shift+Cmd+KeyK")).toBe("Shift+Cmd+K");
    expect(canonicalShortcut("F5")).toBe("F5");
    expect(() => canonicalShortcut("K")).toThrow(/⌃, ⌥ lub ⌘/);
    expect(() => canonicalShortcut("Shift+K")).toThrow();
    expect(() => canonicalShortcut("Ctrl+Alt")).toThrow();
    expect(() => canonicalShortcut("Ctrl+NoSuchKey")).toThrow();
  });

  it("pretty-prints", () => {
    expect(prettyShortcut("Ctrl+Alt+Space")).toBe("⌃⌥Spacja");
    expect(shortcutKeys("Shift+Cmd+K")).toEqual(["⇧", "⌘", "K"]);
  });
});

describe("mock settings", () => {
  it("round-trips settings and reports taken shortcuts", async () => {
    const api = createMockApi();
    const seen: string[] = [];
    await api.onSettingsChange((s) => seen.push(s.shortcut));
    const s0 = await api.getSettings();
    expect(s0.shortcut).toBe("Ctrl+Alt+Space");
    expect(s0.reminders).toBe(true);
    expect(s0.digest_hour).toBe(9);
    expect(s0.quick_destination).toEqual({ kind: "inbox" });

    const s1 = await api.setShortcut("cmd+shift+keyk");
    expect(s1.shortcut).toBe("Shift+Cmd+K");
    await expect(api.setShortcut(MOCK_TAKEN_SHORTCUTS[0]!)).rejects.toThrow(/zajęty/);
    expect((await api.getSettings()).shortcut).toBe("Shift+Cmd+K");

    await api.updateSettings({ digest_hour: 7, reminders: false });
    await expect(api.updateSettings({ digest_hour: 24 })).rejects.toThrow();
    await expect(api.updateSettings({ quick_destination: { kind: "project", project: "nie-ma" } })).rejects.toThrow();
    const s2 = await api.setAutostart(true);
    expect(s2).toMatchObject({ digest_hour: 7, reminders: false, autostart: true });

    await new Promise((r) => setTimeout(r, 0));
    expect(seen).toContain("Shift+Cmd+K");
  });
});
