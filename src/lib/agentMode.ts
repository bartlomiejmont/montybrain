import { t as tr } from "../i18n";
// The Skrzynka field's agent mode, the parts without React: what a request
// asks for, what the agent can and cannot reach with each choice of tools,
// and which entries go along as context. Kept honest: every sentence here is
// what the app actually sets up (src-tauri runs/recipes.rs, runs/command.rs).

import type { InboxAgentTools, ProviderId } from "../api/types";

export interface AgentAskPlan {
  /** `command`: one entry, the answer is a command for the field; `proposals`: changes to tick. */
  mode: "command" | "proposals";
  /** The request as the agent gets it. */
  text: string;
}

/**
 * What to run for `raw` (the agent field's text) about `entries`. `rozpisz …`
 * and `dopasuj …` at the start keep their meaning from the `?` commands. With
 * the user's own tools the answer is always proposals (it comes back through a
 * file, not into the field).
 */
export function planAsk(raw: string, entries: number, tools: InboxAgentTools): AgentAskPlan {
  const text = raw.trim().replace(/^\?\s*/, "");
  const m = /^(rozpisz|dopasuj)\b\s*/i.exec(text);
  const rest = m ? text.slice(m[0].length).trim() : text;
  let ask = text;
  if (m && m[1]!.toLowerCase() === "rozpisz") ask = tr("Rozpisz te wpisy na konkretne kroki. {0}", rest).trim();
  if (m && m[1]!.toLowerCase() === "dopasuj") ask = tr("Zaproponuj miejsca dla tych wpisów. {0}", rest).trim();
  const single = !m && entries === 1 && tools === "montybrain";
  return { mode: single ? "command" : "proposals", text: ask };
}

/** Which entries the agent gets: the selected ones, else the focused one, else every open entry (empty list). */
export function agentEntries(selected: string[], focused: string | null): string[] {
  if (selected.length > 0) return selected;
  return focused ? [focused] : [];
}

/** "zaznaczone wpisy (3)", "wpis „…”", "całą Skrzynkę (12 wpisów)". */
export function contextLabel(selected: number, focusedTitle: string | null, open: number): string {
  if (selected > 0) return selected === 1 ? tr("zaznaczony wpis") : tr("zaznaczone wpisy ({0})", selected);
  if (focusedTitle) return tr("wpis „{0}”", focusedTitle);
  const w = open === 1 ? tr("wpis") : open % 10 >= 2 && open % 10 <= 4 && (open % 100 < 12 || open % 100 > 14) ? tr("wpisy") : tr("wpisów");
  return tr("całą Skrzynkę ({0} {1})", open, w);
}

export const PROVIDER_NAME: Record<ProviderId, string> = { claude: "Claude Code", codex: "Codex" };

/** What the agent can and cannot reach, for the line under the agent field. */
export function agentReach(tools: InboxAgentTools, provider: ProviderId, context: string): { can: string; cannot: string; returns: string } {
  if (tools === "montybrain") {
    return {
      can: tr("W tle, bez okna. Agent widzi {0} oraz MontyBrain tylko do odczytu (tematy, zadania, wyszukiwanie, powiązania).", context),
      cannot: tr("Nie ma internetu, Slacka, poczty, plików ani sterowania komputerem."),
      returns: tr("Niczego nie zapisuje: odpowiada propozycjami do zaznaczenia albo komendą do potwierdzenia."),
    };
  }
  const returns = tr("Rozmowa, narzędzia i prośby o zgodę są w panelu sesji w aplikacji; możesz dopisywać wiadomości. Propozycje wrócą tutaj do zaznaczenia. Zamiast tego możesz otworzyć sesję w Terminalu.");
  if (provider === "claude") {
    return {
      can: tr("Sesja Claude Code w aplikacji, z twoją konfiguracją: twoje serwery MCP, konektory claude.ai i skille (np. Slack albo poczta, jeśli je masz). Dostaje {0}. Narzędzia pytają o zgodę w panelu sesji.", context),
      cannot:
        tr("Claude Code (CLI) nie ma sterowania komputerem (to funkcja aplikacji Claude na Maca), a przeglądarki — bez rozszerzenia Claude w Chrome. MontyBrain tylko do odczytu: zapis przez MontyBrain i edycja jego plików są zablokowane."),
      returns,
    };
  }
  return {
    can: tr("Sesja Codex w aplikacji, z twoją konfiguracją: serwery MCP, wtyczki i skille, a sterowanie komputerem i przeglądarka, jeśli masz je włączone w Codexie (gdyby w aplikacji nie zadziałały, otwórz sesję w Terminalu). Dostaje {0}. Każde narzędzie, które coś zmienia, pyta o zgodę w panelu sesji.", context),
    cannot: tr("MontyBrain tylko do odczytu; polecenia powłoki mogą zapisywać tylko w katalogu sesji, nie w MontyBrain."),
    returns,
  };
}

/** The consent line shown the first time "Moje narzędzia" is turned on. */
export const MY_TOOLS_CONSENT =
  "Agent będzie działał z twoimi uprawnieniami i może czytać to, do czego mają dostęp twoje narzędzia (np. Slack, poczta, ekran). Niczego nie zapisze w MontyBrain: MontyBrain zostaje tylko do odczytu, a każde użycie innego narzędzia zatwierdzasz w panelu sesji (albo w Terminalu). Włącza się na jedną prośbę.";

export const MY_TOOLS_CONSENT_KEY = "montybrain.agent.mojeNarzedzia.zgoda";
