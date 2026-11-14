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
^a1b2c3 /czeka odpis z sądu                 to samo, zadanie albo temat wskazany nazwą
umowa z dostawcą /powiaz płatności          podmiot i odwołanie nazwą (bez ` > `)
[ ] Umowa z dostawcą /powiaz ^a1b2c3        `[ ]` na początku: zawsze nowe zadanie
?rozpisz ^a1b2c3                            prośba do agenta
?dopasuj                                    agent proponuje miejsca dla zaznaczonych albo całej Skrzynki
```

Reguły:

- `!priorytet`, `@termin`, `^id`, `[[projekt/temat]]` i znaczniki statusu `[ ] [/] [x] [!]` znaczą to samo co w plikach.
- ` > ` i komendy z ukośnikiem są tylko wejściem; do plików nie trafiają.
- Daty względne (`@dzis`, `@jutro`, `@pojutrze`, `@pn`…`@nd`, `@+3d`, `@+2t`) są zapisywane jako zwykła data. `@` przed czymś, co nie jest datą, zostaje w tytule.
- Po ` > `: dokładny slug wygrywa; potem dopasowanie bez wielkości liter i polskich znaków do slugów i nazw. Słowo pasujące i do projektu, i do tematu oznacza projekt. Dwa trafienia albo brak trafienia: nic nie jest zapisywane, lista wyboru. Literówka o jeden znak od dokładnie jednej nazwy jest proponowana z etykietą „literówka”.
- Komendy: `/temat`, `/czeka`, `/powiaz`. Prośby do agenta zaczynają się od `?` (w Skrzynce to też skrót przełącznika Komenda | Agent).
- Po `/czeka` i `/powiaz` zadanie albo temat można wskazać zwykłymi słowami zamiast `^id` / `[[…]]`; tak samo słowa na początku linii z `/czeka` albo `/powiaz` (bez ` > `) wskazują, co zmienić. Szukanie w otwartych zadaniach (tytuł) i otwartych tematach aktywnych projektów (tytuł, slug), bez wielkości liter i polskich znaków; nazwa projektu może zawęzić, słowo od czterech liter może mieć jedną literówkę. Jedno trafienie: przyjęte (literówka do potwierdzenia, MCP jej nie zapisze); kilka: nic się nie zapisuje, lista do wyboru z rodzajem i miejscem; podmiot bez trafienia: nowe zadanie (podgląd to mówi). W Skrzynce `/czeka …` bez podmiotu dotyczy zaznaczonych wpisów, a gdy nic nie jest zaznaczone, wpisu podświetlonego na liście.
- Słowo w trakcie pisania (`/`, `/cz`, `^`, ` > ` bez niczego dalej, nazwa jeszcze bez jednego trafienia) daje neutralną podpowiedź, nie błąd.
- Nie ma usuwania i nie da się tą drogą założyć projektu.

## Potwierdzanie

- Pod polem zawsze widać odczyt: co powstanie albo co się zmieni i gdzie.
- Wszystko poza zwykłym dopisaniem do Skrzynki zatrzymuje się na kroku „Potwierdź”: pierwsze Enter pokazuje plan, drugie zapisuje, Esc anuluje. Po zapisie zostaje „Cofnij”.
- Wynik agenta to zawsze propozycje do zaznaczenia, jak przy sprawdzaniu projektu.

## Skrzynka

- Jedno pole u góry; lista wpisów; przy każdym jedna lokalna podpowiedź miejsca albo „?”, gdy dopasowanie jest słabe.
- Klawiatura: strzałki albo `j`/`k`, Enter przyjmuje podpowiedź (z potwierdzeniem), `>` otwiera wybór miejsca, `t` robi temat, `/` zaczyna komendę dla wpisu, spacja zaznacza, `u` cofa.
- Listy pod polem (miejsca po ` > `, komendy po `/`, zadania i tematy po nazwie, literówka, wybór przy niejednoznaczności, projekt dla `/temat`) działają tak samo: ↑/↓ wybór, Enter albo Tab wstawia, Esc najpierw zamyka listę, potem cofa; mysz podświetla to samo co klawiatura. Dopasowane litery są wyróżnione.
- Przełącznik Komenda | Agent przy polu (⌘J). Agent dostaje wybrane wpisy i MontyBrain tylko do odczytu („Tylko MontyBrain”, w tle) albo, na jedną prośbę i po zgodzie, sesję z własnymi narzędziami użytkownika („Moje narzędzia”: Slack, poczta, sterowanie komputerem, jeśli są skonfigurowane). Ta sesja działa w aplikacji: panel z rozmową, wywołaniami narzędzi i prośbami o zgodę (Zezwól / Zezwól na tę sesję / Odmów), wiadomości w trakcie, Przerwij i Zakończ; zapis w MontyBrain i edycja jego plików są odrzucane bez pytania, a blok `montybrain-zmiany` z odpowiedzi staje się propozycjami do zaznaczenia. Sesja w Terminalu zostaje jako druga możliwość (odpowiedź wraca przez plik albo wklejenie). To samo („z moimi narzędziami”) jest przy zleceniu w widoku projektu.
- Zaznaczenie kilku wpisów i linia `> cel` przenosi wszystkie; `/temat Nazwa > projekt` robi z nich jeden temat z tymi zadaniami, z zachowaniem identyfikatorów i notatek.

## MCP

- `command { text, apply }`: domyślnie podgląd (plan i linie Markdown, które zostałyby zapisane); zapis dopiero z `apply: true`. Błąd zaczyna się od „Nothing was changed:” i przy niejednoznaczności wymienia możliwe slugi albo `^id` (także dla zadań i tematów wskazanych słowami). Prośby z `?` są odrzucane: serwer MCP nie woła modelu.
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
