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
- Lokalny indeks historii C1 jest osobnym dostarczalnym wycinkiem P2; nie wymaga
  ukończenia pełnego launchera i nie oznacza odbioru późniejszych źródeł.

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

### C1 — samodzielne lokalne FTS5 sesji i transkryptów

To pierwszy wycinek roadmapy #23 (P2), niezależny od pełnego launchera A/B.
Istniejący zapis transkryptu jest snapshotem JSON w storage; indeks nie może
traktować surowego JSON jako tekstu rozmowy. Projektowe wyszukiwanie plików
pozostaje bez zmian. Pliki, symbole, artefakty i relay nie blokują C1.

1. [ ] Ustalić dokument indeksu: lokalny project/session ID, stabilny message/tool
   ID, rodzaj treści, revision źródła i widoczny tekst. Dla starszych/importowanych
   wiadomości bez ID dodać trwałe mapowanie przy normalizacji; offset w wektorze
   ani sam hash tekstu nie zapewniają tożsamości po edycji/identycznych wiadomościach.
2. [ ] Dodać migrację FTS5 i tabel pomocniczych bez zależności storage → desktop.
   Storage otrzymuje typowane dokumenty od projekcji; snapshot historii pozostaje
   źródłem prawdy. Backfill ma trwały checkpoint, wersję ekstraktora i bounded batch;
   wznowienie po awarii jest idempotentne, a przebudowa nie kasuje historii.
3. [ ] Zasilać indeks z jednej ścieżki zapisu/importu/edycji; współdzielić transakcję
   lub trwały outbox z rewizją snapshotu. Obsłużyć streaming bez pełnej reindeksacji
   całej historii co chunk, restart podczas zapisu, rename sesji i reimport.
   Podczas migracji nowe zapisy nie giną pomiędzy checkpointem a końcem backfillu.
4. [ ] Usuwanie sesji/projektu, Clear history, retencja i przyszłe Undo aktualizują
   indeks razem ze źródłem; tombstoned rekordy są od razu niewidoczne. Usunięcie
   trafia też do FTS segmentów według jawnej polityki purge/secure-delete właściwej
   dla użytej wersji SQLite; nie obiecywać fizycznego wymazania WAL/backupów.
5. [ ] Domyślnie indeksować tytuł sesji oraz widoczny tekst użytkownika/asystenta.
   Wyłączyć reasoning, raw tool payloads, załączniki/binary, pola credentials/env
   i ukryte dane. Tool text dopiero po jawnej polityce redakcji/opt-in. Wykluczyć
   lokalne cache transkryptów relay — wyszukiwanie remote jest osobnym etapem C2.
6. [ ] API zapytania: scope, filtry, bounded limit/cursor, generation i cancellation.
   Parametryzować MATCH; zwykłe pole wyszukiwania nie udostępnia przypadkiem całej
   gramatyki FTS. Niepoprawne zapytanie daje czytelny błąd, nie crash ani SQL fallback.
   Ranking: dopasowanie tytułu/treści + aktywny scope/recency, stabilny tie-breaker.
7. [ ] Dodać mały widok wyszukiwania historii korzystający z kontraktu wyników A,
   bez wymogu gotowej wspólnej palety. Wynik zawiera projekt/sesję, snippet i rodzaj;
   otwiera dokładną wiadomość poza oknem 120 elementów i rozwija właściwą kartę.
   Sanitizować snippet, nie interpretować tekstu rozmowy jako markup UI.
8. [ ] Pokazać indeksowanie/częściowe wyniki, brak indeksu i możliwość rebuild.
   Zapytania/backfill poza wątkiem UI, z limitami czasu, pamięci, tekstu i stron;
   wstrzymanie indeksowania nie blokuje zapisu rozmowy.

Odbiór C1: Unicode/diakrytyki i frazy, dwa identyczne komunikaty, edycja treści,
usunięcie/Clear/Undo, przerwany backfill, aktualizacja podczas backfillu,
wykluczenie danych prywatnych/remote i stale wynik po zmianie scope. Fixture dużej
historii ma ustalony rozmiar/urządzenie i pomiar responsywności; nie wpisywać
niezmierzonego SLA jako wyniku. Testować rzeczywiste zapytania SQLite, nie mock FTS.

### C2 — rozszerzenia po lokalnym FTS

1. [ ] Podłączyć C1 do wspólnej palety A/B i lokalnego `⌘F` rozmowy. Korzystać
   z tego samego dokumentu/ID i nawigacji, bez drugiego indeksu transkryptów.
2. [ ] Rozszerzyć indeks o pliki z osobną polityką names/contents, ignorowania,
   rozmiaru i watcherów. Nowe źródła mają własną wersję ekstraktora/backfill;
   nie uzależniać zapisu rozmów od gotowości indeksu plików.
3. [ ] Dodać zatwierdzone decyzje i artefakty z aktualnością/retencją. Symbole
   wykorzystują plan [7](07-lsp-agent-workbench.md); brak LSP nie blokuje launchera.
4. [ ] Dla relay przyjąć jawny model wyszukiwania serwerowego; brak domyślnego
   kopiowania historii na Maca. Stary serwer pokazuje unavailable, bez local fallback.
5. [ ] Ustalić limity indeksu, snippetów, czasu i stron; raportować częściowe wyniki.
   Zachować kontrakty migracji, retencji/Undo i prywatności C1 dla nowych źródeł.
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
