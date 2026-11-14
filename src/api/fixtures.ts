// Polish fixture data for the browser mock. Dates are relative to "today" so the
// Dziś view always has something overdue, due today and in progress.

import type { JournalEntry, Project, Snapshot, Task, TaskLocation, Topic } from "./types";
import { addDays } from "../lib/dates";

export function buildFixtures(today: string, now: string): Snapshot {
  const d = (n: number) => addDays(today, n);
  const at = (days: number, time: string) => `${d(days)}T${time}`;
  const nowTime = now.slice(11, 16);

  const projects: Project[] = [
    {
      slug: "whenubuy",
      name: "WhenUBuy",
      color: "#a3e635",
      state: "active",
      repos: [
        { name: "whenubuy-backend", path: "/Users/ja/Projects/WhenUBuy/whenubuy-backend" },
        { name: "whenubuy-mobile", path: "/Users/ja/Projects/WhenUBuy/whenubuy-mobile" },
      ],
      description: "Porównywarka cen z historią i alertami. Backend w Kotlinie, aplikacja w React Native.",
      path: "whenubuy/projekt.md",
    },
    {
      slug: "montybrain",
      name: "MontyBrain",
      color: "#818cf8",
      state: "active",
      repos: [{ name: "montybrain", path: "/Users/ja/Projects/montybrain" }],
      description: "Lokalne centrum dowodzenia: projekty, tematy, zadania w plikach Markdown.",
      path: "montybrain/projekt.md",
    },
    {
      slug: "remont-kuchni",
      name: "Remont kuchni",
      color: "#fb923c",
      state: "paused",
      repos: [],
      description: "",
      path: "remont-kuchni/projekt.md",
    },
    {
      slug: "stary-blog",
      name: "Stary blog",
      color: null,
      state: "archived",
      repos: [],
      description: "Blog na Jekyllu, zastąpiony notatkami.",
      path: "stary-blog/projekt.md",
    },
  ];

  const topic = (
    project: string,
    slug: string,
    title: string,
    status: Topic["status"],
    summary: string,
    updated: string,
    repo: string | null = null,
  ): Topic => ({
    project,
    slug,
    title,
    status,
    repo,
    summary,
    created: d(-30),
    updated,
    path: `${project}/tematy/${slug}.md`,
  });

  const topics: Topic[] = [
    topic(
      "whenubuy",
      "ab-testy-cen",
      "A/B testy cen",
      "in_progress",
      "Wariant B (ceny z końcówką ,99) idzie na 20% ruchu. Czekamy na tydzień danych, potem decyzja o rolloucie.",
      at(0, "09:41"),
      "whenubuy-backend",
    ),
    topic(
      "whenubuy",
      "platnosci-stripe-przelewy24",
      "Płatności: Stripe → Przelewy24",
      "blocked",
      "Integracja gotowa na sandboksie. Blokuje nas weryfikacja konta firmowego w Przelewy24.",
      at(-1, "16:20"),
      "whenubuy-backend",
    ),
    topic(
      "whenubuy",
      "powiadomienia-push",
      "Powiadomienia push w aplikacji mobilnej",
      "waiting",
      "Czekamy na review w App Store.",
      at(-3, "11:05"),
      "whenubuy-mobile",
    ),
    topic(
      "whenubuy",
      "refaktoryzacja-koszyka",
      "Refaktoryzacja modułu koszyka i synchronizacji stanu między urządzeniami użytkownika",
      "planned",
      "",
      at(-9, "10:00"),
    ),
    topic("whenubuy", "onboarding", "Onboarding nowych użytkowników", "done", "Wdrożone w wersji 2.4.", at(-20, "18:30")),
    topic(
      "montybrain",
      "aplikacja-desktopowa",
      "Aplikacja desktopowa",
      "in_progress",
      "Szkielet Tauri, widoki Dziś i projektu, pełna edycja.",
      at(0, nowTime),
      "montybrain",
    ),
    topic(
      "montybrain",
      "serwer-mcp",
      "Serwer MCP",
      "in_progress",
      "Narzędzia do odczytu i zapisu zadań dla agentów.",
      at(0, "08:12"),
      "montybrain",
    ),
    topic("remont-kuchni", "wybor-blatu", "Wybór blatu", "waiting", "Czekam na wycenę konglomeratu.", at(-12, "19:00")),
  ];

  let line = 0;
  const task = (location: TaskLocation, id: string, title: string, extra: Partial<Task> = {}): Task => ({
    id,
    title,
    status: "todo",
    priority: "normal",
    due: null,
    done: null,
    links: [],
    notes: "",
    location,
    line: ++line,
    ...extra,
  });
  const inbox: TaskLocation = { kind: "inbox" };
  const tp = (project: string, topicSlug: string): TaskLocation => ({ kind: "topic", project, topic: topicSlug });
  const pr = (project: string): TaskLocation => ({ kind: "project", project });

  const tasks: Task[] = [
    task(inbox, "a1b2c3", "Odpisać Kasi w sprawie faktury za październik", { due: d(-1), priority: "high" }),
    task(inbox, "k9x2m1", "Sprawdzić ofertę hostingu na VPS z Frankfurtem"),
    task(inbox, "p0o9i8", "Zarezerwować przegląd auta", { due: d(4) }),
    task(inbox, "z7y6x5", "Przeczytać artykuł o CRDT", {
      links: ["https://martin.kleppmann.com/2020/07/06/crdt-hard-parts-hilbert-hotel.html"],
    }),
    task(inbox, "q1w2e3", "Zamówić filtry do oczyszczacza", { status: "done", done: d(-2) }),

    task(pr("whenubuy"), "r4t5y6", "Odnowić certyfikat domeny whenubuy.pl", { due: d(2) }),
    task(pr("whenubuy"), "u7i8o9", "Uporządkować tablicę w Linear", { priority: "low" }),

    task(tp("whenubuy", "ab-testy-cen"), "g7h8i9", "Przygotować dashboard konwersji dla wariantu B", {
      status: "doing",
      priority: "high",
      due: `${today}T14:00`,
      links: ["https://gitlab.com/whenubuy/backend/-/merge_requests/42"],
      notes: "Metryki: CR, AOV, odsetek porzuconych koszyków.\nPorównać z tym samym tygodniem w zeszłym roku.",
    }),
    task(tp("whenubuy", "ab-testy-cen"), "j1k2l3", "Ustalić próg istotności statystycznej z Tomkiem", { due: d(1) }),
    task(tp("whenubuy", "ab-testy-cen"), "m4n5b6", "Dodać flagę wariantu do eventów analitycznych", {
      status: "done",
      done: d(-1),
    }),
    task(tp("whenubuy", "ab-testy-cen"), "v7c8x9", "Skonfigurować podział ruchu 80/20 w feature flagach", {
      status: "done",
      done: d(-3),
    }),

    task(tp("whenubuy", "platnosci-stripe-przelewy24"), "s1d2f3", "Wysłać dokumenty rejestrowe do Przelewy24", {
      status: "blocked",
      notes: "Brakuje aktualnego odpisu z KRS.",
    }),
    task(tp("whenubuy", "platnosci-stripe-przelewy24"), "h4j5k6", "Obsłużyć webhook zwrotów (refund)", { due: d(-2) }),
    task(tp("whenubuy", "platnosci-stripe-przelewy24"), "w2e3r4", "Testy end-to-end płatności BLIK na sandboksie"),

    task(tp("whenubuy", "powiadomienia-push"), "t5y6u7", "Odpowiedzieć na uwagi z App Store Review", {
      due: d(0),
      links: ["https://appstoreconnect.apple.com/apps/123/appstore"],
    }),

    task(tp("whenubuy", "refaktoryzacja-koszyka"), "l8k7j6", "Spisać obecne przepływy stanu koszyka (web, iOS, Android) i miejsca, w których się rozjeżdżają", {
      priority: "low",
    }),

    task(tp("whenubuy", "onboarding"), "o1n2b3", "Ekran powitalny z wyborem kategorii", { status: "done", done: d(-21) }),

    task(tp("montybrain", "aplikacja-desktopowa"), "c3v4b5", "Widok projektu z rozwijanymi tematami", { status: "doing" }),
    task(tp("montybrain", "aplikacja-desktopowa"), "n6m7q8", "Tryb mock w przeglądarce do zrzutów ekranu", { status: "done", done: d(0) }),
    task(tp("montybrain", "aplikacja-desktopowa"), "e9r0t1", "Ikona w pasku menu z licznikiem", { priority: "low" }),

    task(tp("montybrain", "serwer-mcp"), "y2u3i4", "Narzędzie add_journal_entry z opcjonalną zmianą statusu", { status: "doing" }),
    task(tp("montybrain", "serwer-mcp"), "a5s6d7", "Opisać instalację w Claude Code", { due: d(3) }),

    task(tp("remont-kuchni", "wybor-blatu"), "f8g9h0", "Zadzwonić do kamieniarza", { status: "blocked" }),
  ];

  const j = (project: string, topicSlug: string, when: string, author: string, text: string): JournalEntry => ({
    project,
    topic: topicSlug,
    at: when,
    author,
    text,
    line: ++line,
  });

  const journal: JournalEntry[] = [
    j("montybrain", "aplikacja-desktopowa", at(0, nowTime), "claude-code", "Widok Dziś i lista tematów działają na danych z mocka."),
    j("montybrain", "aplikacja-desktopowa", at(-1, "21:15"), "ja", "Ustalony wygląd: ciemny, minimalny, kolor projektu jako jedyny akcent."),
    j("montybrain", "serwer-mcp", at(0, "08:12"), "agent", "Dodane narzędzia list_tasks i update_task. Testy zielone."),
    j("whenubuy", "ab-testy-cen", at(0, "09:41"), "agent", "Wariant B: +3,1% konwersji po 4 dniach, przedział ufności jeszcze szeroki."),
    j("whenubuy", "ab-testy-cen", at(-2, "15:30"), "ja", "Ruch podzielony 80/20, flagi włączone na produkcji."),
    j(
      "whenubuy",
      "platnosci-stripe-przelewy24",
      at(-1, "16:20"),
      "ja",
      "Przelewy24 poprosiło o aktualny odpis z KRS — zamówione, będzie w poniedziałek.",
    ),
    j("whenubuy", "platnosci-stripe-przelewy24", at(-4, "12:00"), "agent", "Sandbox: płatności kartą i BLIK przechodzą, zwroty jeszcze nie."),
    j("whenubuy", "powiadomienia-push", at(-3, "11:05"), "ja", "Build 2.5.0 wysłany do review."),
    j("remont-kuchni", "wybor-blatu", at(-12, "19:00"), "ja", "Wybrane dwie próbki, czekam na wycenę."),
  ];

  return {
    root: "/Users/ja/MontyBrain",
    projects,
    topics,
    tasks,
    journal,
    warnings: [
      { path: "whenubuy/tematy/ab-testy-cen.md", line: 14, message: "invalid due date `@2026-02-30`; kept as title text" },
      { path: "skrzynka.md", line: 3, message: "checkbox `[?]` is not a task status; line kept as is" },
    ],
  };
}
