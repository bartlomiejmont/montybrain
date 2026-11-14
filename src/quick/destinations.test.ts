import { describe, expect, it } from "vitest";

import { buildFixtures } from "../api/fixtures";
import { destinationKey, destinations, filterDestinations, resolveDestination } from "./destinations";

const snap = buildFixtures("2026-11-13", "2026-11-13T09:00");

describe("quick-capture destinations", () => {
  const all = destinations(snap);

  it("starts with the inbox, skips archived projects and done topics", () => {
    expect(all[0]!.key).toBe("inbox");
    const archived = snap.projects.filter((p) => p.state === "archived").map((p) => p.slug);
    for (const d of all) {
      if (d.location.kind !== "inbox") expect(archived).not.toContain(d.location.project);
      if (d.location.kind === "topic") {
        const loc = d.location;
        const t = snap.topics.find((x) => x.project === loc.project && x.slug === loc.topic)!;
        expect(t.status).not.toBe("done");
        expect(d.depth).toBe(1);
      }
    }
    // Topics follow their project.
    const firstTopic = all.findIndex((d) => d.depth === 1);
    expect(all[firstTopic - 1]!.location.kind).not.toBe("inbox");
  });

  it("filters ignoring case and diacritics", () => {
    const topic = all.find((d) => d.depth === 1)!;
    const word = topic.label.split(" ")[0]!.toUpperCase();
    expect(filterDestinations(all, word).map((d) => d.key)).toContain(topic.key);
    expect(filterDestinations(all, "skrzynka").map((d) => d.key)).toEqual(["inbox"]);
    expect(filterDestinations(all, "zzz-nic")).toEqual([]);
    expect(filterDestinations(all, " ")).toBe(all);
  });

  it("falls back to the inbox when the remembered place is gone", () => {
    const topic = all.find((d) => d.depth === 1)!;
    expect(resolveDestination(all, topic.location).key).toBe(topic.key);
    expect(resolveDestination(all, { kind: "topic", project: "nie-ma", topic: "x" }).key).toBe("inbox");
    expect(resolveDestination(all, null).key).toBe("inbox");
    expect(destinationKey({ kind: "project", project: "a" })).toBe("p:a");
  });
});
