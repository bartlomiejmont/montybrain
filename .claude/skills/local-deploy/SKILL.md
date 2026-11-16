---
name: local-deploy
description: Zbuduj MontyBrain z bieżącego katalogu i podmień zainstalowaną aplikację w /Applications na najnowszą wersję. Używaj przy prośbach „local deploy”, „wdróż lokalnie”, „podmień aplikację”, „zainstaluj najnowszą wersję”, „zaktualizuj apkę u mnie” oraz gdy użytkownik pyta, czy zmiany są już w zainstalowanej aplikacji.
---

# Local deploy MontyBrain

Zmiany w źródłach nie trafiają same do `/Applications/MontyBrain.app`. Ten skill buduje pakiet i podmienia aplikację.

## Kroki

1. Uruchom z katalogu repozytorium:

   ```sh
   scripts/local-deploy.sh
   ```

   Skrypt: uruchamia testy, buduje pakiet (`pnpm tauri build --bundles app`), zamyka działającą aplikację, przenosi starą do `target/local-deploy-backups/` (zostają 3 ostatnie), kopiuje nową, porównuje SHA256 i otwiera aplikację.

   - `--no-build`: użyj już zbudowanego pakietu z `target/release/bundle/macos/`.
   - `--no-open`: nie otwieraj aplikacji po podmianie.

   Budowanie trwa do kilku minut; ustaw odpowiednio długi limit czasu.

2. Jeśli skrypt się nie powiedzie, pokaż użytkownikowi końcówkę wyjścia i nie podmieniaj aplikacji ręcznie z pominięciem testów. Nie naprawiaj przy okazji niepowiązanych błędów bez pytania.

3. Zgłoś wynik: czy aplikacja została podmieniona i uruchomiona, oraz gdzie leży poprzednia wersja.

## Uwagi

- Zamknięcie aplikacji przerywa sesję agenta uruchomioną w jej panelu. Jeśli użytkownik nie prosił wprost o podmianę, zapytaj przed uruchomieniem.
- Skrypt nie rusza vaulta ani serwera MCP (`~/.local/bin/montybrain-mcp`). Gdy zmieniał się `crates/` lub serwer MCP, powiedz o tym użytkownikowi i zaproponuj `./install.sh`.
- Cofnięcie: zamknij aplikację i przenieś wybraną kopię z `target/local-deploy-backups/` z powrotem do `/Applications/MontyBrain.app`.
