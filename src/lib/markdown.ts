// Tiny Markdown-subset parser for agent answers (untrusted model output).
// Produces plain data; the view turns it into React elements, so nothing is ever
// interpreted as HTML. Supported: paragraphs, **bold**, `code`, -/*/+ and 1. lists
// (one nesting level), # headings, fenced code blocks. Everything else (links,
// tables, HTML, *italics*…) stays literal text. All scans are linear.

export type Inline = { t: "text"; text: string } | { t: "bold"; children: Inline[] } | { t: "code"; text: string };

export interface ListItem {
  inline: Inline[];
  children: ListBlock | null;
}

export interface ListBlock {
  t: "list";
  ordered: boolean;
  items: ListItem[];
}

export type Block =
  | { t: "p"; inline: Inline[] }
  | { t: "heading"; inline: Inline[] }
  | { t: "code"; text: string }
  | ListBlock;

/** Input beyond this is cut off (a runaway answer must not freeze the UI). */
export const MAX_MARKDOWN_CHARS = 200_000;

export function parseInline(src: string, allowBold = true): Inline[] {
  const out: Inline[] = [];
  let buf = "";
  let i = 0;
  // Once a closing marker is missing after position p, it is missing for every later start too.
  let noBoldClose = !allowBold;
  let noCodeClose = false;
  const flush = () => {
    if (buf) out.push({ t: "text", text: buf });
    buf = "";
  };
  while (i < src.length) {
    if (!noBoldClose && src.startsWith("**", i)) {
      const end = src.indexOf("**", i + 2);
      if (end === -1) noBoldClose = true;
      else if (end > i + 2) {
        flush();
        out.push({ t: "bold", children: parseInline(src.slice(i + 2, end), false) });
        i = end + 2;
        continue;
      }
    }
    if (!noCodeClose && src[i] === "`") {
      const end = src.indexOf("`", i + 1);
      if (end === -1) noCodeClose = true;
      else if (end > i + 1) {
        flush();
        out.push({ t: "code", text: src.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    buf += src[i];
    i++;
  }
  flush();
  return out;
}

const FENCE = /^\s{0,3}(`{3,}|~{3,})/;
const HEADING = /^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$/;
const BULLET = /^(\s*)[-*+•]\s+(.*)$/;
const NUMBERED = /^(\s*)\d{1,9}[.)]\s+(.*)$/;

function indentOf(ws: string): number {
  let n = 0;
  for (const c of ws) n += c === "\t" ? 4 : 1;
  return n;
}

export function parseMarkdown(input: string): Block[] {
  let truncated = false;
  let text = input;
  if (text.length > MAX_MARKDOWN_CHARS) {
    text = text.slice(0, MAX_MARKDOWN_CHARS);
    truncated = true;
  }
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let para: string[] = [];
  let list: ListBlock | null = null;

  const flushPara = () => {
    if (para.length) blocks.push({ t: "p", inline: parseInline(para.join("\n")) });
    para = [];
  };

  for (let n = 0; n < lines.length; n++) {
    const line = lines[n]!;
    const fence = FENCE.exec(line);
    if (fence) {
      flushPara();
      list = null;
      const mark = fence[1]!;
      const body: string[] = [];
      n++;
      // Unclosed fence: everything to the end is code.
      while (n < lines.length) {
        const l = lines[n]!;
        const t = l.trim();
        if (t.startsWith(mark[0]!.repeat(mark.length)) && /^(`+|~+)$/.test(t)) break;
        body.push(l);
        n++;
      }
      blocks.push({ t: "code", text: body.join("\n") });
      continue;
    }
    if (line.trim() === "") {
      flushPara();
      list = null;
      continue;
    }
    const h = HEADING.exec(line);
    if (h) {
      flushPara();
      list = null;
      blocks.push({ t: "heading", inline: parseInline(h[1]!) });
      continue;
    }
    const m = BULLET.exec(line) ?? NUMBERED.exec(line);
    if (m) {
      flushPara();
      const ordered = NUMBERED.test(line) && !BULLET.test(line);
      const indent = indentOf(m[1]!);
      const item: ListItem = { inline: parseInline(m[2]!), children: null };
      if (list && indent >= 2) {
        // One nesting level: anything indented goes under the last top-level item.
        const parent: ListItem = list.items[list.items.length - 1]!;
        if (!parent.children) parent.children = { t: "list", ordered, items: [] };
        parent.children.items.push(item);
      } else {
        if (!list || (indent < 2 && list.ordered !== ordered)) {
          list = { t: "list", ordered, items: [] };
          blocks.push(list);
        }
        list.items.push(item);
      }
      continue;
    }
    if (list && /^\s{2,}\S/.test(line)) {
      // Continuation of the last list item.
      const top: ListItem = list.items[list.items.length - 1]!;
      const target: ListItem = top.children ? top.children.items[top.children.items.length - 1]! : top;
      target.inline = [...target.inline, ...parseInline(" " + line.trim())];
      continue;
    }
    list = null;
    para.push(line);
  }
  flushPara();
  if (truncated) blocks.push({ t: "p", inline: [{ t: "text", text: "… (odpowiedź skrócona w podglądzie — użyj „Kopiuj”, by wziąć całość)" }] });
  return blocks;
}
