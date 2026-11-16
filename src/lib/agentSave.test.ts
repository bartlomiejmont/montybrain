import { describe, expect, it } from "vitest";

import type { Agents } from "../api/types";
import { CUSTOM_MODEL, createAgentSaver, modelSelection, modelToSave, selectedModel } from "./agentSave";

const empty: Agents = { claude: { model: null, effort: null }, codex: { model: null, effort: null } };

describe("Settings → Agenci saving", () => {
  it("keeps both a model and an effort saved right after each other", async () => {
    const sent: Agents[] = [];
    let stored = empty;
    const send = async (next: Agents) => {
      sent.push(next);
      await new Promise((r) => setTimeout(r, 5));
      stored = next;
      return stored;
    };
    const saver = createAgentSaver(send, empty);
    // Blur of the model field and the effort select in the same moment.
    const a = saver.save("claude", { model: "opus" });
    const b = saver.save("claude", { effort: "high" });
    await Promise.all([a, b]);
    expect(stored.claude).toEqual({ model: "opus", effort: "high" });
    expect(sent).toHaveLength(2);
    expect(sent[1]!.claude).toEqual({ model: "opus", effort: "high" });
  });

  it("a failed save does not block the next one", async () => {
    let fail = true;
    const saver = createAgentSaver(async (next) => {
      if (fail) {
        fail = false;
        throw new Error("Nieprawidłowy model");
      }
      return next;
    }, empty);
    await expect(saver.save("codex", { model: "zły model" })).rejects.toThrow("Nieprawidłowy model");
    await expect(saver.save("codex", { effort: "low" })).resolves.toMatchObject({ codex: { model: null, effort: "low" } });
  });

  it("knows when the model field has nothing new", () => {
    expect(modelToSave(" opus ", null)).toBe("opus");
    expect(modelToSave("opus", "opus")).toBeUndefined();
    expect(modelToSave("  ", "opus")).toBeNull();
    expect(modelToSave("", null)).toBeUndefined();
  });
});


describe("model list mapping", () => {
  it("preserves default, known and saved custom ids", () => {
    expect(modelSelection(null, ["opus"])).toBe("");
    expect(modelSelection("opus", ["opus"])).toBe("opus");
    expect(modelSelection("private-model", ["opus"])).toBe(CUSTOM_MODEL);
    expect(modelToSave("private-model", "private-model")).toBeUndefined();
    expect(selectedModel(CUSTOM_MODEL)).toBeUndefined();
    expect(selectedModel("")).toBeNull();
    expect(selectedModel("opus")).toBe("opus");
  });
});
