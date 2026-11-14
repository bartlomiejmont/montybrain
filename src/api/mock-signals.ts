// Mock of the drift detector and the waiting schedule (src-tauri/src/drift/,
// src-tauri/src/waiting.rs) for the browser and tests. The drift data is a
// fixed, synthetic collection for project „whenubuy”; links and ignores
// change it like the real sidecar does. The waiting schedule follows the mock
// vault the way `reconcile` does.

import type { DriftBranch, ProjectDrift, Snapshot, WaitView } from "./types";
import { addDays } from "../lib/dates";

export interface SignalsOptions {
  today: () => string;
  now: () => string;
  snapshot: () => Snapshot;
  waitingDays: () => number;
  /** `?drift=none`: no collection yet. */
  drift?: "none" | "default";
}

interface Branch {
  repo: string;
  branch: string;
  days: number;
  worktree?: string;
  topic?: string;
}

const PROJECT = "whenubuy";

/** Synthetic branches of the fixture project (days = age of the last commit). */
const BRANCHES: Branch[] = [
  { repo: "whenubuy-backend", branch: "feature/p24-refunds", days: 0, topic: "platnosci-stripe-przelewy24" },
  { repo: "whenubuy-backend", branch: "feature/blik-e2e", days: 1, topic: "platnosci-stripe-przelewy24" },
  { repo: "whenubuy-mobile", branch: "feature/p24-webhooks", days: 2, topic: "platnosci-stripe-przelewy24" },
  { repo: "whenubuy-backend", branch: "feature/ab-ceny-dashboard", days: 0, topic: "ab-testy-cen" },
  { repo: "whenubuy-mobile", branch: "push-notifications-ios", days: 1, topic: "powiadomienia-push" },
  { repo: "whenubuy-mobile", branch: "feature/loyalty", days: 1, worktree: "/Users/ja/Projects/WhenUBuy/whenubuy-mobile-loyalty" },
  { repo: "whenubuy-backend", branch: "spike/graphql", days: 3 },
  { repo: "whenubuy-backend", branch: "chore/stara-migracja", days: 40 },
];

