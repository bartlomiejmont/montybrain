import { t as tr, getLanguage } from "../i18n";
// ⌘K palette: search over tasks, topics and journal, plus navigation targets.
// Opens from anywhere; focus is trapped while open and restored on close.

import { Columns3, Inbox, Network, NotebookPen, Play, Settings2, Sun } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

import { mockCommandState, MOCK_COMMAND_TEXT, type JournalEntry, type SearchHit, type Snapshot, type Task, type Topic } from "../api";
import { CommandField } from "./CommandField";
import { fragment, highlight, searchTerms, type Segment } from "../lib/highlight";
import { formatStamp } from "../lib/dates";
import { taskHref, topicHref } from "../lib/nav";
import { GROUP_LABEL, buildItems, nextActive, resolveItem, type PaletteGroup, type PaletteItem } from "../lib/palette";
import { links, navigate } from "../router/router";
import { revealTask, revealTopic } from "../store/ui";
import { useVault } from "../store/vault";
import { StatusGlyph } from "./StatusToggle";
import { ProjectDot, ProjectName } from "./ui";

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
  // `?palette=confirm` (browser mock): open on the command step, for screenshots.
  const [open, setOpen] = useState(() => mockCommandState() !== null && typeof window !== "undefined" && new URLSearchParams(window.location.search).has("palette"));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
      if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        e.stopPropagation();
        setOpen((o) => !o);
      }
    };
    const onOpen = () => setOpen(true);
    window.addEventListener("montybrain:search", onOpen);
    window.addEventListener("keydown", onKey, true);
    // A link inside the palette ("Pokaż na mapie") navigated: close it.
    const onHash = () => setOpen(false);
    window.addEventListener("hashchange", onHash);
    return () => {
      window.removeEventListener("montybrain:search", onOpen);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("hashchange", onHash);
    };
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
  const mockPalette = open0();
  const [query, setQuery] = useState(mockPalette ?? "");
  // "Nowe…" chosen: the palette becomes the command field (the same one as Skrzynka).
  const [command, setCommand] = useState<string | null>(mockPalette);
  const [newReading, setNewReading] = useState<{ q: string; text: string } | null>(null);
  const [result, setResult] = useState<{ q: string; hits: SearchHit[] }>({ q: "", hits: [] });
  const [active, setActive] = useState(0);
  const [searchError, setSearchError] = useState<string | null>(null);
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
    setSearchError(null);
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
        .catch(() => {
          if (mine !== seq.current) return;
          setResult({ q, hits: [] });
          setSearchError(tr("Nie udało się wyszukać. Spróbuj ponownie."));
          notify(tr("Wyszukiwanie nie powiodło się."));
        });
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(t);
  }, [query, version, api, notify]);

  // The reading of the query as a new task or command (for the "Nowe" row).
  useEffect(() => {
    const q = query.trim();
    if (!q) return;
    const t = window.setTimeout(() => {
      api
        .commandPreview({ text: q })
        .then((r) => setNewReading({ q, text: r.preview.reading || (r.preview.problems[0] ?? "") }))
        .catch(() => {});
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(t);
  }, [query, api]);

  const items = useMemo(() => buildItems(snapshot, query, result.q === query.trim() ? result.hits : []), [snapshot, query, result, getLanguage()]);
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
        notify(tr("Tego już nie ma — zostało usunięte poza aplikacją."));
        return;
      }
      if (item.group === "new") {
        setCommand(item.text);
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
      // ⌘↵ / Ctrl+↵ on a topic: show it on the map.
      if (item?.group === "topic" && (e.metaKey || e.ctrlKey)) {
        onClose();
        navigate(links.map(`${item.topic.project}/${item.topic.slug}`));
      } else if (item) choose(item);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === "Tab") {
      e.preventDefault();
      const mapLink = rootRef.current?.querySelector<HTMLAnchorElement>(".pal-map");
      if (document.activeElement === inputRef.current && mapLink) mapLink.focus();
      else inputRef.current?.focus();
    }
  };

  let lastGroup: PaletteGroup | null = null;
  const rows: ReactNode[] = [];
  items.forEach((item, i) => {
    if (item.group !== lastGroup) {
      lastGroup = item.group;
      rows.push(
        <li key={`g:${item.group}`} className="pal-group" role="presentation">
          {query.trim() || item.group !== "nav" ? GROUP_LABEL[item.group] : tr("Przejdź do")}
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
        {item.group === "new" ? (
          <div className="pal-line">
            <span className="pal-lead" aria-hidden>
              +
            </span>
            <span className="pal-title">{newReading && newReading.q === item.text ? newReading.text : tr("Nowe: {0}", item.text)}</span>
            <span className="pal-ctx">{tr("↵ dalej")}</span>
          </div>
        ) : (
          <ItemRow item={item} snapshot={snapshot} today={today} terms={terms} />
        )}
      </li>,
    );
  });

  if (command !== null) {
    return createPortal(
      <div className="pal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
        <div ref={rootRef} className="pal pal-command" role="dialog" aria-modal="true" aria-label={tr("Nowe zadanie albo komenda")}>
          <CommandField
            variant="palette"
            source="palette"
            initialText={command}
            autoFocus
            autoSubmit={!mockPalette}
            startConfirmed={!!mockPalette}
            onEscape={() => {
              setCommand(null);
              requestAnimationFrame(() => inputRef.current?.focus());
            }}
            onWritten={() => window.setTimeout(onClose, 900)}
          />
          <div className="pal-foot" aria-hidden>
            <span>{tr("esc w polu pustym: wróć do wyszukiwania")}</span>
          </div>
        </div>
      </div>,
      document.body,
    );
  }

  return createPortal(
    <div className="pal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div ref={rootRef} className="pal" role="dialog" aria-modal="true" aria-label={tr("Szukaj i przechodź")} onKeyDown={(e) => {
        if (e.key === "Escape") { e.stopPropagation(); onClose(); }
        if (e.key === "Tab" && e.target !== inputRef.current) { e.preventDefault(); inputRef.current?.focus(); }
      }}>
        <input
          ref={inputRef}
          className="pal-input"
          placeholder={tr("Szukaj zadań, tematów, dziennika…")}
          aria-label={tr("Szukaj")}
          aria-describedby="pal-help"
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
        <ul id="pal-list" ref={listRef} className="pal-list" role="listbox" aria-label={tr("Wyniki")}>
          {rows}

        </ul>
        {(!settled || searchError || items.length === 0) && (
          <p className="pal-none" role="status" aria-live="polite">
            {!settled ? tr("Szukam…") : searchError ?? tr("Brak wyników dla „{0}”. Zmień zapytanie.", query.trim())}
          </p>
        )}
        <div className="pal-foot" id="pal-help">
          <span>{tr("↑↓ wybierz")}</span>
          <span>{tr("↵ otwórz")}</span>
          {items[active]?.group === "topic" && (
            <a className="pal-map" href={links.map(`${items[active].topic.project}/${items[active].topic.slug}`)}>{tr("Pokaż na mapie")} <kbd>{IS_MAC ? "⌘" : "Ctrl+"}↵</kbd></a>
          )}
          <span>{tr("esc zamknij")}</span>
        </div>
      </div>
    </div>,
    document.body,
  );
}

