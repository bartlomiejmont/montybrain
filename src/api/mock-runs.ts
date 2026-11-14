// Mock of the agent-run backend (recipes, providers, timed runs), used in a plain
// browser and in tests. Behaviour that can be provoked from the UI:
//  - text containing "fail" / "awaria" → the run fails after a few lines;
//  - text containing "dużo" / "many"   → ~400 log lines before the answer;
//  - `codex: "missing"` option (URL `?codex=off`) → Codex is unavailable with a `problem`;
//  - `claude: "missing"` (URL `?update=unavailable`) → Claude Code is unavailable too;
//  - recipe `aktualizacja` ends with a `montybrain-zmiany` block; the proposal itself is
//    built by mock-updates.ts from the fixtures when the run ends;
//  - only one background run at a time (a second start rejects).

import type { AgentChoice, ProviderId, ProviderInfo, Recipe, RunEvent, RunInfo, RunRequest } from "./types";

export interface MockRunOptions {
  /** Delay between scripted log lines (ms). 0 = next macrotask. Default 700. */
  stepMs?: number;
  codex?: "ok" | "missing";
  claude?: "ok" | "missing";
  now?: () => Date;
  /** The model / effort from Settings → "Agenci" (the mock's settings). */
  agentFor?: (provider: ProviderId) => AgentChoice;
}

