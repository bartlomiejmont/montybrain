// Renders the parsed Markdown subset as React elements. Everything is a text
// node or a plain element — never HTML, never a clickable link.

import { Fragment, useMemo } from "react";

import { parseMarkdown, type Block, type Inline, type ListBlock } from "../../lib/markdown";

function Inlines({ nodes }: { nodes: Inline[] }) {
  return (
    <>
      {nodes.map((n, i) =>
        n.t === "text" ? (
          <Fragment key={i}>{n.text}</Fragment>
        ) : n.t === "bold" ? (
          <strong key={i}>
            <Inlines nodes={n.children} />
          </strong>
        ) : n.t === "italic" ? (
          <em key={i}>
            <Inlines nodes={n.children} />
          </em>
        ) : (
          <code key={i} className="md-code">
            {n.text}
          </code>
        ),
      )}
    </>
  );
}

function List({ list }: { list: ListBlock }) {
  const Tag = list.ordered ? "ol" : "ul";
  return (
    <Tag className="md-list">
      {list.items.map((it, i) => (
        <li key={i}>
          <Inlines nodes={it.inline} />
          {it.children && <List list={it.children} />}
        </li>
      ))}
    </Tag>
  );
}

function BlockView({ block }: { block: Block }) {
  switch (block.t) {
    case "p":
      return (
        <p className="md-p">
          <Inlines nodes={block.inline} />
        </p>
      );
    case "heading": {
      const Tag = block.level <= 2 ? "h3" : "h4";
      return (
        <Tag className={`md-heading md-h${block.level <= 2 ? 1 : 2}`}>
          <Inlines nodes={block.inline} />
        </Tag>
      );
    }
    case "code":
      return <pre className="md-pre">{block.text}</pre>;
    case "list":
      return <List list={block} />;
  }
}

export function Markdown({ text }: { text: string }) {
  const blocks = useMemo(() => parseMarkdown(text), [text]);
  return (
    <div className="md">
      {blocks.map((b, i) => (
        <BlockView key={i} block={b} />
      ))}
    </div>
  );
}
