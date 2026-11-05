# Import sesji

**Settings → Harnesses → karta harnessa → Import sessions…** automatycznie
skanuje standardowe foldery sesji wybranego harnessa. Najpierw pojawia się
podgląd tytułów, katalogów, liczby wpisów i przypisań. Nic nie jest zapisywane
przed potwierdzeniem. Osobny link **Import a portable file…** pozostaje dla
jawnego formatu przenośnego; nie otwiera się on jako domyślna akcja importu.

## Obsługiwane źródła

- **OpenCode**: Dunlin otwiera lokalny `opencode.db` wyłącznie w trybie
  read-only i czyta sesje najwyższego poziomu z bieżącego schematu
  (`session_v2` + `session_message`), z fallbackiem do starszych tabel
  `session`/`message`/`part`. Importuje tekst, reasoning, historię narzędzi,
  zapisane liczniki tokenów oraz statystyki diffów plików; nie odczytuje
  credentiali, blobów ani snapshotów. Nadal obsługiwany jest eksport
  JSON `experimental.session.export` V2 przez **Import a portable file…**:
  `{ "data": { "info": {…}, "messages": […] } }`, albo samo `{info,messages}`.
  Akceptowane są także eksporty z wiadomościami `{info,parts}`.
  Publiczny endpoint to `GET /api/experimental/session/{sessionID}/export`;
  eksport można uzyskać przez udokumentowany klient/API OpenCode. Dunlin nigdy
  nie odczytuje credentiali ani nie zapisuje do bazy OpenCode.
- **Claude Code**: jeden log sesji JSONL na plik (`sessionId`, `cwd`, `message`).
  Skan obejmuje `$CLAUDE_CONFIG_DIR/projects` lub `~/.claude/projects`, rekurencyjnie.
- **OpenAI Codex**: rollout JSONL (`session_meta`, `response_item`, `turn_context`).
  Skan obejmuje `$CODEX_HOME/sessions` lub `~/.codex/sessions`, rekurencyjnie.
- **Custom ACP** (również Claude/Codex): jawny format przenośny poniżej.
  ACP nie definiuje uniwersalnego eksportu historii; aplikacja nie zgaduje
  prywatnego formatu bazy niestandardowego adaptera.

```json
{
  "format": "dunlin.session.v1",
  "id": "stable-source-session-id",
  "title": "Refaktoryzacja API",
  "directory": "/Users/me/code/project",
  "model": "model-name",
  "updated_at": "2026-10-30T12:00:00Z",
  "messages": [
    {"role": "user", "content": "Sprawdź API", "timestamp": "2026-10-30T11:59:00Z"},
    {"role": "assistant", "content": "Wynik analizy", "timestamp": "2026-10-30T12:00:00Z"}
  ]
}
```

Plik JSON może być tablicą eksportów tego samego typu. **Nie ma limitu liczby
skanowanych plików ani importowanych sesji**, również w jednej tablicy JSON lub
bazie OpenCode. Nie ma też zbiorczego limitu 64 MiB. Pojedyncze pliki nadal
podlegają walidacji: maksymalnie 16 MiB i 30 000 wpisów/sesję. Błędny lub zbyt
duży plik jest zgłaszany, ale nie zatrzymuje odczytu pozostałych plików.

Skanowanie i zapis działają w tle. Status pokazuje liczbę znalezionych plików,
następnie odczytanych plików/sesji i zapisywanych sesji (`wykonane / wszystkie`).
Podgląd trzyma pełne historie w prywatnym katalogu tymczasowym na dysku, nie
wszystkie naraz w RAM. Lista renderuje tylko widoczne wiersze; ostrzeżenia mają
osobny przewijany obszar. Katalog tymczasowy jest sprzątany po anulowaniu,
zastąpieniu podglądu lub zakończeniu importu. Potrzebne jest wolne miejsce na
kopie historii i transakcję SQLite.

## Dopasowanie i bezpieczeństwo

- Automatyczne przypisanie tylko do istniejących lokalnych projektów po
  kanonicznej ścieżce. Dla katalogu wewnątrz projektu wygrywa najgłębszy root.
- Nazwy projektów nie służą do dopasowania. Brak katalogu, przeniesiony projekt,
  duplikaty rootów lub ścieżka z innego komputera wymagają ręcznego przypisania.
- Sesje bez przypisania są pomijane. Można zmienić automatyczne przypisanie
  i odznaczyć wybrane sesje przed potwierdzeniem.
