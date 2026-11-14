// ⌘K palette: search over tasks, topics and journal, plus navigation targets.
// Opens from anywhere; focus is trapped while open and restored on close.

import { Columns3, Inbox, NotebookPen, Sun } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

import type { JournalEntry, SearchHit, Snapshot, Task, Topic } from "../api";
import { fragment, highlight, searchTerms, type Segment } from "../lib/highlight";
import { formatStamp } from "../lib/dates";
import { taskHref, topicHref } from "../lib/nav";
import { GROUP_LABEL, buildItems, nextActive, resolveItem, type PaletteGroup, type PaletteItem } from "../lib/palette";
import { navigate } from "../router/router";
import { revealTask, revealTopic } from "../store/ui";
import { useVault } from "../store/vault";
import { StatusGlyph } from "./StatusToggle";
import { ProjectDot } from "./ui";

const IS_MAC = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
const DEBOUNCE_MS = 120;

export function Marked({ segments }: { segments: Segment[] }) {
  return (
    <>
      {segments.map((s, i) =>
        s.hit ? (
          <mark key={i} className="hl">
            {s.text}
          </mark>
        ) : (
          <span key={i}>{s.text}</span>
        ),
      )}
    </>
  );
}

export function Palette() {
  const { view } = useVault();
  const [open, setOpen] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
      if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        e.stopPropagation();
        setOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  if (!open || !view) return null;
  return <PaletteDialog snapshot={view.snapshot} version={view.version} today={view.today} onClose={() => setOpen(false)} />;
}

function PaletteDialog({
  snapshot,
  version,
  today,
  onClose,
}: {
  snapshot: Snapshot;
  version: number;
  today: string;
  onClose: () => void;
}) {
  const { api, notify } = useVault();
  // Read at activation time: the item resolves against the newest snapshot.
  const latestSnapshot = useRef(snapshot);
  latestSnapshot.current = snapshot;
  const [query, setQuery] = useState("");
  const [result, setResult] = useState<{ q: string; hits: SearchHit[] }>({ q: "", hits: [] });
  const [active, setActive] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const seq = useRef(0);

  // Focus trap: the app behind is inert, focus returns to where it was.
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    const app = document.getElementById("root");
    app?.setAttribute("inert", "");
    inputRef.current?.focus();
    return () => {
      app?.removeAttribute("inert");
      if (before && document.contains(before)) before.focus();
    };
  }, []);

  // Debounced backend search; stale answers are dropped. It reruns when the
  // vault changes (an agent may have added, moved or deleted what is shown).
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      seq.current++;
      setResult({ q: "", hits: [] });
      return;
    }
    const mine = ++seq.current;
    const t = window.setTimeout(() => {
      api
        .search(q)
        .then((hits) => mine === seq.current && setResult({ q, hits }))
        .catch(() => mine === seq.current && notify("Wyszukiwanie nie powiodło się."));
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(t);
  }, [query, version, api, notify]);

  const items = useMemo(() => buildItems(snapshot, query, result.hits), [snapshot, query, result.hits]);
  const settled = !query.trim() || result.q === query.trim();
  const terms = useMemo(() => searchTerms(query), [query]);

  // Results of a new query start from the top; the same query's results
  // refreshed after a vault change keep the highlighted item.
  const shown = useRef({ items, active, query: result.q });
  useEffect(() => {
    const prev = shown.current;
    if (prev.items !== items) setActive(nextActive(prev, items, result.q));
  }, [items]);
  useEffect(() => {
    shown.current = { items, active, query: result.q };
  });
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const choose = useCallback(
    (picked: PaletteItem) => {
      const item = resolveItem(latestSnapshot.current, picked);
      if (!item) {
        notify("Tego już nie ma — zostało usunięte poza aplikacją.");
        return;
      }
      onClose();
      switch (item.group) {
        case "nav":
          navigate(item.to);
          break;
        case "task": {
          const loc = item.task.location;
          revealTask(item.task.id, loc.kind === "inbox" ? null : loc.project, loc.kind === "topic" ? loc.topic : null);
          navigate(taskHref(item.task));
          break;
        }
        case "topic":
          revealTopic(item.topic.project, item.topic.slug);
          navigate(topicHref(item.topic));
          break;
        case "journal":
          revealTopic(item.entry.project, item.entry.topic);
          navigate(topicHref({ project: item.entry.project, slug: item.entry.topic }));
          break;
      }
    },
    [onClose, notify],
  );

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.nativeEvent.isComposing) return;
    const n = items.length;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (n) setActive((a) => (a + 1) % n);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (n) setActive((a) => (a - 1 + n) % n);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = items[active];
      if (item) choose(item);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === "Tab") {
      e.preventDefault(); // the input is the only stop
    }
  };

  let lastGroup: PaletteGroup | null = null;
  const rows: ReactNode[] = [];
  items.forEach((item, i) => {
    if (item.group !== lastGroup) {
      lastGroup = item.group;
      rows.push(
        <li key={`g:${item.group}`} className="pal-group" role="presentation">
          {query.trim() || item.group !== "nav" ? GROUP_LABEL[item.group] : "Przejdź do"}
        </li>,
      );
    }
    rows.push(
      <li
        key={item.key}
        id={`pal-opt-${i}`}
        data-index={i}
        role="option"
        aria-selected={i === active}
        className={`pal-item${i === active ? " active" : ""}`}
        onMouseMove={() => i !== active && setActive(i)}
        onClick={() => choose(item)}
      >
        <ItemRow item={item} snapshot={snapshot} today={today} terms={terms} />
      </li>,
    );
  });

  return createPortal(
    <div className="pal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div ref={rootRef} className="pal" role="dialog" aria-modal="true" aria-label="Szukaj i przechodź">
        <input
          ref={inputRef}
          className="pal-input"
          placeholder="Szukaj zadań, tematów, dziennika…"
          aria-label="Szukaj"
          role="combobox"
          aria-expanded="true"
          aria-controls="pal-list"
          aria-activedescendant={items.length ? `pal-opt-${active}` : undefined}
          autoComplete="off"
          spellCheck={false}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
        />
        <ul id="pal-list" ref={listRef} className="pal-list" role="listbox" aria-label="Wyniki">
          {rows}
          {items.length === 0 && settled && <li className="pal-none">Nic nie znaleziono dla „{query.trim()}”.</li>}
        </ul>
        <div className="pal-foot" aria-hidden>
          <span>↑↓ wybierz</span>
          <span>↵ otwórz</span>
          <span>esc zamknij</span>
        </div>
      </div>
    </div>,
    document.body,
  );
}

