// Mock data for „Mapa" (browser and tests): `?map=demo` adds a fourth project,
// more topics and tasks with long Polish titles and references between them
// (cross-project ones, a wait chain of three, a cycle, a dangling reference,
// a task without an id); `?map=large` replaces the vault with a synthetic one
// of 5 projects, 40 topics and 300 tasks for layout checks. All synthetic.

import type { Project, Snapshot, Task, TaskLocation, TaskStatus, Topic, TopicStatus } from "./types";
import { addDays } from "../lib/dates";

let line = 5000;

function topic(project: string, slug: string, title: string, status: TopicStatus, summary: string, created: string, updated: string): Topic {
  return { project, slug, title, status, repo: null, summary, created, updated, path: `${project}/tematy/${slug}.md` };
}

function task(id: string | null, project: string, t: string, title: string, status: TaskStatus = "todo", notes = ""): Task {
  const location: TaskLocation = { kind: "topic", project, topic: t };
  return { id, title, status, priority: "normal", due: null, done: status === "done" ? "2026-11-01" : null, links: [], notes, location, line: ++line };
}

const TASK_TITLES = [
  "Spisać kroki ręcznego testu na urządzeniu z najstarszą obsługiwaną wersją systemu",
  "Porównać koszty dwóch dostawców i przygotować krótką notatkę z rekomendacją",
  "Uzupełnić dokumentację wdrożenia o przypadki brzegowe zgłoszone przez wsparcie",
  "Ustalić z zespołem zakres pierwszej wersji i to, co świadomie odkładamy na później",
  "Dodać alert, gdy kolejka zadań w tle rośnie dłużej niż kwadrans",
  "Przejrzeć uwagi z przeglądu kodu i zamknąć wątki, które już nie są aktualne",
  "Napisać skrypt migracji danych z możliwością uruchomienia na sucho",
  "Zebrać metryki z ostatniego tygodnia i porównać je z okresem sprzed zmiany",
];

/** `?map=demo`: the fixture vault, extended. */
export function addMapDemo(db: Snapshot, today: string): void {
  const d = (n: number) => addDays(today, n);
  const set = (project: string, slug: string, f: (t: Topic) => void) => {
    const t = db.topics.find((x) => x.project === project && x.slug === slug);
    if (t) f(t);
  };
  if (!db.projects.some((p) => p.slug === "firma")) {
    const firma: Project = {
      slug: "firma",
      name: "Firma i formalności",
      color: "#38bdf8",
      state: "active",
      repos: [],
      description: "Rejestracje, umowy, księgowość.",
      path: "firma/projekt.md",
    };
    // After the active ones, before paused / archived (sidebar order).
    const at = db.projects.findIndex((p) => p.state !== "active");
    db.projects.splice(at < 0 ? db.projects.length : at, 0, firma);
  }
  // Existing topics get references.
  set("whenubuy", "platnosci-stripe-przelewy24", (t) => {
    t.summary = `${t.summary}\nCzeka na: [[firma/odpis-krs]]`;
  });
  set("whenubuy", "powiadomienia-push", (t) => {
    t.summary = `${t.summary} Kolejkę wysyłki robimy jak w [[morwisko/serwer-mcp]].`;
  });
  set("whenubuy", "ab-testy-cen", (t) => {
    t.summary = `${t.summary}\nPowiązane: [[firma/ksiegowosc-vat]]\nStary plan był w [[whenubuy/stary-checkout]].`;
  });
  db.topics.push(
    topic("whenubuy", "checkout-mobilny", "Nowy checkout w aplikacji mobilnej z zapisanymi kartami i Apple Pay", "waiting", "Ekrany gotowe, czekamy na bramkę płatności.\nCzeka na: [[platnosci-stripe-przelewy24]]", d(-40), `${d(-2)}T10:00`),
    topic("whenubuy", "wyszukiwarka", "Wyszukiwarka produktów z podpowiedziami i literówkami", "in_progress", "Indeks w Meilisearch, trwa strojenie wag. Powiązane: [[ab-testy-cen]]", d(-35), `${d(-1)}T15:00`),
    topic("whenubuy", "alerty-cenowe", "Alerty cenowe e-mail i push dla obserwowanych produktów", "planned", "Do zaplanowania po wydaniu powiadomień push.\nPowiązane: [[powiadomienia-push]]", d(-20), `${d(-20)}T09:00`),
    topic("morwisko", "mapa-mysli", "Mapa myśli: projekty, tematy i połączenia na jednym płótnie", "in_progress", "Układ wysp i odwołania w opisach.\nCzeka na: [[serwer-mcp-polaczenia]]", d(-6), `${d(0)}T11:00`),
    topic("morwisko", "serwer-mcp-polaczenia", "Narzędzie MCP do odczytu połączeń między tematami", "planned", "Najpierw format odwołań.\nCzeka na: [[mapa-mysli]]", d(-5), `${d(-5)}T11:00`),
    topic("firma", "odpis-krs", "Odpis z KRS i weryfikacja kont u operatorów płatności", "waiting", "Zamówiony odpis, sąd ma termin do poniedziałku.\nCzeka na: [[umowy-dostawcy]]", d(-14), `${d(-1)}T09:00`),
    topic("firma", "umowy-dostawcy", "Umowy z dostawcami usług: hosting, poczta, płatności", "in_progress", "Brakuje podpisu na umowie z operatorem.", d(-30), `${d(-3)}T12:00`),
    topic("firma", "ksiegowosc-vat", "Księgowość i rozliczenie VAT za czwarty kwartał", "in_progress", "Biuro potrzebuje zestawienia transakcji.\nPowiązane: [[whenubuy/platnosci-stripe-przelewy24]]", d(-25), `${d(-2)}T16:00`),
  );
  const add: Task[] = [];
  const topics = db.topics.filter((t) => t.status !== "done" && db.projects.find((p) => p.slug === t.project)?.state === "active");
  let n = 0;
  for (const t of topics) {
    for (let i = 0; i < 4; i++) {
      const status: TaskStatus = i === 3 ? "done" : i === 0 && n % 3 === 0 ? "doing" : "todo";
      add.push(task(`m${String(n).padStart(2, "0")}x${i}${t.slug.length % 10}`.slice(0, 6).padEnd(6, "0"), t.project, t.slug, TASK_TITLES[(n + i) % TASK_TITLES.length]!, status));
    }
    n++;
  }
  // One task without an id (before normalization) and one task waiting for another.
  add.push(task(null, "whenubuy", "wyszukiwarka", "Zadanie dopisane ręcznie, jeszcze bez identyfikatora"));
  add.push(task("w8a1t2", "whenubuy", "checkout-mobilny", "Podpiąć zapisane karty po stronie serwera", "blocked", "Czeka na: ^s1d2f3"));
  const ids = new Set(db.tasks.map((t) => t.id));
  for (const t of add) {
    if (t.id && ids.has(t.id)) t.id = `${t.id.slice(0, 4)}${String(line % 100).padStart(2, "0")}`;
    if (t.id) ids.add(t.id);
    db.tasks.push(t);
  }
}

