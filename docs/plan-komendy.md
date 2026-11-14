# Plan: komendy w Skrzynce i na serwerze MCP (do wersji 1.0.0)

Stan: decyzje właściciela podjęte 2026-11-14 po propozycjach trzech modeli. Nic nie jest zbudowane.

## Decyzje

1. Miejsce docelowe wskazuje ` > ` (ze spacjami).
2. Każda komenda wymaga potwierdzenia przed zapisem. Zwykłe dopisanie zadania do Skrzynki zapisuje się jak dotąd.
3. Agenci dostają oba wejścia: tekstową komendę i pola w narzędziach strukturalnych.
4. Sama nazwa tematu wystarcza, gdy jest jednoznaczna; przy kolizji aplikacja pyta.
5. Zakres 1.0.0: język, Skrzynka i cztery pomysły poniżej; całość do dopracowania w recenzjach przed wydaniem.

## Język

Jeden parser w `core`, używany w Skrzynce, oknie szybkiego dodawania, palecie ⌘K i w MCP.

```text
Zadzwonić do sądu @jutro !wysoki            zadanie w Skrzynce
Podpiąć karty > whenubuy/checkout           zadanie w temacie
Sprawdzić fakturę > whenubuy                zadanie na poziomie projektu
[/] Dokończyć opis > mapa-mysli             zadanie od razu w toku; sam temat, bo jednoznaczny
/temat Płatności mobilne > whenubuy         nowy temat
/temat ^a1b2c3 > firma                      temat z istniejącego wpisu (wpis zostaje jego zadaniem)
^a1b2c3 > whenubuy/checkout                 przeniesienie
^a1b2c3 [x]                                 status
^a1b2c3 !wysoki @2026-11-20                 priorytet i termin
^a1b2c3 /czeka ^c3d4e5                      dopisuje linię „Czeka na: …”
[[whenubuy/checkout]] /czeka [[firma/krs]]  to samo dla tematu (linia w opisie stanu)
^a1b2c3 /powiaz [[whenubuy/platnosci]]      dopisuje linię „Powiązane: …”
?rozpisz ^a1b2c3                            prośba do agenta
?dopasuj                                    agent proponuje miejsca dla zaznaczonych albo całej Skrzynki
```

Reguły:

- `!priorytet`, `@termin`, `^id`, `[[projekt/temat]]` i znaczniki statusu `[ ] [/] [x] [!]` znaczą to samo co w plikach.
- ` > ` i komendy z ukośnikiem są tylko wejściem; do plików nie trafiają.
- Daty względne (`@dzis`, `@jutro`, `@pojutrze`, `@pn`…`@nd`, `@+3d`, `@+2t`) są zapisywane jako zwykła data. `@` przed czymś, co nie jest datą, zostaje w tytule.
- Po ` > `: dokładny slug wygrywa; potem dopasowanie bez wielkości liter i polskich znaków do slugów i nazw. Słowo pasujące i do projektu, i do tematu oznacza projekt. Dwa trafienia albo brak trafienia: nic nie jest zapisywane, lista wyboru. Literówka o jeden znak od dokładnie jednej nazwy jest proponowana z etykietą „literówka”.
- Komendy: `/temat`, `/czeka`, `/powiaz`. Prośby do agenta zaczynają się od `?`.
- Nie ma usuwania i nie da się tą drogą założyć projektu.

## Potwierdzanie

- Pod polem zawsze widać odczyt: co powstanie albo co się zmieni i gdzie.
- Wszystko poza zwykłym dopisaniem do Skrzynki zatrzymuje się na kroku „Potwierdź”: pierwsze Enter pokazuje plan, drugie zapisuje, Esc anuluje. Po zapisie zostaje „Cofnij”.
- Wynik agenta to zawsze propozycje do zaznaczenia, jak przy sprawdzaniu projektu.

## Skrzynka

- Jedno pole u góry; lista wpisów; przy każdym jedna lokalna podpowiedź miejsca albo „?”, gdy dopasowanie jest słabe.
- Klawiatura: strzałki albo `j`/`k`, Enter przyjmuje podpowiedź (z potwierdzeniem), `>` otwiera wybór miejsca, `t` robi temat, spacja zaznacza, `u` cofa.
- Zaznaczenie kilku wpisów i linia `> cel` przenosi wszystkie; `/temat Nazwa > projekt` robi z nich jeden temat z tymi zadaniami, z zachowaniem identyfikatorów i notatek.

## MCP

- `command { text, apply }`: domyślnie podgląd (plan i linie Markdown, które zostałyby zapisane); zapis dopiero z `apply: true`. Błąd zaczyna się od „Nothing was changed:” i przy niejednoznaczności wymienia możliwe slugi. Prośby z `?` są odrzucane: serwer MCP nie woła modelu.
- `connections { ref }`: połączenia w obie strony i łańcuch „czeka na”, czytane tak jak na mapie.
- `create_task`, `update_task`, `create_topic`: pola `waits_for` i `related`, które dopisują po jednej linii.
- Instrukcje serwera uczą składni na trzech przykładach.
- Tryb tylko do odczytu dopuszcza wyłącznie podgląd.

## Cztery pomysły w zakresie 1.0.0

1. **Kilka wpisów → jeden temat** (opisane wyżej).
2. **Echo**: po zapisie przez chwilę widać dokładną linię i plik, do którego trafiła; `command` zwraca to samo.
3. **Podpowiedź duplikatu przy pisaniu**: „podobne do ^a1b2c3 — powiązać?”; jedno kliknięcie dopisuje powiązanie.
4. **Agent odpowiada komendą**: `?` na wpisie wstawia do pola gotową komendę do potwierdzenia; ta ścieżka nie ma narzędzi zapisu.

## Poza zakresem

Rozpoznawanie dat pisanych zdaniami, agent działający w tle, reguły uczone z decyzji, podgląd propozycji jako nakładka na mapie, powtarzalne zadania.

## Kolejność

1. Parser i resolver w `core` z testami (wszystkie przykłady, kolizje, literówki, daty).
2. Skrzynka: pole, odczyt, potwierdzenie, podpowiedzi lokalne, klawiatura, zaznaczanie.
3. Szybkie dodawanie i ⌘K na tym samym polu.
4. MCP: `command`, `connections`, nowe pola, instrukcje.
5. Agent w Skrzynce (`?`), echo, podpowiedź duplikatu.
6. Recenzje trzech modeli, poprawki, ponowna ocena, wydanie 1.0.0.
