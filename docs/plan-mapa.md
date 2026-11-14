# Plan: widok „Mapa” (mapa myśli)

Stan: plan po recenzji trzech modeli (Astra, Grok, Hydrogen). Nic nie jest zbudowane.

## Cel

Jeden obraz całej pracy: projekty, ich tematy i zadania oraz połączenia między nimi w poprzek tej hierarchii. Wzorem jest graf z Obsidiana, ale ma być przystępniejszy i czytelny bez wprawy.

Skala, dla której projektujemy: do 5 projektów, do 40 tematów, do 300 zadań (z czego większość zrobiona).

## Pytania, na które mapa odpowiada lepiej niż inne widoki

1. **Co na co czeka?** Na co czeka ten temat i czy tamta rzecz się rusza; co zależy od tego tematu.
2. **Co łączy projekty?** Które tematy w różnych projektach to ta sama praca.
3. **Jaki kontekst dać agentowi?** Temat razem z jego sąsiadami, do skopiowania jednym kliknięciem.

„Dziś” już ustawia kolejność uwagi, tablica grupuje po statusie, widok projektu pokazuje zadania. Mapa nie ma ich powtarzać.

## Układ

Płótno z przesuwaniem i powiększaniem, a na nim wyspy projektów.

```
+--- WhenUBuy ● -----------------------+    +--- Morwisko ● -------------+
|  │  Płatności      w toku   3  ⏳ 2↔ |    |  │  Mapa        w toku   2 |
|  │  Checkout       czeka    1        |    |  │  MCP         w toku     |
|  │  A/B testy      planowany         |    +----------------------------+
|  │     ├ zadanie a                   |
|  │     └ zadanie b      (+4)         |
+--------------------------------------+
```

- **Wyspy** w siatce o dwóch kolumnach, w kolejności paska bocznego. Odstęp między wyspami wyraźnie większy niż między wierszami, każda wyspa w cienkiej ramce: bliskość znaczy tylko „ten sam projekt”.
- **Tematy** jako poziome wiersze pod nazwą projektu, wzdłuż jednej pionowej linii. Każdy wiersz ma własne miejsce, więc etykiety nie mogą na siebie nachodzić.
- **Kolejność tematów**: data utworzenia, potem slug. Status, data aktualizacji ani liczba zadań nie przesuwają wiersza. Nowy temat dopisuje się na końcu; wpis w dzienniku niczego nie przestawia.
- **Zadania** pojawiają się dopiero po rozwinięciu tematu kliknięciem: do ośmiu otwartych, reszta jako „+N”. Zrobione są tylko licznikiem.
- **Powiększenie tylko skaluje.** Nie zmienia położenia i samo niczego nie rozwija.
- Tematy zakończone oraz projekty wstrzymane i zarchiwizowane są ukryte; przełącznik „Pokaż zakończone”.
- Skrzynki na mapie nie ma.

Odrzucone: drzewo promieniste (przy 40 tematach poziome etykiety się nie mieszczą, a przy górze i dole koła przestają być poziome) i drzewo dwustronne (rodzeństwo trafia na przypadkowe strony, obrazu nie da się zapamiętać).

## Połączenia

Zapisane w Markdownie, jak wikilinki w Obsidianie. Bez nowego pliku pomocniczego i bez zmiany formatu.

**Składnia**

- `[[slug]]` — temat w tym samym projekcie,
- `[[projekt/slug]]` — temat w innym projekcie (ten sam slug może istnieć w dwóch projektach),
- `^id` — zadanie.

**Gdzie są czytane**: tylko opis stanu tematu i notatki zadań, z pominięciem bloków kodu. **Dziennik nie jest czytany**: wpis to historia, powtórzone wzmianki tworzyłyby fałszywe powiązania, a usunięcie połączenia wymagałoby zmieniania przeszłości.

**Rodzaje — tylko dwa**

| Rodzaj | Zapis | Znaczenie |
|---|---|---|
| Powiązane | odwołanie gdziekolwiek w opisie stanu lub notatce | nieskierowane |
| Czeka na | osobna linia `Czeka na: [[projekt/slug]]` albo `Czeka na: ^id` | skierowane: od czekającego do tego, na co czeka |

**Reguły**

- Odwołanie do czegoś, czego nie ma, jest ostrzeżeniem w panelu, nie węzłem. Aplikacja niczego nie zgaduje.
- Zmiana tytułu tematu nie zmienia sluga, a `^id` przetrwa przeniesienie zadania, więc odwołania psuje tylko ręczna zmiana nazwy pliku.
- Pętle w „czeka na” są wykrywane i przecinane przy rysowaniu.
- Opis narzędzi MCP wyjaśnia składnię, inaczej agenci jej nie użyją.

Odrzucone: połączenie „wspólna gałąź” (dzisiejszy zapis przypisuje gałąź tylko do jednego tematu, więc nie da relacji między tematami) i „wspólny link” jako linia na mapie (tworzy kliki; później najwyżej lista w panelu).

## Co widać

**Domyślnie**: wyspy, wiersze tematów ze statusem (ikona i słowo), liczbą otwartych zadań i znacznikiem „wymaga uwagi” (ta sama logika co w „Dziś”). Temat z połączeniami ma mały znacznik z ich liczbą; temat, który na coś czeka, ma osobny znacznik. **Żadnych linii w poprzek.**

