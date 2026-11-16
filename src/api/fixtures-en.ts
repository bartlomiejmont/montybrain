// English demo data for the browser mock (`?demo=en`). Used for the screenshots
// on the landing page; the default fixtures stay Polish (see fixtures.ts).

import type { JournalEntry, Project, Snapshot, Task, TaskLocation, Topic } from "./types";
import { addDays } from "../lib/dates";

export function buildEnglishFixtures(today: string, now: string): Snapshot {
  const d = (n: number) => addDays(today, n);
  const at = (days: number, time: string) => `${d(days)}T${time}`;
  const nowTime = now.slice(11, 16);

  const projects: Project[] = [
    {
      slug: "pricewatch",
      name: "Pricewatch",
      color: "#a3e635",
      state: "active",
      repos: [
        { name: "pricewatch-backend", path: "/Users/me/Projects/pricewatch/pricewatch-backend" },
        { name: "pricewatch-mobile", path: "/Users/me/Projects/pricewatch/pricewatch-mobile" },
      ],
      description: "Price tracker with history and alerts. Kotlin backend, React Native app.",
      path: "pricewatch/projekt.md",
    },
    {
      slug: "montybrain",
      name: "MontyBrain",
      color: "#818cf8",
      state: "active",
      repos: [{ name: "montybrain", path: "/Users/me/Projects/montybrain" }],
      description: "A local command center: projects, topics, and tasks in Markdown files.",
      path: "montybrain/projekt.md",
    },
    {
      slug: "kitchen-remodel",
      name: "Kitchen remodel",
      color: "#fb923c",
      state: "paused",
      repos: [],
      description: "",
      path: "kitchen-remodel/projekt.md",
    },
    {
      slug: "old-blog",
      name: "Old blog",
      color: null,
      state: "archived",
      repos: [],
      description: "A Jekyll blog, replaced by notes.",
      path: "old-blog/projekt.md",
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
      "pricewatch",
      "pricing-ab-test",
      "Pricing A/B test",
      "in_progress",
      "Variant B (prices ending in .99) runs on 20% of traffic. One more week of data, then we decide on the rollout.",
      at(0, "09:41"),
      "pricewatch-backend",
    ),
    topic(
      "pricewatch",
      "payments-migration",
      "Payments: new provider",
      "blocked",
      "The integration works in the sandbox. Blocked by business account verification on the provider's side.",
      at(-1, "16:20"),
      "pricewatch-backend",
    ),
    topic(
      "pricewatch",
      "push-notifications",
      "Push notifications in the mobile app",
      "waiting",
      "Waiting for App Store review.",
      at(-3, "11:05"),
      "pricewatch-mobile",
    ),
    topic("pricewatch", "cart-refactor", "Cart module refactor", "planned", "", at(-9, "10:00")),
    topic("pricewatch", "onboarding", "New user onboarding", "done", "Shipped in version 2.4.", at(-20, "18:30")),
    topic("pricewatch", "release-2-5", "Release 2.5", "done", "Published yesterday. Monitoring is in place.", at(-1, "12:00")),
    topic(
      "montybrain",
      "desktop-app",
      "Desktop app",
      "in_progress",
      "Tauri shell, Today and project views, full editing.",
      at(0, nowTime),
      "montybrain",
    ),
    topic(
      "montybrain",
      "mcp-server",
      "MCP server",
      "in_progress",
      "Tools that let agents read and update tasks.",
      at(0, "08:12"),
      "montybrain",
    ),
    topic("kitchen-remodel", "countertop", "Choosing a countertop", "waiting", "Waiting for a quote on the quartz option.", at(-12, "19:00")),
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
    task(inbox, "a1b2c3", "Reply to Kate about the October invoice", { due: d(-1), priority: "high" }),
    task(inbox, "k9x2m1", "Compare VPS hosting offers in Frankfurt"),
    task(inbox, "p0o9i8", "Book the car inspection", { due: d(4) }),
    task(inbox, "z7y6x5", "Read the article about CRDTs", {
      links: ["https://martin.kleppmann.com/2020/07/06/crdt-hard-parts-hilbert-hotel.html"],
    }),
    task(inbox, "q1w2e3", "Order air purifier filters", { status: "done", done: d(-2) }),

    task(pr("pricewatch"), "r4t5y6", "Renew the pricewatch.app domain certificate", { due: d(2) }),
    task(pr("pricewatch"), "u7i8o9", "Tidy up the issue tracker", { priority: "low" }),

    task(tp("pricewatch", "pricing-ab-test"), "g7h8i9", "Build the conversion dashboard for variant B", {
      status: "doing",
      priority: "high",
      due: `${today}T14:00`,
      links: ["https://gitlab.com/pricewatch/backend/-/merge_requests/42"],
      notes: "Metrics: CR, AOV, cart abandonment rate.\nCompare with the same week last year.",
    }),
    task(tp("pricewatch", "pricing-ab-test"), "j1k2l3", "Agree on the significance threshold with Tom", { due: d(1) }),
    task(tp("pricewatch", "pricing-ab-test"), "m4n5b6", "Add the variant flag to analytics events", {
      status: "done",
      done: d(-1),
    }),
    task(tp("pricewatch", "pricing-ab-test"), "v7c8x9", "Set up the 80/20 traffic split in feature flags", {
      status: "done",
      done: d(-3),
    }),

    task(tp("pricewatch", "payments-migration"), "s1d2f3", "Send company documents to the payment provider", {
      status: "blocked",
      notes: "The current company registry extract is missing.",
    }),
    task(tp("pricewatch", "payments-migration"), "h4j5k6", "Handle the refund webhook", { due: d(-2) }),
    task(tp("pricewatch", "payments-migration"), "w2e3r4", "End-to-end payment tests in the sandbox"),

    task(tp("pricewatch", "push-notifications"), "t5y6u7", "Respond to App Store Review feedback", {
      due: d(0),
      links: ["https://appstoreconnect.apple.com/apps/123/appstore"],
    }),

    task(tp("pricewatch", "cart-refactor"), "l8k7j6", "Map the current cart state flows on web, iOS, and Android", {
      priority: "low",
    }),

    task(tp("pricewatch", "onboarding"), "o1n2b3", "Welcome screen with category picker", { status: "done", done: d(-21) }),

    task(tp("montybrain", "desktop-app"), "c3v4b5", "Project view with expandable topics", { status: "doing" }),
    task(tp("montybrain", "desktop-app"), "n6m7q8", "Browser mock mode for screenshots", { status: "done", done: d(0) }),
    task(tp("montybrain", "desktop-app"), "e9r0t1", "Menu bar icon with a counter", { priority: "low" }),

    task(tp("montybrain", "mcp-server"), "y2u3i4", "log_update tool with an optional status change", { status: "doing" }),
    task(tp("montybrain", "mcp-server"), "a5s6d7", "Document the Claude Code setup", { due: d(3) }),

    task(tp("kitchen-remodel", "countertop"), "f8g9h0", "Call the stonemason", { status: "blocked" }),
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
    j("montybrain", "desktop-app", at(0, nowTime), "claude-code", "The Today view and the topic list run on mock data."),
    j("montybrain", "desktop-app", at(-1, "21:15"), "me", "Look agreed: dark, minimal, the project colour as the only accent."),
    j("montybrain", "mcp-server", at(0, "08:12"), "agent", "Added the list_tasks and update_task tools. Tests are green."),
    j("pricewatch", "pricing-ab-test", at(0, "09:41"), "agent", "Variant B: +3.1% conversion after 4 days; the confidence interval is still wide."),
    j("pricewatch", "pricing-ab-test", at(-2, "15:30"), "me", "Traffic split 80/20, flags enabled in production."),
    j(
      "pricewatch",
      "payments-migration",
      at(-1, "16:20"),
      "me",
      "The provider asked for a current registry extract. Ordered; it arrives on Monday.",
    ),
    j("pricewatch", "payments-migration", at(-4, "12:00"), "agent", "Sandbox: card payments go through, refunds do not yet."),
    j("pricewatch", "push-notifications", at(-3, "11:05"), "me", "Build 2.5.0 sent for review."),
    j("kitchen-remodel", "countertop", at(-12, "19:00"), "me", "Two samples picked; waiting for the quote."),
  ];

  return {
    root: "/Users/me/MontyBrain",
    projects,
    topics,
    tasks,
    journal,
    warnings: [],
  };
}