export function createMockSignals(opts: SignalsOptions) {
  const links = new Map<string, string>();
  const ignored = new Set<string>();
  const schedule = new Map<string, { next: string; interval: number; confirmed: string | null; since: string }>();
  const driftSubs = new Set<() => void>();
  const waitSubs = new Set<() => void>();
  const key = (repo: string, branch: string) => `${repo}|${branch}`;
  const fireDrift = () => setTimeout(() => driftSubs.forEach((f) => f()), 0);
  const fireWait = () => setTimeout(() => waitSubs.forEach((f) => f()), 0);

  // The fixture's links (what an applied proposal would have stored).
  for (const b of BRANCHES) if (b.topic && b.topic.startsWith("platnosci")) links.set(key(b.repo, b.branch), b.topic);

  function drift(): ProjectDrift[] {
    const snap = opts.snapshot();
    const today = opts.today();
    const out: ProjectDrift[] = [];
    for (const p of snap.projects.filter((x) => x.state === "active" && x.repos.length > 0)) {
      const d: ProjectDrift = {
        project: p.slug,
        collected_at: null,
        branches: [],
        unknown: [],
        spread: [],
        moved: [],
        new_commits: null,
        since: null,
        links: [],
      };
      out.push(d);
      if (p.slug !== PROJECT || opts.drift === "none") continue;
      d.collected_at = `${today}T${opts.now().slice(11, 16)}`;
      d.since = `${addDays(today, -1)}T18:00`;
      d.new_commits = 12;
      const topics = snap.topics.filter((t) => t.project === p.slug);
      for (const b of BRANCHES) {
        const k = key(b.repo, b.branch);
        const linked = links.get(k);
        const topic = linked ?? (b.topic && !b.topic.startsWith("platnosci") ? b.topic : undefined);
        const last = addDays(today, -b.days);
        const row: DriftBranch = {
          repo: b.repo,
          branch: b.branch,
          last_commit: last,
          worktree: b.worktree ?? null,
          topic: topic && topics.some((t) => t.slug === topic) ? topic : null,
          matched_by: linked ? "link" : topic ? "words" : null,
          active: b.days <= 14 || !!b.worktree,
          ignored: ignored.has(k),
          seen: d.collected_at,
        };
        d.branches.push(row);
      }
      d.unknown = d.branches.filter((b) => b.active && !b.topic && !b.ignored);
      const per = new Map<string, DriftBranch[]>();
      for (const b of d.branches.filter((x) => x.active && x.topic)) per.set(b.topic!, [...(per.get(b.topic!) ?? []), b]);
      for (const [topic, bs] of per) {
        const repos = new Set(bs.map((b) => b.repo)).size;
        if (bs.length >= 3 || repos >= 2) d.spread.push({ topic, branches: bs.length, repos });
      }
      for (const b of d.branches.filter((x) => x.active && x.topic)) {
        const lastEntry = snap.journal
          .filter((j) => j.project === p.slug && j.topic === b.topic)
          .map((j) => j.at.slice(0, 10))
          .sort()
          .pop();
        if ((!lastEntry || b.last_commit > lastEntry) && !d.moved.some((m) => m.topic === b.topic)) {
          d.moved.push({ topic: b.topic!, repo: b.repo, branch: b.branch, last_commit: b.last_commit, last_entry: lastEntry ?? null });
        }
      }
      d.links = [...links].map(([k, topic]) => {
        const [repo, branch] = k.split("|") as [string, string];
        return { project: p.slug, repo, branch, topic };
      });
    }
    return out;
  }

  /** Mirrors `waiting::targets` + `reconcile` + `views`. */
  function waiting(): WaitView[] {
    const snap = opts.snapshot();
    const today = opts.today();
    const now = opts.now();
    const active = new Set(snap.projects.filter((p) => p.state === "active").map((p) => p.slug));
    const strip = (t: string) => {
      for (const p of ["Czeka: ", "Zablokowany: ", "Zablokowane: "]) if (t.trim().startsWith(p)) return t.trim().slice(p.length).split("\n")[0]!.trim() || null;
      return null;
    };
    const targets: Omit<WaitView, "next" | "interval_days" | "confirmed" | "since" | "due">[] = [];
    for (const t of snap.topics) {
      if ((t.status !== "waiting" && t.status !== "blocked") || !active.has(t.project)) continue;
      const reason =
        snap.journal
          .filter((j) => j.project === t.project && j.topic === t.slug)
          .sort((a, b) => b.at.localeCompare(a.at))
          .map((j) => strip(j.text))
          .find(Boolean) ?? null;
      targets.push({ key: `topic:${t.project}/${t.slug}`, kind: "topic", project: t.project, topic: t.slug, task: null, title: t.title, status: t.status, reason });
    }
    for (const t of snap.tasks) {
      if (t.status !== "blocked" || !t.id) continue;
      const project = t.location.kind === "inbox" ? null : t.location.project;
      if (project && !active.has(project)) continue;
      const reason = t.notes.split("\n").reverse().map(strip).find(Boolean) ?? null;
      targets.push({ key: `task:${t.id}`, kind: "task", project, topic: t.location.kind === "topic" ? t.location.topic : null, task: t.id, title: t.title, status: "blocked", reason });
    }
    for (const k of [...schedule.keys()]) if (!targets.some((t) => t.key === k)) schedule.delete(k);
    for (const t of targets) {
      if (!schedule.has(t.key)) {
        // The fixture's waiting items: one due today, the others later.
        const preset: Record<string, number> = { "topic:whenubuy/platnosci-stripe-przelewy24": 0, "topic:whenubuy/powiadomienia-push": 1, "topic:remont-kuchni/wybor-blatu": 3, "topic:whenubuy/wydanie-android": 2 };
        const days = preset[t.key] ?? opts.waitingDays();
        schedule.set(t.key, { next: addDays(today, days), interval: opts.waitingDays(), confirmed: null, since: `${addDays(today, -(t.key.includes("wybor") ? 12 : 2))}T10:00` });
      }
    }
    void now;
    return targets.map((t) => {
      const s = schedule.get(t.key)!;
      return { ...t, next: s.next, interval_days: s.interval, confirmed: s.confirmed, since: s.since, due: s.next <= today };
    });
  }

  return {
    async listDrift() {
      return drift();
    },
    async refreshDrift() {
      fireDrift();
      return true;
    },
    async linkBranch(project: string, repo: string, branch: string, topic: string) {
      if (!opts.snapshot().topics.some((t) => t.project === project && t.slug === topic)) throw new Error("W tym projekcie nie ma takiego tematu.");
      links.set(key(repo, branch), topic);
      ignored.delete(key(repo, branch));
      fireDrift();
      return drift();
    },
    async ignoreBranch(_project: string, repo: string, branch: string) {
      links.delete(key(repo, branch));
      ignored.add(key(repo, branch));
      fireDrift();
      return drift();
    },
    /** Applying a proposal with `branches` stores links (the mock updates call this). */
    link(repo: string, branch: string, topic: string) {
      links.set(key(repo, branch), topic);
      fireDrift();
    },
    async onDriftChanged(cb: () => void) {
      driftSubs.add(cb);
      return () => void driftSubs.delete(cb);
    },
    async listWaiting() {
      return waiting();
    },
    async setWaiting(k: string, days: number) {
      waiting();
      if (![1, 2, 3, 7].includes(days)) throw new Error(`Nieprawidłowy odstęp przypomnień: ${days} dni (dozwolone 1, 2, 3, 7).`);
      const s = schedule.get(k);
      if (!s) throw new Error("Ta pozycja nie czeka (zmień najpierw jej status).");
      schedule.set(k, { ...s, next: addDays(opts.today(), days), interval: days });
      fireWait();
      return waiting();
    },
    async confirmWaiting(k: string) {
      waiting();
      const s = schedule.get(k);
      if (!s) throw new Error("Ta pozycja już nie czeka.");
      schedule.set(k, { ...s, next: addDays(opts.today(), s.interval), confirmed: opts.now() });
      fireWait();
      return waiting();
    },
    async onWaitingChanged(cb: () => void) {
      waitSubs.add(cb);
      return () => void waitSubs.delete(cb);
    },
  };
}
