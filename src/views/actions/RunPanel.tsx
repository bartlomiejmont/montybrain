// The panel of one run: header, live log, final answer / error.

import { useEffect, useLayoutEffect, useRef, useState } from "react";

import type { Recipe, RunEvent, RunInfo } from "../../api/types";
import { Button } from "../../components/ui";
import { MODE_LABEL, STATUS_LABEL, formatElapsed, formatRunTime, runElapsedMs } from "../../lib/actions";
import { describeError } from "../../lib/errors";
import { agentMeta, stripProposalBlock } from "../../lib/updates";
import { links, navigate } from "../../router/router";
import { runStore } from "../../store/runs";
import { useVault } from "../../store/vault";
import { Markdown } from "./Markdown";

export const PROVIDER_NAME = { claude: "Claude Code", codex: "Codex" } as const;

/** Re-renders every second while `on`. */
function useNow(on: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!on) return;
    setNow(Date.now());
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [on]);
  return now;
}

const clock = (at: string) => (/T(\d{2}:\d{2}:\d{2})/.exec(at)?.[1] ?? at.slice(11, 19));

function Log({ events, running }: { events: RunEvent[]; running: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  // Follow the end only while the reader is at the end.
  useLayoutEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [events.length]);

  return (
    <div
      ref={ref}
      className={`run-log${running ? " live" : ""}`}
      role="log"
      aria-label="Przebieg pracy agenta"
      tabIndex={0}
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
      }}
    >
      {events.length === 0 && <p className="run-log-empty">{running ? "Czekam na pierwsze komunikaty agenta…" : "Brak zapisanego przebiegu."}</p>}
      {events.map((e) => (
        <div key={e.seq} className={`log-line log-${e.kind}`}>
          <time className="log-time">{clock(e.at)}</time>
          <span className="log-text">{e.text}</span>
        </div>
      ))}
    </div>
  );
}

function CopyButton({ text }: { text: string }) {
  const { notify } = useVault();
  const [done, setDone] = useState(false);
  useEffect(() => {
    if (!done) return;
    const t = window.setTimeout(() => setDone(false), 1500);
    return () => window.clearTimeout(t);
  }, [done]);
  return (
    <Button
      variant="quiet"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(text);
          setDone(true);
        } catch {
          notify("Nie udało się skopiować do schowka.");
        }
      }}
    >
      {done ? "Skopiowano" : "Kopiuj"}
    </Button>
  );
}

export function RunPanel({ run, recipe }: { run: RunInfo; recipe: Recipe | null }) {
  const { notify } = useVault();
  const running = run.status === "running";
  const now = useNow(running);
  const [cancelling, setCancelling] = useState(false);
  const terminal = run.mode === "terminal";
  const rootRef = useRef<HTMLElement>(null);
  // A newly shown run may sit below the fold (the form is above it).
  useEffect(() => {
    rootRef.current?.scrollIntoView({ block: "nearest" });
  }, []);
  const elapsed = formatElapsed(runElapsedMs(run.started_at, run.finished_at, now));
  // A proposing recipe's machine-readable block is for the app, not for reading.
  const answer = run.result && recipe?.proposes ? stripProposalBlock(run.result) : run.result;

  async function cancel() {
    setCancelling(true);
    try {
      await runStore.cancel(run.id);
    } catch (e) {
      notify(describeError(e));
    } finally {
      setCancelling(false);
    }
  }

  return (
    <section ref={rootRef} className="run-panel" aria-label="Uruchomienie">
      <header className="run-head">
        <div className="run-head-main">
          <h2 className="run-title">{run.recipe_title}</h2>
          <div className="run-meta">
            <span>{PROVIDER_NAME[run.provider]}</span>
            {agentMeta(run) && (
              <>
                <span aria-hidden>·</span>
                <span title="Model i wysiłek z Ustawień">{agentMeta(run)}</span>
              </>
            )}
            <span aria-hidden>·</span>
            <span>{MODE_LABEL[run.mode]}</span>
            <span aria-hidden>·</span>
            <span className={`run-status st-${run.status}`}>
              {terminal ? "Otwarto w Terminalu" : STATUS_LABEL[run.status]}
              {running && <> · {elapsed}</>}
            </span>
          </div>
          <div className="run-meta run-times">
            <span>Start {formatRunTime(run.started_at)}</span>
            {run.finished_at && !terminal && (
              <>
                <span aria-hidden>·</span>
                <span>
                  Koniec {formatRunTime(run.finished_at)} (czas {elapsed})
                </span>
              </>
            )}
          </div>
        </div>
        {running && (
          <Button variant="danger" onClick={cancel} disabled={cancelling}>
            {cancelling ? "Przerywam…" : "Przerwij"}
          </Button>
        )}
      </header>

      {terminal ? (
        <div className="run-note">
          <p className="run-note-title">Otwarto w Terminalu</p>
          <p className="run-note-text">Sesja toczy się tam dalej. Jej efekty pojawią się w MontyBrain same, gdy agent zapisze zadania lub wpisy.</p>
        </div>
      ) : (
        <>
          <Log events={run.events} running={running} />
          {run.status === "failed" && (
            <div className="run-error" role="alert">
              <p className="run-error-title">Uruchomienie nie powiodło się</p>
              <p className="run-error-text">{run.error ?? "Agent zakończył pracę błędem, ale nie podał powodu."}</p>
            </div>
          )}
          {run.status === "cancelled" && (
            <p className="run-cancelled">
              {recipe?.background_writes ? "Uruchomienie przerwane. Agent mógł zdążyć coś zapisać w MontyBrain." : "Uruchomienie przerwane. Nic nie zostało zapisane w MontyBrain."}
            </p>
          )}
          {run.status === "succeeded" && (
            <div className="run-answer">
              <div className="run-answer-head">
                <span className="run-answer-title">Odpowiedź agenta</span>
                {answer && <CopyButton text={answer} />}
              </div>
              {answer ? <Markdown text={answer} /> : <p className="run-log-empty">Agent nie zwrócił odpowiedzi.</p>}
              {recipe?.proposes && run.project && (
                <p className="run-next">
                  <button type="button" className="btn btn-ghost" onClick={() => navigate(links.project(run.project!))}>
                    Przejdź do propozycji w projekcie
                  </button>
                </p>
              )}
            </div>
          )}
        </>
      )}
    </section>
  );
}
