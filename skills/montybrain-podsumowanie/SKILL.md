---
name: montybrain-podsumowanie
description: "Podsumowuj pracę z użyciem MontyBrain (serwer MCP `montybrain`). Dwa zastosowania: (1) na koniec sesji zapisz w MontyBrain, co zostało zrobione w tym wątku: wpis do dziennika tematu, zamknięte zadania, nowe zadania na resztę; (2) przygotuj raport z dnia lub tygodnia na podstawie dziennika i zadań. Używaj przy prośbach „podsumuj pracę”, „zapisz postęp”, „zamknij sesję”, „co dziś zrobiłem”, „raport z tygodnia”, „status projektu”."
---

# Podsumowanie pracy w MontyBrain

MontyBrain trzyma pracę użytkownika w plikach Markdown: Projekt → Temat (status, krótki opis stanu, dziennik) → Zadanie. Korzystasz z niego przez narzędzia serwera MCP `montybrain`: `overview`, `list_tasks`, `search`, `get_topic`, `connections`, `create_task`, `update_task`, `create_topic`, `log_update`, `command`. Jeśli tych narzędzi nie ma w sesji, powiedz to wprost; podsumowanie możesz wtedy podać tylko w rozmowie.

Ustal najpierw, o co chodzi: **zapis postępu z tej sesji** czy **raport** za okres. Gdy nie wynika to z prośby, a w wątku była praca, zrób zapis postępu.

## Zapis postępu z sesji

1. **Zbierz fakty.** Przejrzyj cały wątek: co zmieniono, co uruchomiono i z jakim wynikiem, co zostało otwarte. Jeśli pracowałeś w repozytorium, sprawdź `git status` i `git log` z tej sesji, żeby opierać się na stanie faktycznym. Podawaj tylko to, co się wydarzyło: nieuruchomionych testów nie opisuj jako zielonych.
2. **Znajdź miejsce.** `overview`, potem `get_topic` dla tematu, którego dotyczyła praca. Gdy nie ma pasującego tematu, zaproponuj nowy (`create_topic`) albo zapytaj, do którego dopisać. Nie zgaduj projektu.
3. **Zadania.** Zadania tematu wykonane w sesji oznacz `update_task` ze `status: "done"`; rozpoczęte i niedokończone ustaw na `doing`; zablokowane na `blocked` z notatką, na co czekają. Do zadań dołącz linki do MR-ów i wątku (`add_links`).
4. **Reszta pracy.** To, co zostało do zrobienia, zapisz jako nowe zadania (`create_task`), po sprawdzeniu przez `search`, czy już nie istnieją. Gdy nowe zadanie czeka na coś, podaj to w `waits_for` (`^id` albo `[[projekt/temat]]`).
5. **Dziennik.** Jeden wpis `log_update` w temacie:
   - `text`: 1–3 zdania po polsku: co zrobiono, z jakim wynikiem, co dalej. Konkret zamiast ogólników („Migracja tabeli cen na stagingu przeszła, 2 testy integracyjne nadal czerwone”, nie „postęp w migracji”).
   - `status`: zmień tylko, gdy stan tematu naprawdę się zmienił (np. `waiting`, gdy czeka na review; `done`, gdy zamknięty).
   - `summary`: zaktualizuj opis „gdzie jesteśmy”, jeśli poprzedni przestał być prawdziwy. Zwykły tekst, bez nagłówków `#`.
   - `author`: nazwa agenta, np. `claude-code` albo `codex`.
6. **Potwierdź.** Wypisz użytkownikowi, co zapisałeś: wpis, zmienione zadania z `^id`, nowe zadania.

Jeden wpis na sesję i temat. Nie dopisuj kolejnego, jeśli od poprzedniego nic się nie zmieniło.

## Raport z dnia lub tygodnia

1. Zbierz dane ze wszystkich miejsc, nie tylko z tematów w toku:
   - `overview` dla listy projektów i tematów (tematy zakończone pokazuje tylko jako liczbę);
   - `list_tasks` z `include_done: true`, osobno dla każdego projektu (`project`) oraz dla Skrzynki (`inbox_only: true`): to obejmuje zadania w tematach, zadania na poziomie projektu i zamknięte tematy. Gdy wynik jest stronicowany („showing a–b of N”), pobierz kolejne strony przez `offset`;
   - `get_topic` z `include_done: true` dla każdego tematu, w którym w okresie coś się działo, także zakończonego: stamtąd bierzesz dziennik.
2. Wybierz wpisy dziennika i zadania zrobione w zadanym okresie (data wpisu; data `✓` przy zadaniu). Gdy użytkownik nie podał okresu: „dziś” dla podsumowania dnia, ostatnie 7 dni dla tygodnia.
3. Ułóż raport według projektów i tematów:
   - **Zrobione**: zamknięte zadania i najważniejsze wpisy dziennika.
   - **W toku**: co trwa, z aktualnym stanem tematu.
   - **Zablokowane / czeka**: na co.
   - **Najbliższe**: zadania z terminem w najbliższych dniach i zaległe.
4. Raport oddaj w rozmowie. Do MontyBrain nic nie zapisuj, chyba że użytkownik o to poprosi.

Pisz zwięźle i po polsku. Opieraj się wyłącznie na tym, co jest w MontyBrain i w repozytoriach; jeśli dziennik jest pusty, napisz, że brak wpisów, zamiast odtwarzać pracę z domysłów.
