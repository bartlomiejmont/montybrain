import { t as tr } from "../i18n";
// Skrzynka's field in agent mode: a free-form request to Claude Code or
// Codex, with the choice of what it can reach ("Tylko MontyBrain" in the
// background, or "Moje narzędzia" in a Terminal session) stated plainly under
// it. The agent never writes: its answer comes back as proposals to tick or a
// command to confirm. The Komenda | Agent switch sits at the end of the row
// in both modes (⌘J flips it; `?` at the start of the command field too).

import { forwardRef, useImperativeHandle, useRef, useState } from "react";

import type { InboxAgentTools, ProviderId, ProviderInfo } from "../api/types";
import { agentReach, MY_TOOLS_CONSENT, PROVIDER_NAME } from "../lib/agentMode";
import "./command.css";

export type FieldMode = "command" | "agent";

const IS_MAC = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
export const MODE_SHORTCUT = IS_MAC ? "⌘J" : "Ctrl+J";

/** The Komenda | Agent switch. */
export function ModeSwitch({ mode, onChange }: { mode: FieldMode; onChange: (m: FieldMode) => void }) {
  return (
    <div className="mode-switch" role="radiogroup" aria-label={tr("Tryb pola ({0} przełącza)", MODE_SHORTCUT)} title={tr("{0} przełącza", MODE_SHORTCUT)}>
      {(["command", "agent"] as const).map((m) => (
        <button
          key={m}
          type="button"
          role="radio"
          aria-checked={mode === m}
          className={`mode-opt${mode === m ? " on" : ""}`}
          // Keep the focus in the field.
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => onChange(m)}
        >
          {m === "command" ? tr("Komenda") : "Agent"}
        </button>
      ))}
    </div>
  );
}

function Segmented<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: { value: T; label: string; title?: string; disabled?: boolean }[]; onChange: (v: T) => void }) {
  return (
    <div className="ag-seg" role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          className={`ag-seg-opt${value === o.value ? " on" : ""}`}
          disabled={o.disabled}
          title={o.title}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export interface AgentFieldHandle {
  focus(): void;
  setText(t: string): void;
}

export const AgentField = forwardRef<
  AgentFieldHandle,
  {
    providers: ProviderInfo[];
    provider: ProviderId | null;
    onProvider: (p: ProviderId) => void;
    tools: InboxAgentTools;
    onTools: (t: InboxAgentTools) => void;
    /** "Moje narzędzia" asked for and not yet agreed to: the consent line. */
    consentPending: boolean;
    onConsent: (ok: boolean) => void;
    /** What goes along: "zaznaczone wpisy (3)", "wpis „…”", "całą Skrzynkę (12 wpisów)". */
    context: string;
    busy: boolean;
    initialText?: string;
    onSubmit: (text: string) => void;
    /** "Moje narzędzia": the same request as a Terminal session instead. */
    onTerminal?: (text: string) => void;
    onMode: (m: FieldMode) => void;
  }
>(function AgentField({ providers, provider, onProvider, tools, onTools, consentPending, onConsent, context, busy, initialText = "", onSubmit, onTerminal, onMode }, ref) {
  const [text, setText] = useState(initialText);
  const inputRef = useRef<HTMLInputElement>(null);
  useImperativeHandle(ref, () => ({
    focus: () => inputRef.current?.focus(),
    setText: (t: string) => {
      setText(t);
      requestAnimationFrame(() => inputRef.current?.focus());
    },
  }));
  const reach = provider ? agentReach(tools, provider, context) : null;
  const can = !!provider && !!text.trim() && !busy && !consentPending;

  function submit() {
    if (!can) return;
    onSubmit(text.trim());
    setText("");
  }

  return (
    <div className="cf cf-inbox cf-agent">
      <div className="cf-row">
        <span className="cf-lead" aria-hidden>
          ?
        </span>
        <input
          ref={inputRef}
          className="cf-input"
          value={text}
          autoFocus
          spellCheck={false}
          autoComplete="off"
          aria-label={tr("Prośba do agenta")}
          placeholder={tools === "mine" ? tr("Poproś agenta, np. sprawdź na Slacku, co z fakturą, i zaproponuj zadania…") : tr("Poproś agenta, np. rozpisz na kroki · dopasuj miejsca · co z tego jest pilne…")}
          disabled={busy}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing) return;
            if (e.key === "Enter") {
              e.preventDefault();
              submit();
            } else if (e.key === "Escape") {
              e.preventDefault();
              e.stopPropagation();
              if (consentPending) onConsent(false);
              else onMode("command");
            }
          }}
        />
        <ModeSwitch mode="agent" onChange={onMode} />
      </div>
      <div className="ag-opts">
        <span className="ag-opts-label">Agent</span>
        <Segmented
          label="Agent"
          value={provider ?? "claude"}
          onChange={onProvider}
          options={(["claude", "codex"] as const).map((id) => {
            const p = providers.find((x) => x.id === id);
            return { value: id, label: id === "claude" ? "Claude" : "Codex", disabled: !p?.available, title: p?.available ? (p.version ?? undefined) : tr("{0} nie jest dostępny{1}", PROVIDER_NAME[id], p?.problem ? `: ${p.problem}` : "") };
          })}
        />
        <span className="ag-opts-label">{tr("Narzędzia")}</span>
        <Segmented
          label={tr("Narzędzia agenta")}
          value={tools}
          onChange={onTools}
          options={[
            { value: "montybrain", label: tr("Tylko MontyBrain"), title: tr("W tle: MontyBrain tylko do odczytu, nic więcej") },
            { value: "mine", label: tr("Moje narzędzia"), title: tr("Slack, poczta, sterowanie komputerem: sesja w panelu aplikacji z twoją konfiguracją") },
          ]}
        />
        {tools === "mine" && onTerminal && (
          <button
            type="button"
            className="cf-link ag-term"
            disabled={!can}
            title={tr("Ta sama prośba jako sesja w oknie Terminala (odpowiedź wraca przez plik albo wklejenie)")}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => {
              if (!can) return;
              onTerminal(text.trim());
              setText("");
            }}
          >
            {tr("w Terminalu")}</button>
        )}
        <button type="button" className="ag-send" disabled={!can} onMouseDown={(e) => e.preventDefault()} onClick={submit}>
          {tools === "mine" ? tr("Uruchom sesję ↵") : tr("Wyślij ↵")}
        </button>
      </div>
      {consentPending ? (
        <div className="ag-consent" role="alertdialog" aria-label={tr("Zgoda na moje narzędzia")}>
          <p>{tr(MY_TOOLS_CONSENT)}</p>
          <div className="ag-consent-actions">
            <button type="button" className="ag-send" onClick={() => onConsent(true)}>
              {tr("Rozumiem, włącz")}</button>
            <button type="button" className="cf-link" onClick={() => onConsent(false)}>
              {tr("Nie teraz")}</button>
          </div>
        </div>
      ) : reach ? (
        <p className="ag-reach" aria-live="polite">
          <span className="ag-can">{reach.can}</span> <span className="ag-cannot">{reach.cannot}</span> <span className="ag-returns">{reach.returns}</span>
        </p>
      ) : (
        <p className="ag-reach">{tr("Żaden agent nie jest dostępny (Claude Code ani Codex).")}</p>
      )}
    </div>
  );
});
