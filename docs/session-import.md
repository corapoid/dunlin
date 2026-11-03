# Import sesji

**Settings → Harnesses → karta harnessa → Import sessions…** automatycznie
skanuje standardowe foldery sesji wybranego harnessa. Najpierw pojawia się
podgląd tytułów, katalogów, liczby wpisów i przypisań. Nic nie jest zapisywane
przed potwierdzeniem. Osobny link **Import a portable file…** pozostaje dla
jawnego formatu przenośnego; nie otwiera się on jako domyślna akcja importu.

## Obsługiwane źródła

- **OpenCode**: automatyczny skan nie czyta prywatnej bazy `opencode.db` ani
  katalogu `storage`. Użyj `opencode export <session>` i **Import a portable
  file…**. Obsługiwany jest eksport JSON `experimental.session.export` V2:
  `{ "data": { "info": {…}, "messages": […] } }`, albo samo `{info,messages}`.
  Akceptowane są także eksporty z wiadomościami `{info,parts}`.
  Publiczny endpoint to `GET /api/experimental/session/{sessionID}/export`;
  eksport można uzyskać przez udokumentowany klient/API OpenCode. Dunlin nie
  otwiera jego bazy SQLite ani nie odczytuje credential API.
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

Plik JSON może być tablicą eksportów tego samego typu. Limity podglądu:
16 MiB na plik, 64 MiB łącznie, 256 sesji, 30 000 wpisów/sesję. Błędy i pliki
pominięte z powodu limitów są pokazywane; nie ma cichego przycinania rozmowy.

## Dopasowanie i bezpieczeństwo

- Automatyczne przypisanie tylko do istniejących lokalnych projektów po
  kanonicznej ścieżce. Dla katalogu wewnątrz projektu wygrywa najgłębszy root.
- Nazwy projektów nie służą do dopasowania. Brak katalogu, przeniesiony projekt,
  duplikaty rootów lub ścieżka z innego komputera wymagają ręcznego przypisania.
- Sesje bez przypisania są pomijane. Można zmienić automatyczne przypisanie
  i odznaczyć wybrane sesje przed potwierdzeniem.
- Stała tożsamość harnessa i źródłowego ID chroni przed ponownym importem.
  Zapis sesji i transkryptów odbywa się w jednej transakcji, wyłącznie insert-only.
  Kolizja nigdy nie zastępuje istniejącej rozmowy ani transkryptu.
- Import jest **kopią historii**, nie wznowieniem sesji źródłowej. Nie podpinamy
  oryginalnego ACP session ID. Żaden prompt, tool call ani skrypt nie jest
  wykonywany podczas importu. Rekordy narzędzi i nieznane typy są zachowane jako
  tekst; załączniki nie są automatycznie pobierane ani otwierane ze źródła.
- To nie jest skaner billing usage: import nie rekonstruuje cen, czasu pracy ani
  rozliczeniowych liczników z niejednorodnych logów.

Skan jest ograniczony do standardowych folderów harnessa, maksymalnie 512
plików i 64 MiB; nie ma synchronizacji w tle ani importu sesji z relay/SSH.
Źródłowe pliki pozostają niezmienione.
