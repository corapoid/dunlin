# 8. Search Everywhere i szybkie otwieranie

## Cel i stan obecny

Odnajdywać plik, sesję, decyzję lub akcję bez pamiętania struktury paneli.
Inspiracje: JetBrains Search Everywhere, VS Code Quick Open i Orca Jump Palette.
Obecna paleta filtruje polecenia, projekty i gałęzie; wyszukiwanie projektu działa
osobno. Globalny indeks FTS5 jest już pozycją roadmapy, a nie dostarczoną funkcją.

## Zakres v1 i miejsca integracji

- Lokalny launcher: polecenia, projekty, otwarte karty/sesje i nazwy plików.
- Fuzzy matching, ostatnio używane wyniki, filtry i podgląd bez aktywacji.
- `apps/desktop/src/{app.rs,views/palette.rs,workspace_panes.rs,session_list.rs}`.
- `crates/workspace/src/{search.rs,tree.rs}` i `crates/storage/src/lib.rs`.
- Pełna treść rozmów, decyzje/artefakty, symbole i relay jako kolejne etapy.

## Etap A — kontrakt wyników i responsywny launcher

1. [ ] Wprowadzić typowany wynik: kind, stabilne ID, host/project/session,
   źródło dopasowania, revision i docelowa akcja. Nie identyfikować kart tytułem.
2. [ ] Oddzielić providerów wyników od UI; akcje pochodzą z planu
   [16](16-akcje-skroty-i-fokus.md). Usunąć blokujące odczyty Git/dysku z otwierania
   palety; provider zwraca loading, błąd lub bounded batch poza wątkiem UI.
3. [ ] Zdefiniować ranking: trafność nazwy/ścieżki, aktywny scope i recency.
   Otwarta karta wygrywa z duplikatem tego samego pliku. Wyniki mają jawny host.
4. [ ] Dodać anulowanie, generation ID zapytania i deterministyczne tie-breakery.
   Spóźnione wyniki nie zastępują nowszych; podgląd nie wykonuje poleceń ani promptów.
5. [ ] Zamrozić listę i kolejność recents przy otwarciu. Wyszukiwanie nie może
   przesuwać zaznaczenia przez tło; po zmianie zapytania utrzymać cel, jeśli istnieje.

## Etap B — obsługa i bezpieczne otwieranie

1. [ ] Zachować istniejące wejścia `⌘K`, `⌘⇧P` i wyszukiwanie projektów;
   dodać `⌘P` dla plików/kart po audycie skrótów. Wspólny silnik, różne filtry.
2. [ ] Dodać filtry kategorii, nawigację klawiaturą, Escape i powrót fokusu.
   Nazwa pliku jest pierwsza, ścieżka/host drugie; pełna informacja w AX/tooltipie.
3. [ ] Otwierać istniejący canonical buffer i właściwą sesję/panel. Respektować
   dirty-buffer guards, usunięte cele, utratę trust i zmianę uprawnień relay.
4. [ ] Nie odczytywać treści sekretów, ignorowanych plików ani obcych katalogów
   jako efektu wyszukiwania nazw. Zdefiniować oddzielną politykę names vs contents;
   brak wyniku nie oznacza nieistnienia pliku poza zakresem indeksowania.

## Etap C — globalny indeks i pozostałe źródła

1. [ ] Wersjonować FTS5 i wykonać backfill w tle z checkpointem postępu.
   Aktualizacje oraz usunięcia rozmów, Undo i retencja z planów 2/3 aktualizują
   indeks transakcyjnie lub przez trwałą kolejkę; indeks jest odbudowywalny.
2. [ ] Dodać treść rozmów i lokalne `⌘F`: trafienie prowadzi do właściwego ID
   także poza oknem 120 elementów i wewnątrz zwiniętej karty. Rozróżnić zakres
   prompt/odpowiedź/tool; nie indeksować payloadów wykluczonych polityką prywatności.
3. [ ] Dodać zatwierdzone decyzje i artefakty z aktualnością/retencją. Symbole
   wykorzystują plan [7](07-lsp-agent-workbench.md); brak LSP nie blokuje launchera.
4. [ ] Dla relay przyjąć jawny model wyszukiwania serwerowego; brak domyślnego
   kopiowania historii na Maca. Stary serwer pokazuje unavailable, bez local fallback.
5. [ ] Ustalić limity indeksu, snippetów, czasu i stron; raportować częściowe wyniki.
   Ranking nie potrzebuje LLM ani zewnętrznej usługi embeddingów w v1.

## Testy i kryteria odbioru

- [ ] Ranking: literówki, kilka segmentów ścieżki, Unicode, identyczne nazwy na
  różnych hostach, recency i brak przetasowania recents pod kursorem.
- [ ] Anulowane zapytanie, zmiana projektu, usunięty cel i stale response.
- [ ] Migracja/backfill/purge/Undo nie ujawniają usuniętych lub niedostępnych danych.
- [ ] Wynik rozmowy otwiera właściwą wiadomość, a plik nie duplikuje bufora.
- [ ] Natywny smoke: dark/light, dwa panele, VoiceOver i obsługa bez myszy.
- [ ] Ustalić budżet czasu na urządzeniu referencyjnym; zmierzyć duże repo
  i długą historię. Otwarcie launchera nie czeka na Git, sieć ani pełny indeks.
- [ ] Raportować odbiór launchera, FTS i relay osobno; nie zamykać całego zakresu
  globalnego wyszukiwania po samym dodaniu fuzzy match do komend.

## Zależności i granice

Minimalny rejestr akcji (16); załączniki kontekstu (9) są opcjonalną akcją wyniku.
Nie dodawać wykonywania natural-language commands ani automatycznej aktywacji AI.

Źródła: [JetBrains](https://www.jetbrains.com/help/idea/searching-everywhere.html),
[Orca](https://www.onorca.dev/docs/model/quick-open).
