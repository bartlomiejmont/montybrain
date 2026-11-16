import { describe, expect, it } from "vitest";

import { topicSummary } from "./topicSummary";

describe("readable topic summaries", () => {
  it("surfaces the existing conclusion and keeps all supporting detail", () => {
    expect(topicSummary("Backend: gałąź feature/qr (12.11), commit abc123. Wniosek: Integracja czeka na test całości.")).toEqual({ lead: "Integracja czeka na test całości.", details: "Backend: gałąź feature/qr (12.11), commit abc123." });
  });

  it("handles formatted conclusions without leaving Markdown markers behind", () => {
    expect(topicSummary("Dowody w repozytorium.\n\n**Wniosek:** Brak potwierdzenia wdrożenia.")).toEqual({ lead: "Brak potwierdzenia wdrożenia.", details: "Dowody w repozytorium." });
  });

  it("keeps a short plain description intact and never manufactures a conclusion", () => {
    expect(topicSummary("Czekamy na odpowiedź dostawcy.")).toEqual({ lead: "Czekamy na odpowiedź dostawcy.", details: "" });
    const text = "Stan niepotwierdzony. " + "Szczegóły techniczne. ".repeat(30);
    expect(topicSummary(text)).toEqual({ lead: "Stan niepotwierdzony.", details: text.trim() });
  });

  it("does not drop an unfinished conclusion or an empty description", () => {
    expect(topicSummary("Sprawdzenie trwa. Wniosek:")).toEqual({ lead: "Sprawdzenie trwa. Wniosek:", details: "" });
    expect(topicSummary("   ")).toEqual({ lead: "", details: "" });
  });
});
