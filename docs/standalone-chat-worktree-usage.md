# Standalone chat, worktree per subtask i billing evidence — plan i odbiór v1

Data: 2026-11-04. Zakres: cztery pionowe wycinki z roadmapy:

1. standalone chat / scratch workspace,
2. worktree per subtask,
3. attribution pracy wewnętrznej/subagentów w Usage,
4. ręczna QA logów i sync billing evidence przez relay.

Ten dokument jest planem implementacji i jednocześnie macierzą odbioru
dostarczonego zakresu v1. Nie oznacza ukończenia całego M6 ani pełnego
rozliczenia providera.

## Zasady wspólne

- Najpierw model domenowy i migracja, potem backend, UI i testy regresji.
- Żadna operacja nie uruchamia agenta, nie wysyła promptu, nie robi merge,
  push, fetch ani resetu bez osobnego, jawnego potwierdzenia.
- Nieznane dane (model, cena, attribution, koszt dziecka) nigdy nie są zerem.
- Każdy nowy storage/sync ma limit rozmiaru i jawny błąd zamiast cichego
  fallbacku.
- Stare projekty, sesje i transkrypty pozostają czytelne.

## 1. Standalone chat / scratch workspace

### Model

- `ProjectKind::{Repo, ChatSpace}` z `#[serde(default)] = Repo`.
- Migracja SQLite v8 dodaje `projects.kind` z domyślnym `repo`; istniejące
  projekty pozostają repozytoriami.
- Chat space ma własne `project.id`, własne sesje i transkrypty. Nie jest
  sztucznym projektem w drzewie Projects.
- Scratch workspace: `DUNLIN_DATA_DIR/chat-spaces/<chat-id>` (albo domyślny
  katalog danych aplikacji). Pliki nie są usuwane przy usuwaniu rozmowy.
- Opcjonalny kontekst repo: osobny chat space z jawną zgodą użytkownika i
  `path` wybranego folderu. To nowa przestrzeń z izolowaną historią; istniejące
  sesje projektowe nigdy nie zmieniają po cichu `cwd`.

### UI i zachowanie

- Command palette: **New Standalone Chat**, **New Chat in Repository…** oraz
  **New Chat in &lt;repo&gt;** dla każdego skonfigurowanego lokalnego projektu
  (wyszukiwanie po nazwie/ścieżce w palecie).
- Sidebar: sekcja `CHATS · SCRATCH WORKSPACES` nad projektami; chat space nie
  jest liczony jako projekt i nie pojawia się w `Switch to <project>`.
- Wybór chat space korzysta z tego samego composera, harnessa, modelu i trybu
  uprawnień co zwykły czat.
- Scratch chat bez `.git` nie otwiera Git history; pokazuje jawny komunikat i
  prowadzi do `New Chat in Repository…`.
- Historia pozostaje rozdzielona przez `project_id` sesji; sesje projektowe nie
  są przepinane. Draft composera pozostaje zachowaniem sesji, nie wspólnym
  buforem projektu.

### Granice v1

- Jeden chat space = jeden kontekst (scratch albo wskazany folder). Zmiana
  kontekstu tworzy nową przestrzeń zamiast mutować istniejące sesje.
- Brak automatycznego discovery repozytoriów i skanowania dysku.
- Chat space nie jest sandboxem; obowiązuje normalny permission mode.

### Odbiór

- Migracja v8 na bazie z v5/v6/v7 zachowuje projekty, sesje i transkrypty.
- Scratch chat tworzy katalog i osobną sesję; projekt repo nie dostaje tej
  sesji.
- `New Chat in Repository…` wymaga potwierdzenia i nie zmienia `cwd` żadnej
  istniejącej sesji projektowej.
- Restart aplikacji zachowuje wybór chat space i jego historię.
- Ręczna QA: klawiatura/focus, pusty stan, brak data dir, usunięcie folderu
  kontekstu, długi transkrypt.

## 2. Worktree per subtask

### Model i Git

- Nowa akcja `Action::SubtaskAdd { name, revision }`.
- Preflight: walidacja nazwy, rozwiązanie commita, bezpieczny katalog
  `dunlin-worktrees/subtask-<name>`, brak kolizji brancha
  `dunlin/subtask/<name>` i katalogu.
- Execute: `git worktree add -b dunlin/subtask/<name> -- <path> <commit>`.
  Żadnego agenta, merge, push ani usuwania.
- `dunlin_workspace::subtask::Registry` przechowuje task, branch, path, base i
  project id; waliduje duplikaty i limit 256 wpisów.
- Rejestr jest zapisywany w ustawieniach SQLite (`subtask-worktrees-v1`) i
  przeżywa restart.

### UI

- Command palette: **Subtask Worktrees**.
- Formularz: etykieta zadania, nazwa worktree, commit/branch/tag.
- Potwierdzenie pokazuje branch, ścieżkę, bazowy commit i jawne granice.
- Lista wpisów: obecność katalogu, branch, base, **Open as project** i
  **Remove worktree…**.
