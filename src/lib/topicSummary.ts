import { firstSentence } from "./today";

/** Surface the author's own conclusion; never invent or rewrite project facts. */
export function topicSummary(text: string): { lead: string; details: string } {
  const clean = text.trim();
  const conclusion = /(?:^|\s)(?:\*\*)?Wniosek:(?:\*\*)?\s*/i.exec(clean);
  if (conclusion) {
    const lead = clean.slice(conclusion.index + conclusion[0].length).trim();
    if (lead) return { lead, details: clean.slice(0, conclusion.index).trim() };
  }
  if (clean.length <= 260) return { lead: clean, details: "" };
  return { lead: firstSentence(clean), details: clean };
}
