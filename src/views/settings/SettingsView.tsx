// „Ustawienia": global shortcut, reminders, autostart, vault folder, version.
// Settings live outside the vault (src-tauri/src/settings.rs); every change goes
// through the typed API and comes back as the new `AppSettings`.

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import { AGENT_EFFORTS, api, type AgentChoice, type Agents as AgentsT, type AppSettings, type ProviderId } from "../../api";
import { DEFAULT_SHORTCUT, canonicalShortcut, shortcutFromEvent, shortcutKeys } from "../../api/shortcut";
import { createAgentSaver, modelToSave, type AgentSaver } from "../../lib/agentSave";
import { describeError } from "../../lib/errors";
import { PROVIDER_LABEL, autoLabel, formatNext } from "../../lib/updates";
import { links } from "../../router/router";
import { updateStore, useUpdates } from "../../store/updates";
import { useView } from "../../store/vault";

const HOURS = Array.from({ length: 24 }, (_, h) => h);

function isValidShortcut(s: string): boolean {
  try {
    canonicalShortcut(s);
    return true;
  } catch {
    return false;
  }
}

function Row({ label, hint, children, id }: { label: string; hint?: ReactNode; children?: ReactNode; id?: string }) {
  return (
    <div className="set-row">
      <div className="set-text">
        <div className="set-label" id={id}>
          {label}
        </div>
        {hint && <div className="set-hint">{hint}</div>}
      </div>
      <div className="set-control">{children}</div>
    </div>
  );
}

function Switch({ on, onChange, labelledBy, disabled }: { on: boolean; onChange: (v: boolean) => void; labelledBy: string; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-labelledby={labelledBy}
      disabled={disabled}
      className={`set-switch${on ? " on" : ""}`}
      onClick={() => onChange(!on)}
    >
      <span className="set-knob" />
    </button>
  );
}

function Keys({ shortcut }: { shortcut: string }) {
  return (
    <span className="set-keys" aria-label={shortcut}>
      {shortcutKeys(shortcut).map((k, i) => (
        <kbd key={i} className="set-kbd">
          {k}
        </kbd>
      ))}
    </span>
  );
}

function ShortcutRow({ s, apply }: { s: AppSettings; apply: (call: () => Promise<AppSettings>) => Promise<string | null> }) {
  const [recording, setRecording] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const boxRef = useRef<HTMLButtonElement>(null);

  const stop = useCallback(
    async (next: string | null) => {
      setRecording(false);
      if (next) {
        const err = await apply(() => api.setShortcut(next));
        setError(err);
      } else {
        // Resuming can fail (another app took the combination meanwhile): the
        // shortcut is then inactive, and the user must be told why.
        setError(await apply(() => api.pauseShortcut(false)));
      }
    },
    [apply],
  );

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey) {
        void stop(null);
        return;
      }
      const sc = shortcutFromEvent(e);
      if (sc) void stop(sc);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording, stop]);

  async function start() {
    setError(null);
    await api.pauseShortcut(true).catch(() => {});
    setRecording(true);
    boxRef.current?.focus();
  }

  const problem = error ?? s.shortcut_error;
  return (
    <>
      <Row
        id="set-shortcut"
        label="Szybkie zadanie"
        hint="Skrót działa w każdej aplikacji i otwiera małe okno do zapisania zadania."
      >
        <button
          ref={boxRef}
          type="button"
          className={`set-shortcut${recording ? " recording" : ""}${s.shortcut_error && !recording ? " broken" : ""}`}
          aria-labelledby="set-shortcut"
          aria-live="polite"
          onClick={() => (recording ? void stop(null) : void start())}
          onBlur={() => recording && void stop(null)}
        >
          {recording ? <span className="set-recording">Naciśnij skrót…</span> : <Keys shortcut={s.shortcut} />}
        </button>
        {!recording && s.shortcut !== DEFAULT_SHORTCUT && (
          <button type="button" className="btn btn-quiet" onClick={() => void stop(DEFAULT_SHORTCUT)}>
            Domyślny
          </button>
        )}
      </Row>
      {(problem || recording) && (
        <div className={`set-note${problem && !recording ? " danger" : ""}`} role={problem ? "alert" : undefined}>
          {recording ? (
            "Użyj ⌃, ⌥ lub ⌘ z dowolnym klawiszem. Esc anuluje."
          ) : (
            <>
              {problem}{" "}
              {s.shortcut_error && isValidShortcut(s.shortcut) && (
                <button type="button" className="set-link" onClick={() => void apply(() => api.setShortcut(s.shortcut)).then(setError)}>
                  Spróbuj ponownie
                </button>
              )}
            </>
          )}
        </div>
      )}
    </>
  );
}