function Context({ snapshot, project, topic }: { snapshot: Snapshot; project: string | null; topic: string | null }) {
  const p = project ? snapshot.projects.find((x) => x.slug === project) : null;
  const t = project && topic ? snapshot.topics.find((x) => x.project === project && x.slug === topic) : null;
  const label = p ? (t ? `${p.name} · ${t.title}` : p.name) : tr("Skrzynka");
  return (
    <span className="pal-ctx" title={label}>
      {p ? <><ProjectName name={p.name} color={p.color} />{t && <span className="ctx-text"> · {t.title}</span>}</> : <span className="ctx-text">{tr("Skrzynka")}</span>}
    </span>
  );
}

/** The mock's `?palette` URL state: the example text to open the command step with. */
function open0(): string | null {
  const state = mockCommandState();
  if (!state || typeof window === "undefined" || !new URLSearchParams(window.location.search).has("palette")) return null;
  return MOCK_COMMAND_TEXT[state].text;
}

function ItemRow({ item, snapshot, today, terms }: { item: PaletteItem; snapshot: Snapshot; today: string; terms: string[] }) {
  switch (item.group) {
    case "new":
      return null;
    case "nav":
      return (
        <div className="pal-line">
          <span className="pal-lead">
            {item.key === "nav:dzis" ? <Sun size={14} /> : item.key === "nav:skrzynka" ? <Inbox size={14} /> : item.key === "nav:tablica" ? <Columns3 size={14} /> : item.key === "nav:mapa" ? <Network size={14} /> : item.key === "nav:akcje" ? <Play size={14} /> : item.key === "nav:ustawienia" ? <Settings2 size={14} /> : <ProjectDot color={item.color} />}
          </span>
          <span className="pal-title" style={item.color ? { color: item.color } : undefined}>
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
          <ProjectName name={p?.name ?? topic.project} color={p?.color} />
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