**Po wybraniu węzła** (skupienie): rysują się jego połączenia, najwyżej osiem, pozostałe są w panelu z licznikiem „+N”. Sąsiedzi zostają wyraźni, reszta gaśnie. Linie między wyspami biegną odstępami, nie przez wiersze.

**Panel boczny** wybranego węzła:

- status (ta sama kontrolka co w projekcie), opis stanu, otwarte zadania,
- połączenia w obie strony, z pierwszym zdaniem opisu każdego sąsiada,
- dla „czeka na”: cel, jego status i czas ostatniego wpisu; cały łańcuch w jednej linii, do czterech poziomów,
- ostrzeżenia o odwołaniach donikąd,
- „Otwórz w projekcie” i „Kopiuj kontekst” (z sąsiadami).

## Interakcje

- przesuwanie i powiększanie; „Dopasuj widok” (klawisz 0),
- kliknięcie tematu: skupienie i panel; drugie kliknięcie albo strzałka w prawo: rozwinięcie zadań,
- Enter: przejście do tematu w widoku projektu; Esc: koniec skupienia,
- strzałki: poruszanie się po wierszach w kolejności listy,
- filtry: projekt, „wymaga uwagi” (przygasza resztę), „Pokaż zakończone”,
- adres `#/mapa?focus=…`: wejście prosto na węzeł z palety ⌘K („Pokaż na mapie”) i z „Dziś”.

## Technika

- **Graf**: czysta funkcja w TypeScripcie z migawki danych (projekty, tematy, otwarte zadania, połączenia), bez zależności od dziennika, żeby nie przeliczać mapy przy każdym wpisie.
- **Układ**: własna czysta funkcja zwracająca prostokąty. Bez biblioteki grafowej i bez symulacji.
- **Rysowanie**: wiersze jako zwykłe przyciski HTML w kolejności dokumentu, pod nimi warstwa SVG z liniami. Klawiatura i czytnik ekranu działają dzięki temu od pierwszego dnia, a style są te same co w reszcie aplikacji.
- **Bez animacji układu.** Bez nowych zależności.
- „Wymaga uwagi” wymaga danych spoza migawki (czekające, rozjazd, propozycje): używamy istniejącej funkcji z „Dziś”, a mapa nie może przeskakiwać, gdy te dane dojdą z opóźnieniem.
- Zadania bez `^id` (przed normalizacją) dostają klucz zastępczy i nie mogą być celem odwołania.

## Etapy

| Etap | Zakres | Wynik |
|---|---|---|
| 0 | Klikalny szkic układu na danych sztucznych (5 projektów, 40 tematów, 300 zadań, długie polskie tytuły) i funkcja układu z testami | wiadomo, czy układ jest czytelny, zanim powstanie reszta |
| 1 | Mapa tylko do odczytu: rozpoznawanie odwołań, skupienie, panel, filtry, klawiatura, wejście z ⌘K i „Dziś”, opis składni w narzędziach MCP | używalna mapa |
| 2 | Tworzenie i usuwanie połączeń z mapy (dopisanie linii w opisie stanu lub notatce, nigdy przepisanie całości), „na co czeka” jako wybór tematu w kroku czekania, połączenia w propozycjach agenta, narzędzie MCP do odczytu połączeń | mapa jako miejsce pracy |
| 3 | Eksport PNG/SVG, lista wspólnych linków w panelu, pomiary wydajności | dopracowanie |

## Kryteria gotowości etapu 1

- 40 tematów i 150 otwartych zadań: żadne dwa prostokąty układu na siebie nie nachodzą (test na wyniku funkcji układu).
- Dodanie zadania, wpisu w dzienniku albo zmiana statusu nie zmienia kolejności ani położenia innych wierszy.
- Nowy temat dopisuje się na końcu swojej wyspy; inne wyspy się nie ruszają.
- Odwołanie donikąd jest ostrzeżeniem, pętla „czeka na” niczego nie zawiesza, zadanie bez `^id` nie psuje mapy.
- Odpowiedź na „na co czeka temat X i czy to się rusza” w jednym kliknięciu.
- Całość obsługiwalna z klawiatury; działa w oknie 760×480 (tytuły skracane, pełny tytuł w panelu).
- Położenie i powiększenie zostają po odświeżeniu danych z dysku.

## Ryzyka

- **Mało połączeń na początku.** Dziś w danych nie ma żadnych odwołań, więc pierwsza mapa pokaże samą hierarchię. Wartość przyjdzie dopiero, gdy agenci i ty zaczniecie je wpisywać; stąd opis składni w MCP już w etapie 1 i tworzenie połączeń w etapie 2.
- **Mniej „organiczna” niż Obsidian.** Recenzenci zgodnie przesunęli projekt w stronę uporządkowanego układu. To cena czytelności.
- **Plątanina przy wielu połączeniach**: linie tylko przy skupieniu, limit ośmiu.
- **Własny układ to własne utrzymanie**: jeden układ, bez opcji, testowany jak logika „Dziś”.
- **Dopisywanie połączeń do notatek** (etap 2): zapis notatki przepisuje dziś cały blok, więc trzeba dopisywać linię, a nie generować tekst od nowa.
