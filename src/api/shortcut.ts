import { t as tr } from "../i18n";
// Global shortcut strings, mirroring src-tauri/src/shortcut.rs.
// Stored form: "Ctrl+Alt+Shift+Cmd" modifiers in that order + one key named
// like `KeyboardEvent.code` without the "Key"/"Digit" prefix ("Ctrl+Alt+Space").

export const DEFAULT_SHORTCUT = "Ctrl+Alt+Space";

const MODIFIERS: [string, string[]][] = [
  ["Ctrl", ["CTRL", "CONTROL"]],
  ["Alt", ["ALT", "OPTION"]],
  ["Shift", ["SHIFT"]],
  ["Cmd", ["CMD", "COMMAND", "SUPER", "META"]],
];

const MODIFIER_CODES = /^(Control|Alt|Shift|Meta|OS|Fn|CapsLock)(Left|Right)?$/;

/** Keys the backend parser knows besides letters, digits and F-keys. */
const NAMED_KEYS = new Set([
  "Space", "Enter", "Tab", "Backspace", "Delete", "Escape", "Home", "End", "PageUp", "PageDown",
  "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Backquote", "Backslash", "BracketLeft",
  "BracketRight", "Comma", "Equal", "Minus", "Period", "Quote", "Semicolon", "Slash", "Insert",
]);

export interface KeyLike {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/** Builds a shortcut from a keydown; null while only modifiers are held. */
export function shortcutFromEvent(e: KeyLike): string | null {
  if (!e.code || MODIFIER_CODES.test(e.code)) return null;
  const key = e.code.replace(/^Key([A-Z])$/, "$1").replace(/^Digit(\d)$/, "$1");
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Cmd");
  parts.push(key);
  return parts.join("+");
}

/** Validates + normalizes like the backend; throws with a Polish message. */
export function canonicalShortcut(input: string): string {
  const bad = () => new Error(tr("Nieprawidłowy skrót: {0}", input));
  const tokens = input.split("+").map((t) => t.trim());
  const rawKey = tokens.pop();
  if (!rawKey) throw bad();
  const on = [false, false, false, false];
  for (const m of tokens) {
    const i = MODIFIERS.findIndex(([, names]) => names.includes(m.toUpperCase()));
    if (i < 0) throw bad();
    on[i] = true;
  }
  const bare = rawKey.replace(/^key([a-z])$/i, "$1").replace(/^digit(\d)$/i, "$1");
  const isFn = /^f\d{1,2}$/i.test(bare);
  const named = [...NAMED_KEYS].find((n) => n.toLowerCase() === bare.toLowerCase());
  const key = /^[a-z0-9]$/i.test(bare) || isFn ? bare.toUpperCase() : named;
  if (!key) throw bad();
  if (!(on[0] || on[1] || on[3] || isFn)) throw new Error(tr("Skrót musi zawierać ⌃, ⌥ lub ⌘."));
  const mods = MODIFIERS.filter((_, i) => on[i]).map(([name]) => name);
  return [...mods, key].join("+");
}

const SYMBOLS: Record<string, string> = {
  Ctrl: "⌃",
  Alt: "⌥",
  Shift: "⇧",
  Cmd: "⌘",
  get Space() { return tr("Spacja"); },
  Enter: "↩",
  Tab: "⇥",
  Backspace: "⌫",
  Delete: "⌦",
  Escape: "Esc",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Backquote: "`",
  Backslash: "\\",
  BracketLeft: "[",
  BracketRight: "]",
  Comma: ",",
  Equal: "=",
  Minus: "-",
  Period: ".",
  Quote: "'",
  Semicolon: ";",
  Slash: "/",
};

/** "Ctrl+Alt+Space" → ["⌃", "⌥", "Spacja"] (one entry per key cap). */
export function shortcutKeys(shortcut: string): string[] {
  return shortcut
    .split("+")
    .filter(Boolean)
    .map((t) => SYMBOLS[t] ?? t);
}

/** "Ctrl+Alt+Space" → "⌃⌥Spacja". */
export function prettyShortcut(shortcut: string): string {
  return shortcutKeys(shortcut).join("");
}