const pad = (n: number) => String(n).padStart(2, "0");
export function isoSeconds(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export const MOCK_RECIPES: Recipe[] = [
  {
    id: "postep",
    title: "Otwarte zadania i postęp",
    description: "Przegląda otwarte zadania i dziennik, i opisuje, na czym stoisz.",
    skill: "montybrain-podsumowanie",
    modes: ["background", "terminal"],
    background_writes: false,
    proposes: false,
    needs_repos: false,
    params: [{ key: "project", label: "Projekt", kind: "project", required: false, placeholder: null, default: null }],
  },
  {
    id: "raport",
    title: "Raport z dnia lub tygodnia",
    description: "Składa raport z dziennika i zamkniętych zadań.",
    skill: "montybrain-podsumowanie",
    modes: ["background", "terminal"],
    background_writes: false,
    proposes: false,
    needs_repos: false,
    params: [
      { key: "period", label: "Okres", kind: "period", required: true, placeholder: null, default: "dzis" },
      { key: "project", label: "Projekt", kind: "project", required: false, placeholder: null, default: null },
    ],
  },
  {
    id: "zadania",
    title: "Zadania z notatek",
    description: "Zamienia wklejone notatki na zadania w MontyBrain.",
    skill: "montybrain-zadania",
    modes: ["background", "terminal"],
    background_writes: true,
    proposes: false,
    needs_repos: false,
    params: [
      { key: "text", label: "Notatki", kind: "text", required: true, placeholder: "Wklej notatki ze spotkania, listę rzeczy do zrobienia…", default: null },
      { key: "project", label: "Projekt", kind: "project", required: false, placeholder: null, default: null },
    ],
  },
  {
    id: "poczta",
    title: "Poczta → zadania",
    description: "Przeszukuje pocztę i proponuje z niej zadania. Wymaga terminala.",
    skill: "montybrain-poczta",
    modes: ["terminal"],
    background_writes: false,
    proposes: false,
    needs_repos: false,
    params: [
      { key: "text", label: "Czego szukać", kind: "text", required: false, placeholder: "Np. wiadomości od księgowej, faktury (opcjonalnie)", default: null },
      { key: "period", label: "Okres", kind: "period", required: true, placeholder: null, default: "tydzien" },
    ],
  },
  {
    id: "aktualizacja",
    title: "Aktualizacja projektu",
    description: "Porównuje repozytoria z tematami i zadaniami; proponuje nowe tematy i zmiany do akceptacji.",
    skill: "montybrain-postep",
    modes: ["background", "terminal"],
    background_writes: false,
    proposes: true,
    needs_repos: true,
    params: [{ key: "project", label: "Projekt", kind: "project", required: true, placeholder: "Wybierz projekt", default: null }],
  },
  {
    id: "zlecenie",
    title: "Zlecenie dla agenta",
    description: "Twoje polecenie dla projektu, np. rozpisanie tematu na podstawie historii rozmów. Agent korzysta z MontyBrain, repozytoriów i historii rozmów; zmiany proponuje do akceptacji.",
    skill: "montybrain-postep",
    modes: ["background", "terminal"],
    background_writes: false,
    proposes: true,
    needs_repos: false,
    params: [
      { key: "project", label: "Projekt", kind: "project", required: true, placeholder: "Wybierz projekt", default: null },
      { key: "text", label: "Zlecenie", kind: "text", required: true, placeholder: "np. rozpisz temat płatności na podstawie historii rozmów", default: null },
    ],
  },
];

export function mockProviders(codex: "ok" | "missing", claude: "ok" | "missing" = "ok"): ProviderInfo[] {
  return [
    claude === "ok"
      ? { id: "claude", name: "Claude Code", available: true, path: "/usr/local/bin/claude", version: "2.1.0", problem: null }
      : { id: "claude", name: "Claude Code", available: false, path: null, version: null, problem: "Nie znaleziono polecenia „claude”. Zainstaluj Claude Code i zaloguj się." },
    codex === "ok"
      ? { id: "codex", name: "Codex", available: true, path: "/usr/local/bin/codex", version: "0.58.0", problem: null }
      : { id: "codex", name: "Codex", available: false, path: null, version: null, problem: "Nie znaleziono polecenia „codex”. Zainstaluj Codex CLI i zaloguj się." },
  ];
}

export interface Step {
  kind: RunEvent["kind"];
  text: string;
}

function script(req: RunRequest, providerName: string): Step[] {
  const project = req.params.project || "";
  const scope = project ? `projekt ${project}` : "wszystkie aktywne projekty";
  const bulk = /dużo|many/i.test(req.params.text ?? "");
  const steps: Step[] = [
    { kind: "note", text: `Uruchamiam agenta (${providerName}) — ${scope}.` },
    { kind: "tool", text: "overview {}" },
    { kind: "text", text: "Mam ogólny obraz projektów. Sprawdzam szczegóły." },
    { kind: "tool", text: `list_tasks {"project":"${project || "whenubuy"}","status":"open"}` },
    { kind: "text", text: "Znalazłem 9 otwartych zadań, 3 są w toku." },
    { kind: "tool", text: `recent_journal {"project":"${project || "whenubuy"}","limit":10}` },
    { kind: "note", text: "Dziennik przeczytany, składam odpowiedź." },
  ];
  if (req.recipe === "zlecenie") {
    steps.splice(1, 0, { kind: "note", text: "Zbieram stan repozytoriów (2; tylko odczyt)…" }, { kind: "note", text: "Historia rozmów: Zebrano 6 sesji (Claude Code 4, Codex 2), 14 KB." });
  }
  if (req.recipe === "zadania") {
    steps.splice(4, 0, { kind: "tool", text: 'add_task {"title":"Zadzwonić do księgowej w sprawie VAT","project":"whenubuy"}' });
  }
  if (bulk) {
    for (let i = 0; i < 400; i++) {
      steps.splice(4 + i, 0, i % 3 === 0 ? { kind: "tool", text: `get_task {"id":"t${i}"}` } : i % 3 === 1 ? { kind: "text", text: `Analizuję zadanie ${i}: wymaga decyzji, zanim ruszy dalej, oraz uzgodnienia terminu z zespołem.` } : { kind: "note", text: `Krok ${i}` });
    }
  }
  return steps;
}

function resultFor(req: RunRequest, recipeTitle: string): string {
  const scope = req.params.project ? `projekcie „${req.params.project}”` : "wszystkich aktywnych projektach";
  if (req.recipe === "raport") {
    const period = req.params.period === "tydzien" ? "tydzień" : "dzień";
    return `# Raport: ${period}\n\nW ${scope} zamknięto **4 zadania**, a dziennik zawiera 6 nowych wpisów.\n\n## Zrobione\n\n- Odnowiono certyfikat domeny \`whenubuy.pl\`\n- Wysłano dokumenty rejestrowe do Przelewy24\n  - potwierdzenie przyszło w ciągu godziny\n  - brak uwag po stronie banku\n- Zamknięto zaległe uwagi z App Store Review\n\n## Na jutro\n\n1. Dokończyć dashboard konwersji dla wariantu B\n2. Ustalić próg istotności statystycznej z Tomkiem\n3. Przejrzeć nowe zgłoszenia\n\nZmiana w \`BoardView.tsx\`:\n\n\`\`\`ts\nconst done = tasks.filter((t) => t.status === "done");\n\`\`\`\n\nŹródło: <script>alert(1)</script> [link](javascript:alert(1)) — pokazane jako tekst.`;
  }
  if (req.recipe === "aktualizacja") {
    return `W projekcie „${req.params.project}” jedno zadanie wygląda na skończone, jedno trwa.\n\n- **Webhook zwrotów** — commit \`a1c9f02\` „Obsługa refundów” na \`main\`, testy przechodzą.\n- **Testy BLIK** — gałąź \`feature/blik-e2e\`, bez merge.\n\n\`\`\`montybrain-zmiany\n{"changes": [{"kind": "task_status", "task": "h4j5k6", "status": "done", "reason": "Commit a1c9f02."}]}\n\`\`\``;
  }
  if (req.recipe === "zlecenie") {
    return `Nie mam w tym trybie dostępu do poczty ani Slacka, więc opieram się na MontyBrain, repozytoriach i historii rozmów.\n\n- W historii są dwie sesje o tym temacie (Claude Code z 10.11, Codex z 12.11).\n- W repozytorium: gałąź \`feature/blik-e2e\`, bez merge.\n\n\`\`\`montybrain-zmiany\n{"changes": []}\n\`\`\``;
  }
  if (req.recipe === "zadania") {
    return `Dodałem **3 zadania** w ${scope}:\n\n1. Zadzwonić do księgowej w sprawie VAT (jutro)\n2. Przygotować ofertę dla nowego klienta\n3. Odpisać na pytania z rekrutacji\n\nJedno zdanie z notatek było zbyt ogólne („ogarnąć sprawy”) i zostało pominięte.`;
  }
  return `## ${recipeTitle}\n\nW ${scope} jest **9 otwartych zadań**: 3 w toku, 1 zablokowane, 5 do zrobienia.\n\n**Na czym stoisz**\n\n- Płatności: Stripe → Przelewy24 — brakuje testów end-to-end płatności BLIK na sandboksie; webhook zwrotów czeka od 11 lis\n- A/B testy cen — dashboard dla wariantu B jest w toku, próg istotności do ustalenia z Tomkiem\n- Powiadomienia push — odpowiedzi do App Store Review są na dziś\n\n**Co bym zrobił najpierw**\n\n1. Zamknąć obsługę webhooka zwrotów (*refund*), bo blokuje testy\n2. Dokończyć dashboard, zanim zmienią się dane z eksperymentu\n3. Odpowiedzieć do App Store Review do końca dnia\n\nBlokada: wysyłka dokumentów rejestrowych do Przelewy24 czeka na podpis.`;
}

export function createMockRuns(opts: MockRunOptions = {}) {
  const now = opts.now ?? (() => new Date());
  const stepMs = opts.stepMs ?? 700;
  const providers = mockProviders(opts.codex ?? "ok", opts.claude ?? "ok");
  let nextId = 1;
  const runs: RunInfo[] = []; // newest first
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  const eventListeners = new Set<(e: RunEvent) => void>();
  const updatedListeners = new Set<(r: RunInfo) => void>();
  const copy = (r: RunInfo): RunInfo => structuredClone(r);

  const emitUpdated = (r: RunInfo) => {
    const payload = { ...copy(r), events: [] };
    setTimeout(() => updatedListeners.forEach((cb) => cb(structuredClone(payload))), 0);
  };
  const emitEvent = (r: RunInfo, kind: RunEvent["kind"], text: string) => {
    const e: RunEvent = { run_id: r.id, seq: r.events.length + 1, at: isoSeconds(now()), kind, text };
    r.events.push(e);
    eventListeners.forEach((cb) => cb({ ...e }));
  };
  const finish = (r: RunInfo, status: RunInfo["status"], extra: Partial<RunInfo>) => {
    timers.delete(r.id);
    Object.assign(r, { status, finished_at: isoSeconds(now()) }, extra);
    emitUpdated(r);
  };

  return {
    async listRecipes() {
      return structuredClone(MOCK_RECIPES);
    },
    async listProviders() {
      return structuredClone(providers);
    },
    async listRuns() {
      return runs.map(copy);
    },
    async startRun(req: RunRequest): Promise<RunInfo> {
      const recipe = MOCK_RECIPES.find((r) => r.id === req.recipe);
      if (!recipe) throw new Error(`unknown recipe: ${req.recipe}`);
      if (!recipe.modes.includes(req.mode)) throw new Error(`Ta akcja działa tylko w trybie: ${recipe.modes.join(", ")}.`);
      const provider = providers.find((p) => p.id === req.provider);
      if (!provider) throw new Error(`unknown provider: ${req.provider}`);
      if (!provider.available) throw new Error(`${provider.name} jest niedostępny: ${provider.problem ?? "brak polecenia"}`);
      for (const p of recipe.params) {
        if (p.required && !(req.params[p.key] ?? "").trim()) throw new Error(`invalid input: missing param ${p.key}`);
      }
      if (req.mode === "background" && runs.some((r) => r.mode === "background" && r.status === "running")) {
        throw new Error("Inna akcja w tle jest jeszcze w toku. Poczekaj na jej koniec albo ją przerwij.");
      }
      const at = isoSeconds(now());
      const run: RunInfo = {
        id: `run-${nextId++}`,
        recipe: recipe.id,
        recipe_title: recipe.title,
        provider: req.provider,
        mode: req.mode,
        project: req.params.project || null,
        request: recipe.id === "zlecenie" ? req.params.text?.trim() || null : null,
        model: opts.agentFor?.(req.provider).model ?? null,
        effort: opts.agentFor?.(req.provider).effort ?? null,
        status: req.mode === "terminal" ? "succeeded" : "running",
        started_at: at,
        finished_at: req.mode === "terminal" ? at : null,
        result: null,
        error: null,
        events: [],
      };
      runs.unshift(run);
      runs.length = Math.min(runs.length, 20);
      emitUpdated(run);
      if (req.mode === "terminal") return copy(run);

      const steps = script(req, provider.name);
      const fail = /\b(fail|awaria)\b/i.test(req.params.text ?? "");
      const failAt = fail ? 3 : -1;
      let i = 0;
      const tick = () => {
        if (run.status !== "running") return;
        if (i === failAt) {
          emitEvent(run, "error", "429: przekroczono limit żądań, agent przerywa pracę");
          finish(run, "failed", { error: `${provider.name} zakończył pracę z błędem (kod 1): przekroczono limit żądań.` });
          return;
        }
        const step = steps[i++];
        if (!step) {
          finish(run, "succeeded", { result: resultFor(req, recipe.title) });
          return;
        }
        emitEvent(run, step.kind, step.text);
        timers.set(run.id, setTimeout(tick, stepMs));
      };
      timers.set(run.id, setTimeout(tick, stepMs));
      return copy(run);
    },
    /** Mock-only: a background run that is already running (for screenshots). */
    seedRunning(req: RunRequest, elapsedSec: number, lines: Step[]): RunInfo {
      const recipe = MOCK_RECIPES.find((r) => r.id === req.recipe)!;
      const run: RunInfo = {
        id: `run-${nextId++}`,
        recipe: recipe.id,
        recipe_title: recipe.title,
        provider: req.provider,
        mode: "background",
        project: req.params.project || null,
        request: recipe.id === "zlecenie" ? req.params.text?.trim() || null : null,
        model: opts.agentFor?.(req.provider).model ?? null,
        effort: opts.agentFor?.(req.provider).effort ?? null,
        status: "running",
        started_at: isoSeconds(new Date(now().getTime() - elapsedSec * 1000)),
        finished_at: null,
        result: null,
        error: null,
        events: [],
      };
      runs.unshift(run);
      for (const l of lines) emitEvent(run, l.kind, l.text);
      return copy(run);
    },
    async cancelRun(runId: string): Promise<RunInfo> {
      const run = runs.find((r) => r.id === runId);
      if (!run) throw new Error(`run not found: ${runId}`);
      if (run.status === "running") {
        clearTimeout(timers.get(run.id));
        emitEvent(run, "note", "Przerwano na prośbę użytkownika.");
        finish(run, "cancelled", {});
      }
      return copy(run);
    },
    async onRunEvent(cb: (e: RunEvent) => void) {
      eventListeners.add(cb);
      return () => void eventListeners.delete(cb);
    },
    async onRunUpdated(cb: (r: RunInfo) => void) {
      updatedListeners.add(cb);
      return () => void updatedListeners.delete(cb);
    },
  };
}