- Usunięcie worktree wymaga clean state i osobnego potwierdzenia; branch
  pozostaje. Zdalne usuwanie w tym v1 jest jawnie odrzucane.

### Odbiór

- Test repozytorium: powstaje dokładnie jeden branch i jeden worktree z HEAD.
- Duplikat nazwy/brancha i niebezpieczna nazwa są odrzucane w preflight.
- Stary plan po zmianie repozytorium nie wykonuje mutacji.
- Ręczna QA: hook failure, dirty worktree, locked worktree, brak `git`,
  otwarcie jako projekt i ponowne uruchomienie aplikacji.

## 3. Usage: attribution pracy wewnętrznej/subagentów

### Model

- `WorkKind::{Main, Subagent, Internal, Unknown}` i `Attribution` z
  `session`, `parent`, `agent`, `task`.
- `BillingEvent.attribution` jest serializowane; stare evidence bez pola
  dostaje `Unknown`, nie `Main`.
- Heurystyki:
  - Claude Code: `isSidechain`, `agentId` → subagent; `<synthetic>`,
    `isCompactSummary`, `isApiErrorMessage` → internal; reszta z `sessionId`
    → main.
  - Codex: `session_meta.parent_thread_id`/`parent_id`/`source.subagent` →
    subagent; `source`/`originator` internal → internal; brak sesji → unknown.
  - OpenCode: `session.parentID` → subagent; brak `sessionID` → unknown.
- `Totals` liczy eventy i znany koszt per kategoria. `Summary` ma
  `by_attribution`, a alerty jawnie ostrzegają o unknown.

### UI

- Usage pokazuje metryki **Subagents** i **Unknown attribution**.
- Przekrój **By work attribution**.
- Unknown nie jest wliczane do main work ani do znanego kosztu jako zero.

### Granice

- To nie jest pełne rozliczenie providera. Provider może nie ujawniać
  child/retry usage; wtedy pozostaje `Unknown`.
- Heurystyki opierają się wyłącznie na udokumentowanych polach logów; nowy
  format nie jest zgadywany.

### Odbiór

- Fixture Claude sidechain/synthetic, Codex subagent `session_meta` i OpenCode
  `parentID` dają oczekiwane kategorie.
- Dedupe i checkpointy nie zmieniają attribution.
- Ręczna QA: prawdziwe logi Claude/Codex/OpenCode, mieszane sesje, log bez
  session id, model `unpriced`.

## 4. Ręczna QA logów i sync billing evidence przez relay

### Lokalna QA

- `ScanReport.sources` zawiera provider, ścieżkę, rozmiar, mtime, offset,
  liczbę eventów i stan (`scanned`, `cached`, `skipped`, `error`).
- `Summary.qa_report()` tworzy kopiowalny raport: totals, attribution, źródła,
  unpriced modele i warningi. Nie zawiera promptów, odpowiedzi, env ani sekretów.
- Settings → Usage: **Copy QA report** oraz lista **Log evidence · manual QA**.

### Relay sync

- `POST /v1/billing/evidence` (member): waliduje `Evidence` v1, limit 2 MiB,
  zapis per `(user_id, device_id)`. Przyjmuje wyłącznie agregaty.
- `GET /v1/billing/evidence` (admin): zwraca per-user/device evidence.
  Member dostaje jawne `403`.
- Klient: **Sync evidence** wysyła agregat do każdego sparowanego relay;
  **Relay evidence** (admin) pobiera agregaty. Błędy są pokazywane per host.
- Brak automatycznego uploadu: sync jest zawsze jawną akcją użytkownika.

### Odbiór

- Relay test: member zapisuje, member nie czyta agregatu, admin czyta,
  niepoprawne evidence jest odrzucane.
- Ręczna QA: prawdziwy relay, dwa urządzenia, ponowny sync, brak sieci,
  member vs admin, duży raport > 2 MiB.

## Kolejność implementacji (wykonana)

1. `dunlin-usage`: attribution, źródła QA, `Evidence`, testy fixture.
2. `dunlin-relay`: tabela i endpointy evidence + testy auth/limitu.
3. `dunlin-workspace`: `SubtaskAdd`, rejestr subtask, testy Git.
4. `dunlin-storage`/`dunlin-domain`: `ProjectKind`, migracja v8, testy.
5. Desktop: chat space, subtask UI, Usage UI/sync, filtry sidebaru.
6. Dokumentacja, changelog i pełna walidacja.

## Czego ten zakres nie zamyka

- pełnego koordynatora/DAG/handoffów M6,
- automatycznego uruchamiania agenta per subtask i auto-merge,
- gwarantowanego limitu rachunku providera,
- automatycznego discovery i sync billing evidence w tle,
- zdalnego usuwania subtask worktree i pełnego sync grup/usage na iOS.
