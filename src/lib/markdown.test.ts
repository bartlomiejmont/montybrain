import { describe, expect, it } from "vitest";

import { MAX_MARKDOWN_CHARS, parseInline, parseMarkdown, type Block } from "./markdown";

const kinds = (b: Block[]) => b.map((x) => x.t);

describe("parseInline", () => {
  it("handles bold and code", () => {
    expect(parseInline("a **b** `c` d")).toEqual([
      { t: "text", text: "a " },
      { t: "bold", children: [{ t: "text", text: "b" }] },
      { t: "text", text: " " },
      { t: "code", text: "c" },
      { t: "text", text: " d" },
    ]);
  });
  it("allows code inside bold", () => {
    expect(parseInline("**`x`**")).toEqual([{ t: "bold", children: [{ t: "code", text: "x" }] }]);
  });
  it("leaves unclosed markers literal", () => {
    expect(parseInline("**a `b")).toEqual([{ t: "text", text: "**a `b" }]);
    expect(parseInline("**** ``")).toEqual([{ t: "text", text: "**** ``" }]);
  });
  it("keeps HTML and links as literal text", () => {
    const src = '<img src=x onerror=alert(1)> [klik](javascript:alert(1)) <script>x</script>';
    expect(parseInline(src)).toEqual([{ t: "text", text: src }]);
  });
  it("is linear on hostile input", () => {
    const t = Date.now();
    parseInline("**a ".repeat(100_000));
    parseInline("`a ".repeat(100_000));
    parseInline("x".repeat(1_000_000));
    expect(Date.now() - t).toBeLessThan(2000);
  });
});

describe("parseMarkdown", () => {
  it("splits paragraphs and headings", () => {
    const b = parseMarkdown("# Tytuł\n\nPierwszy\nakapit\n\nDrugi");
    expect(kinds(b)).toEqual(["heading", "p", "p"]);
    expect(b[1]).toMatchObject({ inline: [{ t: "text", text: "Pierwszy\nakapit" }] });
  });
  it("parses lists with one nesting level", () => {
    const b = parseMarkdown("- a\n  - a1\n  - a2\n- b\n\n1. x\n2. y");
    expect(kinds(b)).toEqual(["list", "list"]);
    const l = b[0] as Extract<Block, { t: "list" }>;
    expect(l.ordered).toBe(false);
    expect(l.items).toHaveLength(2);
    expect(l.items[0]!.children!.items).toHaveLength(2);
    expect((b[1] as typeof l).ordered).toBe(true);
  });
  it("flattens deeper nesting to one level", () => {
    const b = parseMarkdown("- a\n  - b\n    - c");
    const l = b[0] as Extract<Block, { t: "list" }>;
    expect(l.items).toHaveLength(1);
    expect(l.items[0]!.children!.items).toHaveLength(2);
    expect(l.items[0]!.children!.items[0]!.children).toBeNull();
  });
  it("appends continuation lines to the item", () => {
    const b = parseMarkdown("- a\n  dalej");
    const l = b[0] as Extract<Block, { t: "list" }>;
    expect(l.items).toHaveLength(1);
    expect(JSON.stringify(l.items[0]!.inline)).toContain("dalej");
  });
  it("parses fenced code, also unclosed", () => {
    expect(parseMarkdown("```js\nlet a = 1;\n```\npo")).toEqual([
      { t: "code", text: "let a = 1;" },
      { t: "p", inline: [{ t: "text", text: "po" }] },
    ]);
    expect(parseMarkdown("tekst\n```\nx\n- y")).toEqual([
      { t: "p", inline: [{ t: "text", text: "tekst" }] },
      { t: "code", text: "x\n- y" },
    ]);
  });
  it("does not interpret markup inside code", () => {
    expect(parseMarkdown("```\n**a** <b>\n```")).toEqual([{ t: "code", text: "**a** <b>" }]);
  });
  it("handles CRLF and empty input", () => {
    expect(parseMarkdown("")).toEqual([]);
    expect(kinds(parseMarkdown("a\r\n\r\nb"))).toEqual(["p", "p"]);
  });
  it("keeps hostile content as text", () => {
    const b = parseMarkdown("<script>alert(1)</script>\n\n[x](javascript:alert(1))");
    expect(JSON.stringify(b)).not.toContain('"href"');
    expect(kinds(b)).toEqual(["p", "p"]);
  });
  it("copes with very long lines and truncates huge input", () => {
    const t = Date.now();
    const b = parseMarkdown("a".repeat(MAX_MARKDOWN_CHARS * 3));
    expect(Date.now() - t).toBeLessThan(2000);
    expect(b).toHaveLength(2);
  });
});
