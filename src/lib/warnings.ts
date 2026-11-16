import { t as tr } from "../i18n";
export interface WarningExplanation {
  summary: string;
  raw: string;
}

/** Translates core English parser/I/O warnings into concise Polish impact descriptions. */
export function explainWarning(message: string): WarningExplanation {
  const m = message.trim();

  const invalidDue = m.match(/invalid due date\s+`?([^`;]+)`?/i);
  if (invalidDue) {
    return {
      summary: tr("Nieprawidłowa data terminu ({0}) — pozostała w tekście zadania i nie liczy się jako termin.", invalidDue[1]),
      raw: m,
    };
  }

  const invalidDone = m.match(/invalid completion date\s+`?([^`;]+)`?/i);
  if (invalidDone) {
    return {
      summary: tr("Nieprawidłowa data ukończenia ({0}) — pozostała w tekście zadania.", invalidDone[1]),
      raw: m,
    };
  }

  const badBox = m.match(/checkbox\s+`?(\[[^\]]*\])`?\s+is not a task status/i);
  if (badBox) {
    return {
      summary: tr("Nieznany znacznik pola wyboru {0} — wiersz nie został wczytany jako zadanie.", badBox[1]),
      raw: m,
    };
  }

  if (/malformed task line ignored/i.test(m)) {
    return {
      summary: tr("Zignorowano uszkodzony wiersz zadania — sprawdź składnię w pliku."),
      raw: m,
    };
  }

  const unkState = m.match(/unknown project state\s+`?([^`;]+)`?/i);
  if (unkState) {
    return {
      summary: tr("Nieznany stan projektu ({0}) — potraktowano tymczasowo jako aktywny.", unkState[1]),
      raw: m,
    };
  }

  const unkStatus = m.match(/unknown topic status\s+`?([^`;]+)`?/i);
  if (unkStatus) {
    return {
      summary: tr("Nieznany status tematu ({0}) — potraktowano tymczasowo jako planowany.", unkStatus[1]),
      raw: m,
    };
  }

  if (/^invalid\s+.*(date|timestamp)/i.test(m)) {
    return {
      summary: tr("Nieprawidłowa data lub znacznik czasu w metadanych pliku — wartość została pominięta."),
      raw: m,
    };
  }

  if (/^(frontmatter|expected `key: value`|duplicate `|repository entry without)/i.test(m)) {
    return {
      summary: tr("Problem w nagłówku pliku (frontmatter) — część metadanych została pominięta."),
      raw: m,
    };
  }

  if (/^cannot (access|read)/i.test(m)) {
    return {
      summary: tr("Nie udało się odczytać pliku lub katalogu — element został pominięty."),
      raw: m,
    };
  }

  return {
    summary: tr("Nierozpoznany zapis w pliku — linia lub pole wymaga ręcznego sprawdzenia."),
    raw: m,
  };
}
