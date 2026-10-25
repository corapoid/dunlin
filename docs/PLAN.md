# Dunlin — plan techniczny

Ten dokument opisuje kierunek rozwoju Dunlina po wersji `0.1`: docelowy model
usług, decyzje architektoniczne i kolejność prac. Roadmapa z konkretnymi
kamieniami milowymi jest w [`ROADMAP.md`](ROADMAP.md).

## Zasady

1. Dunlin pozostaje natywną aplikacją macOS w Rust/GPUI. Nie wprowadzamy
   Electrona ani Monaco.
2. Widok Markdown ma być wizualny, ale bez webview: edytor blokowy oparty o AST
   Markdown, z możliwością przełączenia na surowy tekst.
3. Computer use dostaje osobny helper w Swift/AppKit. Dzięki temu uprawnienia
   Accessibility i Screen Recording są przyznawane helperowi, a nie całej
   aplikacji — tak jak robi to Orca.
4. Automatyzacje mogą działać przy zamkniętym Dunlinu: lokalnie przez
   LaunchAgent, zdalnie przez scheduler w relayu.
5. SSH jest transportem do istniejącego remote mode: OpenSSH ControlMaster +
   bootstrap `dunlin-relay` na hoście. Pełny parity z relayem, bez drugiego
   backendu plików.
6. „Zapisz jako artefakt” to jawne polecenie agenta. Artefakt jest osobnym
   obiektem sesji, nie przypadkowym plikiem w repo.
7. Przy niejasnościach patrzymy na rozwiązania Orca (MIT) i implementujemy
   niezależnie.

## Warstwy

```
apps/desktop
  features/chat, projects, usage, terminal, skills, automations, artifacts
  services/watcher, notifications, permissions, scheduler, pty, ssh
crates/domain       modele współdzielone
crates/storage      SQLite, migracje, FTS5
crates/workspace    fs, git, search, tree
crates/relay        relay API, ACP bridge, scheduler
crates/usage        subscription probes + dokładne skanery providerów
crates/terminal     PTY, VT, scrollback
crates/skills       discovery, walidacja, materializacja
crates/artifacts    model, storage, podgląd
crates/automation   scheduler, runy, precheck
crates/ssh          OpenSSH ControlMaster, bootstrap relay
crates/mcp          lokalny serwer narzędzi dla harnessów
```

`DunlinApp` nie może dalej rosnąć w jednym pliku. `app.rs` zostaje
rozbity na feature-moduły, a wspólne serwisy są przekazywane przez `Services`.

## Model danych

### Projekt i przestrzeń

- `Project` — istniejący workspace (lokalny, relay, SSH).
- `ProjectGroup` — istniejące sekcje.
- `ChatSpace` — czat bez projektu; prywatny scratch workspace w
  `Application Support/Dunlin/chats/<id>`.
- `ProjectSettings` — nadpisania: harness, model, permission mode, env, skills,
  automation defaults.
- `SshHost` — host, user, port, klucz/agent, jump host, relay bootstrap.

### Sesje i usage

- `SessionSummary` — bez zmian, z opcjonalnym `project_id` po wydzieleniu
  `ChatSpace`.
- `UsageEvent` — pojedynczy turn/provider: input, output, cache read/write,
  reasoning, koszt, model, harness, provider session id, projekt.
- `UsageSession`/`UsageDaily` — agregaty do dashboardu.
- Skanery providerów mają własne checkpointy (mtime, size, offset, owner keys),
  żeby nie czytać logów od zera i nie dublować forków.

### Automatyzacje i artefakty

- `Automation` — prompt, cron/RRULE, strefa, projekt, harness, model, precheck,
  tryb workspace, polityka uprawnień, `next_run_at`.
- `AutomationRun` — status, output, usage, diff, błąd.
- `Artifact` — sesja/projekt, typ (`markdown`, `html`, `csv`, `image`, `diff`,
  `screenshot`), treść/ścieżka, wersja, widoczność, link.
