---
name: morwisko-postep
description: "Sprawdź postęp otwartych zadań z Morwiska (serwer MCP `morwisko`) w repozytoriach projektu i zdaj raport: co wygląda na zrobione, co jest w toku, co stoi. Używaj przy prośbach „co mam otwarte”, „sprawdź postęp”, „jak stoją zadania”, „przejrzyj otwarte zadania w projekcie…”, „co z tego jest już zrobione w kodzie”. Domyślnie tylko czyta: niczego nie zmienia w Morwisku ani w repozytoriach."
---

# Postęp otwartych zadań

Zestawiasz dwie rzeczy: otwarte zadania w Morwisku i faktyczny stan repozytoriów projektu. Z Morwiska korzystasz przez narzędzia serwera MCP `morwisko` (`overview`, `list_tasks`, `get_topic`, `search`). Jeśli tych narzędzi nie ma w sesji, powiedz to wprost i zakończ.

## Zasady

- **Tylko odczyt.** Nie zmieniaj zadań, tematów ani dziennika i nie modyfikuj repozytoriów (żadnych commitów, checkoutów, pull, stash, instalacji). Wyjątek: użytkownik w tej samej prośbie wyraźnie kazał zaktualizować Morwisko; wtedy na końcu, po pokazaniu listy zmian i potwierdzeniu, zapisz ustalenia raportu: nowe tematy (`create_topic`, potem `log_update` z dowodami), wpisy, statusy i opisy istniejących tematów (`log_update`), statusy zadań (`update_task`) i brakujące zadania (`create_task`).
- **Dowody zamiast domysłów.** „Zrobione” piszesz tylko wtedy, gdy widzisz to w repozytorium (commit, zmieniony plik, zmergowana gałąź). Gdy nie da się tego stwierdzić, napisz „nie da się potwierdzić z repozytorium”.
- Nie uruchamiaj testów, buildów ani skryptów projektu, chyba że użytkownik o to poprosi.

## Kroki

1. **Zakres.** Projekt z prośby; gdy go nie podano, wszystkie aktywne projekty. `overview` (z `project`, jeśli jest) daje listę tematów i repozytoriów.
2. **Otwarte zadania.** `list_tasks` dla projektu (statusy do zrobienia, w toku, zablokowane; bez zrobionych). Przy stronicowaniu pobierz kolejne strony przez `offset`. Dla tematów w toku weź `get_topic`, żeby znać opis stanu i ostatnie wpisy dziennika.
3. **Stan repozytoriów.** Ścieżki repozytoriów są w `overview` projektu. Dla repozytorium, którego dotyczy temat lub zadanie, sprawdź poleceniami tylko do odczytu:
   - `git -C <ścieżka> status --short --branch`: bieżąca gałąź, niezacommitowane zmiany;
   - `git -C <ścieżka> log --oneline -15` oraz `git -C <ścieżka> log --since="14 days ago" --oneline --all`: ostatnia praca;
   - `git -C <ścieżka> branch --sort=-committerdate`: gałęzie, w tym pasujące nazwą do zadania;
   - `git -C <ścieżka> worktree list`: drzewa robocze; gałąź pobrana w osobnym drzewie roboczym najpewniej jest teraz w pracy;
   - w razie potrzeby wyszukaj w kodzie nazwy z tytułu zadania (pliki, funkcje, identyfikatory) i zajrzyj do wskazanych plików.
   Jeśli zadanie ma link do MR-a lub issue, podaj go w raporcie; jego stanu nie zgaduj, jeśli nie masz narzędzia, żeby go sprawdzić.
4. **Dopasuj.** Dla każdego otwartego zadania oceń: **wygląda na zrobione** (jest dowód w repozytorium), **w toku** (jest gałąź, commity lub niezacommitowane zmiany, ale brak domknięcia), **bez śladu pracy**, **zablokowane** (z notatek lub statusu), **nie dotyczy kodu** (nie da się ocenić z repozytorium).
5. **Raport** po polsku, zwięźle, według projektów i tematów:
   - przy każdym zadaniu: tytuł, `^id`, ocena i jedno zdanie z dowodem (commit, gałąź, plik);
   - na końcu: zadania po terminie i na najbliższe dni oraz krótka lista „co proponuję zrobić w Morwisku”: najpierw tematy (strumienie pracy widoczne w repozytoriach, których nie ma jeszcze w Morwisku, i tematy do zaktualizowania), potem zadania do zamknięcia lub dodania. To propozycje: niczego nie zapisujesz bez prośby.

Gdy repozytorium nie istnieje pod podaną ścieżką albo nie masz do niego dostępu, napisz to przy danym projekcie i oprzyj tę część raportu wyłącznie na Morwisku.