/** Projects with automatic update or the history source on (configured in the project view). */
function AutoUpdates({ s, onError }: { s: AppSettings; onError: (msg: string | null) => void }) {
  const view = useView();
  const updates = useUpdates();
  useEffect(() => {
    void updateStore.start().catch(() => {});
  }, []);
  const autoOn = (slug: string) => (s.auto_update?.[slug]?.schedule.kind ?? "off") !== "off";
  const historyOn = (slug: string) => s.update_sources?.[slug]?.history ?? false;
  const slugs = [...new Set([...Object.keys(s.auto_update ?? {}), ...Object.keys(s.update_sources ?? {})])].filter((x) => autoOn(x) || historyOn(x)).sort();
  const name = (slug: string) => view.snapshot.projects.find((p) => p.slug === slug)?.name ?? slug;
  const done = (call: Promise<import("../../api").ProjectUpdateView[]>) =>
    void call.then(
      (list) => {
        updateStore.set(list);
        onError(null);
      },
      (e) => onError(describeError(e)),
    );
  return (
    <section className="group" aria-labelledby="set-g-auto">
      <h2 className="group-head" id="set-g-auto">
        Aktualizacja projektów
      </h2>
      {slugs.length === 0 ? (
        <p className="set-empty">
          Automatyczna aktualizacja i historia rozmów są wyłączone we wszystkich projektach. Włączysz je w widoku projektu, w pasku
          „Aktualizacja”. Każde sprawdzenie zużywa limit wybranej subskrypcji.
        </p>
      ) : (
        slugs.map((slug) => {
          const cfg = s.auto_update?.[slug];
          const src = s.update_sources?.[slug];
          const u = updates.byProject[slug];
          const when = u?.held ? "wstrzymane: czekają propozycje" : u?.next_due ? `następne ${formatNext(u.next_due, new Date())}` : "";
          const parts: string[] = [];
          if (cfg && autoOn(slug)) {
            parts.push(`automatycznie ${autoLabel(cfg)}`, PROVIDER_LABEL[cfg.provider], cfg.on_result === "apply" ? "zapisuje bez pytania" : "pokazuje propozycje");
            if (when) parts.push(when);
          } else parts.push("automatycznie: wyłączona");
          if (src?.history) parts.push(`historia rozmów: ${src.history_days} dni`);
          return (
            <Row key={slug} label={name(slug)} hint={parts.join(" · ")}>
              {cfg && autoOn(slug) && (
                <button
                  type="button"
                  className="btn btn-quiet"
                  aria-label={`Wyłącz automatyczną aktualizację projektu ${name(slug)}`}
                  onClick={() => done(api.setAutoUpdate(slug, { ...cfg, schedule: { kind: "off" } }))}
                >
                  Wyłącz
                </button>
              )}
              {src?.history && (
                <button
                  type="button"
                  className="btn btn-quiet"
                  aria-label={`Wyłącz historię rozmów w projekcie ${name(slug)}`}
                  onClick={() => done(api.setUpdateSources(slug, { ...src, history: false }))}
                >
                  Wyłącz historię
                </button>
              )}
              <a className="btn btn-ghost" href={`${links.project(slug)}?auto=1`} aria-label={`Zmień aktualizację projektu ${name(slug)}`}>
                Zmień
              </a>
            </Row>
          );
        })
      )}
      <p className="set-hint set-foot">Działa tylko wtedy, gdy Morwisko jest uruchomione.</p>
    </section>
  );
}

const AGENT_HINT = {
  claude: "Puste pole: model z ustawień Claude Code.",
  codex: "Puste pole: domyślny model Codexa. Morwisko nie wczytuje twojej konfiguracji Codexa.",
} as const;

