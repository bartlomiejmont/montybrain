// The small, skippable step after something starts waiting: "Na co czeka?"
// (written as a normal journal entry of the topic, or a line in the task's
// notes) and "Przypomnij za" (the check schedule, app state). Skipping keeps
// the default reminder the app schedules by itself.

import { useEffect, useRef, useState, useSyncExternalStore } from "react";

import type { Task } from "../api";
import { describeError } from "../lib/errors";
import { waitingStore, useWaitingDays } from "../store/signals";
import { useVault } from "../store/vault";
import { Button, Segmented } from "./ui";

export interface WaitTarget {
  key: string;
  kind: "topic" | "task";
  status: "waiting" | "blocked";
  project?: string;
  topic?: string;
  task?: Task;
}

// Which item's step is open, outside the rows: a topic that starts waiting
// moves to another group (and its row is re-created), the step stays open.
let openKey: string | null = null;
const openSubs = new Set<() => void>();
export function openWaitStep(key: string | null) {
  openKey = key;
  openSubs.forEach((f) => f());
}
export function useWaitStepOpen(key: string): boolean {
  return useSyncExternalStore(
    (f) => {
      openSubs.add(f);
      return () => void openSubs.delete(f);
    },
    () => openKey === key,
  );
}

const DAYS = [1, 2, 3, 7] as const;
const dayLabel = (d: number) => (d === 1 ? "1 dzień" : `${d} dni`);

/** The text written for a reason: "Czeka: …", "Zablokowany: …" (topic), "Zablokowane: …" (task). */
export function reasonText(t: Pick<WaitTarget, "kind" | "status">, reason: string): string {
  const r = reason.replace(/\s+/g, " ").trim();
  if (t.kind === "task") return `Zablokowane: ${r}`;
  return t.status === "blocked" ? `Zablokowany: ${r}` : `Czeka: ${r}`;
}

export function WaitingStep({ targets, onDone, title }: { targets: WaitTarget[]; onDone: () => void; title?: string }) {
  const { api, run, notify } = useVault();
  const defaultDays = useWaitingDays();
  const [reason, setReason] = useState("");
  const [days, setDays] = useState<number>(defaultDays);
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => ref.current?.focus(), []);
  useEffect(() => setDays(defaultDays), [defaultDays]);
  const blocked = targets.every((t) => t.status === "blocked");

  async function save() {
    setBusy(true);
    try {
      const r = reason.trim();
      for (const t of targets) {
        if (r && t.kind === "topic" && t.project && t.topic) {
          await run((a) => a.addJournalEntry(t.project!, t.topic!, { author: "ja", text: reasonText(t, r) }));
        } else if (r && t.kind === "task" && t.task?.id) {
          const notes = t.task.notes.trim() ? `${t.task.notes.trimEnd()}\n${reasonText(t, r)}` : reasonText(t, r);
          await run((a) => a.updateTask(t.task!.id!, { notes }));
        }
      }
      let list = null;
      for (const t of targets) list = await api.setWaiting(t.key, days);
      if (list) waitingStore.set(list);
      notify(`Przypomnę za ${dayLabel(days)}.`);
      onDone();
    } catch (e) {
      notify(describeError(e));
      setBusy(false);
    }
  }

  return (
    <form
      className="wait-step"
      aria-label="Na co czeka"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onDone();
        }
      }}
    >
      {title && <p className="wait-step-title">{title}</p>}
      <label className="wait-step-label">
        {blocked ? "Co blokuje?" : "Na co czeka?"}
        <input
          ref={ref}
          className="field"
          value={reason}
          placeholder={blocked ? "np. brak odpisu z KRS (opcjonalnie)" : "np. weryfikacja konta w Przelewy24 (opcjonalnie)"}
          onChange={(e) => setReason(e.target.value)}
        />
      </label>
      <div className="wait-step-row">
        <span className="wait-step-label">Przypomnij za</span>
        <Segmented<string>
          label="Przypomnij za"
          value={String(days)}
          onChange={(v) => setDays(Number(v))}
          options={DAYS.map((d) => ({ value: String(d), label: dayLabel(d) }))}
        />
      </div>
      <div className="wait-step-actions">
        <Button type="submit" variant="ghost" disabled={busy}>
          {busy ? "Zapisuję…" : "Zapisz"}
        </Button>
        <Button variant="quiet" disabled={busy} onClick={onDone} title={`Przypomnienie i tak za ${dayLabel(defaultDays)}`}>
          Pomiń
        </Button>
      </div>
    </form>
  );
}
