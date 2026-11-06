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
- Import zapisuje katalog źródłowy jako workspace sesji. Ponowny skan pozwala
  poprawić projekt także dla **Update · nazwa projektu**; jeśli ścieżka nie daje
  dopasowania, podgląd zachowuje dotychczasowy projekt. Zatwierdzenie zmienia
  przypisanie w historii bez nadpisywania treści i lokalnych kontynuacji.
  Przed przeniesieniem otwartej sesji zamknij jej agenta i wybierz inną sesję;
  aplikacja nie przenosi działających ani zaparkowanych runtime'ów między projektami.
- Stała tożsamość harnessa i źródłowego ID chroni przed ponownym importem.
  Nowe sesje są zapisywane strumieniowo w jednej transakcji. Błąd zapisu lub
  odczytu tymczasowej historii wycofuje nowe sesje i pozostawia podgląd do ponowienia.
  Zwykłe zamknięcie/Quit jest blokowane do zakończenia zapisu.
  Otwarcie Dashboardu uruchamia
  jednorazowe uzupełnienie metryk w istniejących kopiach OpenCode; ręczny skan
  oznacza je dodatkowo jako **Update · nazwa projektu**. Obie ścieżki dokładają
  brakujące tokeny/diffy i normalizują stare wpisy narzędzi; ręczny import pozwala
  też poprawić projekt i workspace. Nie zastępują tekstu rozmowy ani lokalnych
  kontynuacji.
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
| Historia projektu używa lokalnego `SessionSummary.project_id`, nie ID projektu harnessa ani nazwy repo | `session_list.rs::session_scope`, `views/sidebar.rs::sessions_section`, `storage/src/lib.rs::sessions` | Working tree | High | Brak | Smoke importu z dwoma projektami |
| Stare kopie można przypisać ponownie; zmiana dotyczy tylko projektu i workspace | `session_import.rs::finalize_import`, `assign_imported_location`, `Store::update_session_location` | Working tree | High | Stare importy nie zapisywały cwd; bez ponownego skanu nie da się go odzyskać | Ponowny skan źródłowego harnessa i zatwierdzenie podglądu |

Ścieżka danych: wybór harnessa → standardowy folder/baza lub ręcznie wskazane
pliki → walidowany `Candidate.source_id` → prywatna tymczasowa historia → ręczne
potwierdzenie/przypisanie lokalnego projektu → `imported_id(harness, source_id)` →
transakcja w bazie Dunlina. Każdy krok ma implementację w powyższych plikach;
żaden nie wymaga uruchamiania agenta, dostępu do credentiali ani zapisu do źródła.

Powiązanie potwierdzone w kodzie: `Candidate.directory` →
`match_project(kanoniczna ścieżka)` → lokalne `Project.id` →
`SessionSummary.project_id` → `session_scope` → historia wybranego projektu.
Nazwy repo i identyfikatory projektów z obcych harnessów nie są zamiennikami tego
powiązania. Pełna historia nie ma limitu 8 godzin; limit dotyczy wyłącznie skrótu
ostatnich sesji pod repozytorium w drzewie projektów. Nie zweryfikowano prywatnych
historii użytkownika. Weryfikacja poprawki: 17 testów importu, 34 testy storage
i natywny smoke 700 sesji w dwóch projektach przeszły. Smoke sprawdza widoczność
w historii projektu, korektę przypisania przez ponowny import, zachowanie treści,
fallback bez cwd i blokadę przeniesienia otwartej sesji. Następny krok: ponowny
import w aplikacji przez użytkownika. Właściciel: maintainer/użytkownik.

Potwierdzono: brak limitu liczby sesji, postęp i zapis strumieniowy. Odrzucono:
zwiększenie limitu do kolejnej arbitralnej liczby lub usunięcie walidacji plików.
Następny właściciel: maintainer aplikacji — testy jednostkowe, natywny smoke
dużego importu i ręczna ocena UX na rzeczywistych historiach (bez automatycznego
importowania danych użytkownika). Izolowany test GUI:
`dunlin --session-import-smoke <nowy absolutny katalog dunlin-session-import-smoke-…>`
tworzy 700 syntetycznych sesji i jeden błędny journal, renderuje rzeczywisty podgląd,
sprawdza postęp, zapis w tle, powtórzenie bez duplikatów, sprzątanie i blokadę Quit.
Sukces zapisuje w pliku `PASS` we wskazanym katalogu.