- Stała tożsamość harnessa i źródłowego ID chroni przed ponownym importem.
  Nowe sesje są zapisywane strumieniowo w jednej transakcji. Błąd zapisu lub
  odczytu tymczasowej historii wycofuje nowe sesje i pozostawia podgląd do ponowienia.
  Zwykłe zamknięcie/Quit jest blokowane do zakończenia zapisu.
  Otwarcie Dashboardu uruchamia
  jednorazowe uzupełnienie metryk w istniejących kopiach OpenCode; ręczny skan
  oznacza je dodatkowo jako **Update imported copy**. Obie ścieżki dokładają
  tylko brakujące tokeny/diffy i normalizują stare wpisy narzędzi, nie
  zastępując tekstu rozmowy ani lokalnych kontynuacji.
- Import jest **kopią historii**, nie wznowieniem sesji źródłowej. Nie podpinamy
  oryginalnego ACP session ID. Żaden prompt, tool call ani skrypt nie jest
  wykonywany podczas importu. Rekordy narzędzi i nieznane typy są zachowane jako
  tekst; załączniki nie są automatycznie pobierane ani otwierane ze źródła.
- To nie jest skaner billing usage: import nie rekonstruuje cen ani czasu pracy.
  Przenosi wyłącznie liczniki tokenów i zmienionych linii zapisane przez źródło.

Skan plików obejmuje standardowe foldery harnessa, bez limitu liczby plików ani
głębokości katalogów; pomija ukryte katalogi i dowiązania symboliczne. OpenCode
odczytuje wszystkie sesje najwyższego poziomu; istniejące zabezpieczenia pojedynczej
historii (30 000 wiadomości, 60 000 części, skracanie dużych wyników narzędzi)
pozostają bez zmian. Uzupełnianie metryk również przetwarza historie pojedynczo.
Nie ma synchronizacji w tle ani importu sesji z relay/SSH; źródłowe pliki i baza
OpenCode pozostają niezmienione.

## Kontrakt i regresje

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| Ograniczenia 512/256/64 MiB były lokalną polityką podglądu, nie limitem harnessa | `session_import.rs`: `collect_session_files`, `discover_session_files`, `read_files`; `adapters.rs::parse`; `opencode_db.rs`: zapytania sesji | Dunlin beta.8 / working tree | High | Brak | Testy 600 plików >64 MiB i obu schematów bazy |
| Źródło jest read-only; docelowa tożsamość to harness + source ID | `opencode_db.rs::visit`, `session_import.rs::imported_id`, `adapters.rs::match_project`, `Store::import_session_stream` | Working tree | High | Nie testowano rzeczywistych historii użytkownika | Izolowane fixture JSON/JSONL/SQLite |
| Błąd strumienia wycofuje nowe sesje; powtórzenie nie nadpisuje rozmów | `storage/src/lib.rs::streamed_import_reports_duplicates_and_rolls_back_on_a_source_read_failure`, `session_import.rs::staged_read_failure_rolls_back_inserts_and_leaves_preview_available_for_retry` | Working tree | High | Brak | `cargo test -p dunlin-storage`; testy aplikacji |

Ścieżka danych: wybór harnessa → standardowy folder/baza lub ręcznie wskazane
pliki → walidowany `Candidate.source_id` → prywatna tymczasowa historia → ręczne
potwierdzenie/przypisanie lokalnego projektu → `imported_id(harness, source_id)` →
transakcja w bazie Dunlina. Każdy krok ma implementację w powyższych plikach;
żaden nie wymaga uruchamiania agenta, dostępu do credentiali ani zapisu do źródła.

Potwierdzono: brak limitu liczby sesji, postęp i zapis strumieniowy. Odrzucono:
zwiększenie limitu do kolejnej arbitralnej liczby lub usunięcie walidacji plików.
Następny właściciel: maintainer aplikacji — testy jednostkowe, natywny smoke
dużego importu i ręczna ocena UX na rzeczywistych historiach (bez automatycznego
importowania danych użytkownika). Izolowany test GUI:
`dunlin --session-import-smoke <nowy absolutny katalog dunlin-session-import-smoke-…>`
tworzy 700 syntetycznych sesji i jeden błędny journal, renderuje rzeczywisty podgląd,
sprawdza postęp, zapis w tle, powtórzenie bez duplikatów, sprzątanie i blokadę Quit.
Sukces zapisuje w pliku `PASS` we wskazanym katalogu.