/** Model and effort per agent; empty = the CLI's default. */
function Agents({ s, apply }: { s: AppSettings; apply: (call: () => Promise<AppSettings>) => Promise<string | null> }) {
  const [drafts, setDrafts] = useState<Record<ProviderId, string>>({ claude: s.agents.claude.model ?? "", codex: s.agents.codex.model ?? "" });
  const [errors, setErrors] = useState<Partial<Record<ProviderId, string>>>({});
  // A quiet "Zapisano" by the row for a moment after a save.
  const [saved, setSaved] = useState<Partial<Record<ProviderId, number>>>({});
  useEffect(() => {
    const ids = Object.values(saved);
    if (ids.length === 0) return;
    const t = window.setTimeout(() => setSaved({}), 2000);
    return () => window.clearTimeout(t);
  }, [saved]);
  useEffect(() => {
    setDrafts({ claude: s.agents.claude.model ?? "", codex: s.agents.codex.model ?? "" });
  }, [s.agents.claude.model, s.agents.codex.model]);
  // One save at a time, each merged into the latest saved settings (a model
  // and an effort changed right after each other both stick).
  const saver = useRef<AgentSaver | null>(null);
  saver.current ??= createAgentSaver(async (next: AgentsT) => {
    let out: AppSettings | null = null;
    const err = await apply(async () => (out = await api.updateSettings({ agents: next })));
    if (err || !out) throw new Error(err ?? "Nie udało się zapisać.");
    return (out as AppSettings).agents;
  }, s.agents);
  useEffect(() => saver.current?.sync(s.agents), [s.agents]);
  const flash = (p: ProviderId) => setSaved((x) => ({ ...x, [p]: Date.now() }));
  const save = async (p: ProviderId, patch: Partial<AgentChoice>) => {
    try {
      await saver.current!.save(p, patch);
      setErrors((e) => ({ ...e, [p]: undefined }));
      flash(p);
    } catch (e) {
      setErrors((x) => ({ ...x, [p]: e instanceof Error ? e.message : String(e) }));
    }
  };
  /** The model field left (blur) or confirmed (Enter, which also says "Zapisano" when nothing changed). */
  const commitModel = (p: ProviderId, confirm: boolean) => {
    const v = modelToSave(drafts[p], s.agents[p].model);
    if (v !== undefined) void save(p, { model: v });
    else if (confirm) flash(p);
  };
  return (
    <section className="group" aria-labelledby="set-g-agents">
      <h2 className="group-head" id="set-g-agents">
        Agenci
      </h2>
      {(["claude", "codex"] as const).map((p) => {
        const choice = s.agents[p];
        const label = p === "claude" ? "Claude Code" : "Codex";
        return (
          <div key={p}>
            <Row id={`set-agent-${p}`} label={label} hint={AGENT_HINT[p]}>
              <input
                className="field set-model"
                aria-label={`Model ${label}`}
                placeholder={s.agent_suggestions[p][0] ? `domyślny, np. ${s.agent_suggestions[p][0]}` : "domyślny"}
                list={`set-models-${p}`}
                value={drafts[p]}
                spellCheck={false}
                autoCapitalize="off"
                onChange={(e) => setDrafts((d) => ({ ...d, [p]: e.target.value }))}
                onBlur={() => commitModel(p, false)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    commitModel(p, true);
                  }
                }}
              />
              <datalist id={`set-models-${p}`}>
                {s.agent_suggestions[p].map((m) => (
                  <option key={m} value={m} />
                ))}
              </datalist>
              <select
                className="field select set-select"
                aria-label={`Wysiłek ${label}`}
                value={choice.effort ?? ""}
                onChange={(e) => {
                  // A model typed but not yet saved (no blur happened) goes with it.
                  const model = modelToSave(drafts[p], s.agents[p].model);
                  void save(p, { effort: e.target.value || null, ...(model !== undefined ? { model } : {}) });
                }}
              >
                <option value="">domyślny</option>
                {AGENT_EFFORTS[p].map((x) => (
                  <option key={x} value={x}>
                    {x}
                  </option>
                ))}
              </select>
              <span className="set-saved" role="status">
                {saved[p] ? "Zapisano" : ""}
              </span>
            </Row>
            {errors[p] && (
              <div className="set-note danger" role="alert">
                {errors[p]}
              </div>
            )}
          </div>
        );
      })}
      <p className="set-hint set-foot">Większy wysiłek może wydłużyć wykonanie i zwiększyć zużycie limitu.</p>
    </section>
  );
}

