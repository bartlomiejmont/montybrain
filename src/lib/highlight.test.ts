import { describe, expect, it } from "vitest";

import { fragment, highlight, matchRanges, searchTerms } from "./highlight";

const joined = (segs: { text: string; hit: boolean }[]) => segs.filter((s) => s.hit).map((s) => s.text);

describe("search highlight", () => {
  it("matches case- and diacritics-insensitively, returning the original text", () => {
    const segs = highlight("Zażółć gęślą jaźń", searchTerms("ZAZOLC jazn"));
    expect(joined(segs)).toEqual(["Zażółć", "jaźń"]);
    expect(segs.map((s) => s.text).join("")).toBe("Zażółć gęślą jaźń");
  });

  it("maps ł and ą onto the right characters", () => {
    expect(joined(highlight("Płatności i łódź", searchTerms("platn lodz")))).toEqual(["Płatn", "łódź"]);
  });

  it("handles decomposed input and multi-character folds", () => {
    const decomposed = "Zazółc"; // ó as o + combining acute
    expect(joined(highlight(decomposed, searchTerms("zazolc")))).toEqual([decomposed]);
    expect(joined(highlight("Straße", searchTerms("strasse")))).toEqual(["Straße"]);
  });

  it("merges overlapping terms and ignores empty queries", () => {
    expect(matchRanges("abcdef", searchTerms("abc cde"))).toEqual([[0, 5]]);
    expect(highlight("abc", [])).toEqual([{ text: "abc", hit: false }]);
  });

  it("cuts a fragment around the first match", () => {
    const text = `${"początek ".repeat(12)}Przelewy24 wymaga odpisu ${"koniec ".repeat(12)}`;
    const frag = fragment(text, searchTerms("odpisu"), 20)!;
    const flat = frag.map((s) => s.text).join("");
    expect(joined(frag)).toEqual(["odpisu"]);
    expect(flat.startsWith("…")).toBe(true);
    expect(flat.endsWith("…")).toBe(true);
    expect(flat.length).toBeLessThan(70);
    expect(fragment("nic tu nie ma", searchTerms("xyz"))).toBeNull();
  });
});
