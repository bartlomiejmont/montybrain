import { t as tr } from "../i18n";
import { topicSummary } from "../lib/topicSummary";
import { Markdown } from "../views/actions/Markdown";
import "./topic-summary.css";

export function TopicSummary({ text, compact = false }: { text: string; compact?: boolean }) {
  const { lead, details } = topicSummary(text);
  if (!lead) return <p className="summary-empty">{tr("Brak opisu. Dodaj krótko, gdzie jesteśmy i co dalej.")}</p>;
  return <div className={`topic-reading${compact ? " compact" : ""}`}>
    <div className="summary-lead"><span className="summary-label">{tr("W skrócie")}</span><Markdown text={lead} /></div>
    {!compact && details && <details className="summary-evidence"><summary>{tr("Pełny opis i szczegóły techniczne")}</summary><Markdown text={details} /></details>}
  </div>;
}