export function SettingsView() {
  const [s, setS] = useState<AppSettings | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [rowError, setRowError] = useState<string | null>(null);

  useEffect(() => {
    let off: (() => void) | undefined;
    let disposed = false;
    api.getSettings().then(setS, (e) => setLoadError(describeError(e)));
    void api.onSettingsChange(setS).then((u) => (disposed ? u() : (off = u)));
    return () => {
      disposed = true;
      off?.();
    };
  }, []);

  /** Runs a settings call; returns the error text (or null) and keeps the view in sync. */
  const apply = useCallback(async (call: () => Promise<AppSettings>): Promise<string | null> => {
    try {
      setS(await call());
      setRowError(null);
      return null;
    } catch (e) {
      const msg = describeError(e);
      api.getSettings().then(setS, () => {});
      return msg;
    }
  }, []);

  const applyRow = (call: () => Promise<AppSettings>) => void apply(call).then(setRowError);

  return (
    <div className="view settings">
      <header className="view-head">
        <h1 className="view-title">Ustawienia</h1>
        <p className="view-sub">Działanie aplikacji na tym komputerze. Dane zostają w Twoim folderze.</p>
      </header>

      {loadError && <p className="set-note danger">{loadError}</p>}
      {s && (
        <>
          <section className="group" aria-labelledby="set-g-quick">
            <h2 className="group-head" id="set-g-quick">
              Skrót
            </h2>
            <ShortcutRow s={s} apply={apply} />
          </section>

          <section className="group" aria-labelledby="set-g-rem">
            <h2 className="group-head" id="set-g-rem">
              Przypomnienia
            </h2>
            <Row id="set-reminders" label="Powiadomienia o terminach" hint="Zadanie z godziną przypomni się o tej godzinie.">
              <Switch on={s.reminders} labelledBy="set-reminders" onChange={(v) => applyRow(() => api.updateSettings({ reminders: v }))} />
            </Row>
            <Row
              id="set-digest"
              label="Podsumowanie dnia"
              hint="Jedno powiadomienie z zadaniami na dziś bez godziny i zaległymi."
            >
              <select
                className="field select set-select"
                aria-labelledby="set-digest"
                value={s.digest_hour}
                disabled={!s.reminders}
                onChange={(e) => applyRow(() => api.updateSettings({ digest_hour: Number(e.target.value) }))}
              >
                {HOURS.map((h) => (
                  <option key={h} value={h}>
                    {String(h).padStart(2, "0")}:00
                  </option>
                ))}
              </select>
            </Row>
            <Row
              id="set-waiting"
              label="Przypomnienia o tym, co czeka"
              hint="Tematy Czeka i Zablokowany oraz zablokowane zadania wracają do „Wymaga uwagi” w Dziś po tylu dniach; jedno powiadomienie o godzinie podsumowania."
            >
              <select
                className="field select set-select"
                aria-labelledby="set-waiting"
                value={s.waiting_days}
                onChange={(e) => applyRow(() => api.updateSettings({ waiting_days: Number(e.target.value) }))}
              >
                {[1, 2, 3, 7].map((d) => (
                  <option key={d} value={d}>
                    co {d === 1 ? "1 dzień" : `${d} dni`}
                  </option>
                ))}
              </select>
            </Row>
          </section>

          <Agents s={s} apply={apply} />

          <AutoUpdates s={s} onError={setRowError} />

          <section className="group" aria-labelledby="set-g-sys">
            <h2 className="group-head" id="set-g-sys">
              System
            </h2>
            <Row id="set-autostart" label="Uruchamiaj przy logowaniu" hint="Morwisko startuje w tle, ikoną w pasku menu.">
              <Switch on={s.autostart} labelledBy="set-autostart" onChange={(v) => applyRow(() => api.setAutostart(v))} />
            </Row>
            {s.autostart_error && <div className="set-note danger">{s.autostart_error}</div>}
            <Row id="set-vault" label="Folder danych" hint={<span className="set-path">{s.vault_path}</span>}>
              <button type="button" className="btn btn-ghost" onClick={() => void api.revealVault().catch((e) => setRowError(describeError(e)))}>
                Pokaż w Finderze
              </button>
            </Row>
            <Row label="Wersja">
              <span className="set-value">{s.version}</span>
            </Row>
          </section>
          {rowError && (
            <p className="set-note danger" role="alert">
              {rowError}
            </p>
          )}
        </>
      )}
    </div>
  );
}
