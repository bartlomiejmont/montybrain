// Core errors arrive as English display strings; give them a Polish lead-in.

const PREFIXES: [RegExp, string][] = [
  [/^invalid input:\s*/i, "Nieprawidłowe dane: "],
  [/^project not found:\s*/i, "Nie ma takiego projektu: "],
  [/^topic not found:\s*/i, "Nie ma takiego tematu: "],
  [/^task not found:\s*/i, "Nie ma takiego zadania: "],
  [/^vault directory does not exist:\s*/i, "Katalog danych nie istnieje: "],
  [/^I\/O error at\s*/i, "Błąd zapisu/odczytu: "],
  [/^cannot edit\s*/i, "Nie można edytować pliku: "],
];

export function describeError(err: unknown): string {
  const raw =
    typeof err === "string"
      ? err
      : err instanceof Error
        ? err.message
        : (() => {
            try {
              return JSON.stringify(err);
            } catch {
              return String(err);
            }
          })();
  for (const [re, pl] of PREFIXES) {
    if (re.test(raw)) return raw.replace(re, pl);
  }
  return raw || "Nieznany błąd";
}