- `Skill` — standard `SKILL.md`, źródło, providerzy, hash, enabled per projekt.
- `Toast`/`Notification` — akcje i historia.

## Decyzje techniczne

### Markdown

- Faza 1: istniejący edytor źródła + podgląd Markdown + toolbar.
- Faza 2: edytor blokowy: AST → bloki → edytowalne komponenty GPUI (nagłówki,
  akapity, listy, cytaty, kod, tabele, obrazy), wspólny model undo/redo.
- Surowy tekst zawsze dostępny jednym przełącznikiem.
- Bez HTML/JS w środku. HTML artefakty albo natywny bezpieczny podzbiór, albo
  otwarcie w przeglądarce (do decyzji przy artefaktach).

### Computer use

- `Dunlin Computer Use.app` — osobny bundle id, Swift/AppKit.
- Accessibility API do drzewa, ScreenCaptureKit do zrzutów, CGEvent do inputu.
- Status TCC, otwieranie System Settings, reset uprawnień.
- Main app rozmawia z helperem po lokalnym IPC z tokenem.
- Domyślnie wyłączone; allowlist aplikacji, potwierdzenia akcji destrukcyjnych,
  widoczny wskaźnik pracy i globalny stop.

### SSH

- OpenSSH `ControlMaster`/`ControlPersist`, `ssh -G` do rozwiązywania configu.
- Bootstrap `dunlin-relay` na hoście (upload binarki, uruchomienie, tunel
  lokalny). Dalej działa istniejący remote backend.
- Passphrase w Keychain; known_hosts i host-key verification obowiązkowe.
- Fallback „plain SSH” tylko dla terminala/przeglądania, bez pełnego parity.

### Usage

- ACP `usage_update` = kontekst, nie rozliczenie.
- Dokładne użycie z logów: Claude JSONL, Codex rollout JSONL, OpenCode SQLite.
- Cennik per model, cache i long-context; `unpriced` zamiast zmyślania.
- Skanery lokalne i relayowe; dashboard global/projekt/model/harness.

### Skills

- Standard `SKILL.md` + YAML frontmatter.
- Rooty: `~/.agents/skills`, `~/.claude/skills`, `~/.codex/skills`,
  `.agents/skills`, `.claude/skills`, `.dunlin/skills`.
- Włączanie per projekt przez materializację symlinków do rootów harnessa lub
  listę w konfiguracji.
- Dodawanie z folderu/repo, edycja, walidacja, hash, ostrzeżenia o skryptach.

### Automatyzacje

- Precheck → worktree (opcjonalnie) → sesja ACP → prompt → usage → powiadomienie.
- Domyślnie read-only lub praca na osobnym worktree; bez auto-push.
- Scheduler lokalny w aplikacji, LaunchAgent przy zamkniętej aplikacji, relay
  dla hostów zdalnych.
- Limity czasu, współbieżności i historii runów.

## Bezpieczeństwo

- Terminal i computer use są capabilities, nie domyślnie włączone.
- Skills mogą zawierać instrukcje/ skrypty — pokazujemy źródło i hash.
- Automatyzacje nigdy nie pushują bez jawnej zgody.
- Uprawnienia TCC mają statusy i wyjaśnienia w UI.
- Usage scanners są lokalne; wysyłka statystyk na relay jest opcjonalna.
- Artefakty mają jawną akcję publikacji, wygasanie i revoke.

## Kolejność prac

1. Fundament: rozbicie `app.rs`, modele, migracje, design tokens, event bus.
2. M1 — codzienna praca (kolejka, akcje wiadomości, powiadomienia, watcher,
   brakujący folder, toasty, menu kontekstowe, wąskie okno).
3. M2 — ustawienia per projekt, grupy v2, usage, onboarding, centrum uprawnień.
4. M3 — terminal, komendy, Git v2, search, Markdown, standalone chat, artefakty,
   skills.
5. M4 — SSH, automatyzacje, computer use.
6. M5 — graf/trace, budżety, sharing, accessibility, polish.