/** `?map=large`: 5 projects, 40 topics, 300 tasks (most done). */
export function buildLargeFixture(today: string): Snapshot {
  const names = ["Platforma sprzedażowa", "Aplikacja mobilna", "Infrastruktura i wdrożenia", "Firma i formalności", "Badania i prototypy"];
  const projects: Project[] = names.map((name, i) => ({
    slug: `projekt-${i + 1}`,
    name,
    color: ["#a3e635", "#818cf8", "#fb923c", "#38bdf8", "#f472b6"][i]!,
    state: "active",
    repos: [{ name: `repo-${i + 1}`, path: `/r/repo-${i + 1}` }],
    description: "",
    path: `projekt-${i + 1}/projekt.md`,
  }));
  const statuses: TopicStatus[] = ["in_progress", "in_progress", "planned", "waiting", "blocked", "in_progress", "planned", "in_progress"];
  const topics: Topic[] = [];
  const tasks: Task[] = [];
  let id = 0;
  for (let i = 0; i < 40; i++) {
    const p = projects[i % 5]!;
    const slug = `temat-${String(i + 1).padStart(2, "0")}`;
    const ref = i % 6 === 0 ? `\nPowiązane: [[${projects[(i + 1) % 5]!.slug}/temat-${String(((i + 3) % 40) + 1).padStart(2, "0")}]]` : i % 7 === 3 ? `\nCzeka na: [[temat-${String(((i + 5) % 40) + 1).padStart(2, "0")}]]` : "";
    topics.push(topic(p.slug, slug, `${["Przebudowa", "Wdrożenie", "Analiza", "Migracja", "Porządki w"][i % 5]} ${["modułu rozliczeń z partnerami", "kolejki zadań w tle", "panelu administracyjnego dla obsługi", "procesu zwrotów i reklamacji", "raportów sprzedażowych dla zarządu", "integracji z hurtownią"][i % 6]} (${i + 1})`, statuses[i % statuses.length]!, `Stan tematu numer ${i + 1}.${ref}`, addDays(today, -80 + i), `${addDays(today, -(i % 9))}T10:00`));
  }
  for (let k = 0; k < 300; k++) {
    const t = topics[k % 40]!;
    const status: TaskStatus = k % 2 === 0 ? "done" : k % 7 === 1 ? "doing" : "todo";
    tasks.push(task(`l${String(id++).padStart(5, "0")}`, t.project, t.slug, `${TASK_TITLES[k % TASK_TITLES.length]} — ${k + 1}`, status));
  }
  return { root: "/Users/ja/Morwisko", projects, topics, tasks, journal: [], warnings: [] };
}