function Context({ snapshot, project, topic }: { snapshot: Snapshot; project: string | null; topic: string | null }) {
  const p = project ? snapshot.projects.find((x) => x.slug === project) : null;
  const t = project && topic ? snapshot.topics.find((x) => x.project === project && x.slug === topic) : null;
  const label = p ? (t ? `${p.name} · ${t.title}` : p.name) : "Skrzynka";
  return (
    <span className="pal-ctx" title={label}>
      {p && <ProjectDot color={p.color} size={6} />}
      <span className="ctx-text">{label}</span>
    </span>
  );
}

function ItemRow({ item, snapshot, today, terms }: { item: PaletteItem; snapshot: Snapshot; today: string; terms: string[] }) {
  switch (item.group) {
    case "nav":
      return (
        <div className="pal-line">
          <span className="pal-lead">
            {item.key === "nav:dzis" ? <Sun size={14} /> : item.key === "nav:skrzynka" ? <Inbox size={14} /> : item.key === "nav:tablica" ? <Columns3 size={14} /> : <ProjectDot color={item.color} />}
          </span>
          <span className="pal-title">
            <Marked segments={highlight(item.label, terms)} />
          </span>
        </div>
      );
    case "task":
      return <TaskItem task={item.task} snapshot={snapshot} terms={terms} />;
    case "topic":
      return <TopicItem topic={item.topic} snapshot={snapshot} terms={terms} />;
    case "journal":
      return <JournalItem entry={item.entry} snapshot={snapshot} today={today} terms={terms} />;
  }
}

function TaskItem({ task, snapshot, terms }: { task: Task; snapshot: Snapshot; terms: string[] }) {
  const loc = task.location;
  const note = fragment(task.notes, terms);
  return (
    <>
      <div className="pal-line">
        <span className={`pal-lead status status-${task.status}`}>
          <StatusGlyph status={task.status} />
        </span>
        <span className={`pal-title${task.status === "done" ? " is-done" : ""}`}>
          <Marked segments={highlight(task.title, terms)} />
        </span>
        <Context snapshot={snapshot} project={loc.kind === "inbox" ? null : loc.project} topic={loc.kind === "topic" ? loc.topic : null} />
      </div>
      {note && (
        <div className="pal-snippet">
          <Marked segments={note} />
        </div>
      )}
    </>
  );
}

function TopicItem({ topic, snapshot, terms }: { topic: Topic; snapshot: Snapshot; terms: string[] }) {
  const p = snapshot.projects.find((x) => x.slug === topic.project);
  const summary = fragment(topic.summary, terms);
  return (
    <>
      <div className="pal-line">
        <span className="pal-lead">
          <span className={`badge topic-${topic.status}`}>
            <span className="badge-mark" aria-hidden />
          </span>
        </span>
        <span className="pal-title">
          <Marked segments={highlight(topic.title, terms)} />
        </span>
        <span className="pal-ctx" title={p?.name}>
          <ProjectDot color={p?.color} size={6} />
          <span className="ctx-text">{p?.name ?? topic.project}</span>
        </span>
      </div>
      {summary && (
        <div className="pal-snippet">
          <Marked segments={summary} />
        </div>
      )}
    </>
  );
}

function JournalItem({ entry, snapshot, today, terms }: { entry: JournalEntry; snapshot: Snapshot; today: string; terms: string[] }) {
  const text = fragment(entry.text, terms, 70) ?? [{ text: entry.text, hit: false }];
  return (
    <>
      <div className="pal-line">
        <span className="pal-lead">
          <NotebookPen size={13} />
        </span>
        <span className="pal-title pal-journal">
          <Marked segments={text} />
        </span>
        <Context snapshot={snapshot} project={entry.project} topic={entry.topic} />
        <time className="pal-time" dateTime={entry.at}>
          {formatStamp(entry.at, today)}
        </time>
      </div>
    </>
  );
}
