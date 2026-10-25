# Dunlin — roadmapa

Legenda: `S` mały, `M` średni, `L` duży, `XL` osobny subsystem.

## Stan implementacji

Zrealizowane do tej pory:

- kolejka promptów z edycją, usuwaniem, zmianą kolejności i czyszczeniem,
- kopiowanie i ponawianie wiadomości użytkownika,
- powiadomienia macOS przez `UNUserNotificationCenter` i badge w Docku,
- debounced file watching dla lokalnych projektów,
- wykrywanie brakującego folderu, badge i `Locate folder…`,
- toast z `Undo` po usunięciu grupy,
- kompaktowa szerokość sidebara, zwijanie `⌘B` i ukrywanie hintów w wąskim oknie,
- edycja i ponowienie promptu, podgląd unified diff przy tool callu,
- prawy klik na projektach i sesjach,
- inspector jako overlay w wąskim oknie,
- per-project override harnessu i permission mode,
- ustawienie pokazywania/ukrywania hintów klawiaturowych,
- pierwsze tokeny skali w `crates/ui/src/scale.rs`.

Następne w M1: menu kontekstowe dla plików/wiadomości/tool calls, branch od
wiadomości, zawijanie composera, `Undo` dla projektów i sesji, status
uprawnień powiadomień z linkiem do System Settings oraz tokeny typografii we
wszystkich widokach.

## Release 0.1 — public beta gate

Status: `0.1.0-beta.1` opublikowany 25 października 2026 jako unsigned,
ad-hoc signed build dla macOS 15+ Apple Silicon. Kolejne poprawki beta trafiają
na `main`, a nowy tag `v*` uruchamia release workflow.

Zakres 0.1: macOS 15+, Apple Silicon, projekty lokalne i remote relay w wersji
beta. Poza zakresem: iOS, terminal, computer use, automatyzacje, skills,
artefakty i wizualny edytor Markdown.

- [x] **R1. Nazwa i tożsamość** — Dunlin, bundle ID `dev.dunlin.ide`, Keychain
  service `dev.dunlin.remote`, katalog danych `Dunlin`, ikona, copyright.
  Stare dane i tokeny migrują przy pierwszym uruchomieniu.
- [x] **R2. Onboarding i prawdziwe dane** — usunięte demo projekty/sesje i
  compile-time `workspace_root`; ekran pierwszego uruchomienia.
- [x] **R3. Empty states** — brak projektu, brak sesji, brak wyników
  wyszukiwania; harness offline ma status i akcję ponowienia.
- [x] **R4. Okno i aplikacja** — clamp zapisanej pozycji do widocznego ekranu,
  jedna instancja, ostrzeżenie przy zamykaniu w trakcie tury, domyślne menu
  aplikacji macOS.
- [x] **R5. Powiadomienia** — `UNUserNotificationCenter`, autoryzacja przy
  pierwszym powiadomieniu, zachowanie przy odmowie. Status i link do System
  Settings zostają jako polishing w M1.
- [x] **R6. Licencja** — MIT, `LICENSE`, `THIRD_PARTY_NOTICES`, licencja Geist,
  zmiana `license` w Cargo i `Info.plist`.
- [x] **R7. Release engineering** — wersja z taga, arm64 build, DMG/ZIP,
  checksums, podpis/notaryzacja opcjonalna przez sekrety.
- [x] **R8. CI** — fmt, clippy, testy, relay smoke na macOS runnerze.
- [x] **R9. GitHub Release** — workflow `v*`, draft release, źródła + binaria.
- [x] **R10. Dokumentacja** — README ze screenshotami i porównaniem z Orca,
  CHANGELOG, CONTRIBUTING, SECURITY, issue/PR templates.
- [ ] **R11. QA** — świeże konto macOS, brak harnessa, odmowa powiadomień,
  upgrade bazy, duży projekt, remote beta.
- [ ] **R12. Bezpieczeństwo publicznego repo** — dependency review, secret
  scanning, CodeQL/cargo-deny, branch protection, wymagane review dla ścieżek
  agent/network/auth, podpisane commity/release.

Bez konta Apple Developer pierwszy build będzie ad-hoc signed i oznaczony jako
unsigned beta. Po dodaniu konta workflow podpisze i znotaryzuje artefakt.

## Po 0.1 — kolejne etapy

- usage scanners Claude/Codex/OpenCode i dashboard,
- terminal + autowykrywanie komend,
- Git v2 + global search,
- Markdown source/preview → block editor,
- standalone chat,
- artefakty,
- katalog skills,
- automatyzacje,
- SSH hosty,
- computer use + helper TCC,
- graph/trace advanced,
- iOS build i sync.

## M1 — Codzienna praca i zaufanie

- [x] **2. Kolejka promptów** (M)
  - kolejka per sesja, edycja, usuwanie, zmiana kolejności, czyszczenie,
  - panel kolejki w composerze, licznik `N queued`,
  - dispatch tylko po `PromptFinished`, bez gubienia promptów.
- [ ] **3. Akcje wiadomości** (M)
  - [x] kopiuj i ponów dla wiadomości użytkownika,
  - [x] kopiowanie odpowiedzi agenta,
  - [x] edytuj i wyślij,
  - [x] podgląd unified diff przy tool callu,
  - [ ] branch od wiadomości.
- [x] **4. Powiadomienia macOS + badge** (M)
  - permission request i koniec tury,
  - tylko gdy okno nieaktywne, ustawienie on/off,
  - badge w Docku dla oczekujących sesji.
- [x] **5. File watching** (M)
  - FSEvents/`notify`, debounce, ignorowanie `.git`, `node_modules`, `target`,
  - odświeżanie drzewa, gita i oznaczenie otwartego pliku jako stale.
- [x] **6. Brakujący folder projektu** (S)
  - badge w sidebarze, `Locate folder…`, blokada auto-connectu, czytelny błąd.
- [ ] **13. Design tokens i skala UI** (M)
  - [x] pierwsze tokeny w `crates/ui/src/scale.rs`,
  - [ ] tokeny typografii/spacing w istniejących widokach,
  - [ ] gęstość Compact/Comfortable.
- [ ] **15. Toasty z akcją** (S)
  - [x] kolejka toastów i `Undo` dla usunięcia grupy,
  - [ ] `Undo` dla projektów/sesji.
- [ ] **16. Menu kontekstowe** (M)
  - [x] prawy klik na projekcie i sesji,
  - [ ] plik, wiadomość, tool call.
- [ ] **20. Wąskie okno** (M)
  - [x] kompaktowy sidebar i ukrywanie hintów w titlebarze,
  - [x] zwijany sidebar (`⌘B`, przycisk w titlebarze),
  - [x] inspector jako overlay poniżej 1100 px,
  - [ ] zawijanie composera.

## M2 — Projekty, statystyki, onboarding

- [ ] **10. Ustawienia per projekt** (M)
  - [x] override harness i permission mode, modal w menu projektu,
  - [x] precedence global → projekt,
  - [ ] model, env, ustawienia per grupa,
  - [ ] automatyczne zastosowanie bez restartu harnessu.
- [ ] **11. Grupy v2** (M)
  - drag & drop, kolejność, kolory, zagnieżdżanie, sync relay/iOS.
- [ ] **12/28/31. Usage i statystyki** (XL)
  - skanery Claude/Codex/OpenCode, incremental checkpoints, dedupe,
  - cennik z cache i long-context, `unpriced`, cache reuse rate,
  - dashboard global/projekt/model/harness, sesje, tokeny, koszt, czas,
  - oddzielny „context estimate” od „billing usage”.
- [ ] **19. Onboarding** (M)
  - wykrywanie harnessów, model, trust, relay/SSH, pierwszy projekt.
- [ ] **30. Centrum uprawnień macOS** (M)
  - Notifications, Accessibility, Screen Recording, Automation, FDA;
  - statusy, otwieranie System Settings, ponowne sprawdzenie.

## M3a — Warsztat

- [ ] **7. Git v2** (L)
  - branch create/switch, log, stash, konflikty, worktree,
  - **git graph w głównym panelu** (DAG, refs, diff, checkout/merge/rebase/cherry-pick),
  - **inline git blame** na linii kursora z hoverem na commit.
- [ ] **8. Global search** (M)
  - FTS5 transkrypty/sesje + pliki, filtry, snippety.
- [ ] **25. Terminal** (L)
  - PTY, VT, taby/splity, scrollback, search, linki, shell integration;
  - lokalny, SSH, relay; powiadomienie o końcu długiej komendy,
  - spike: `libghostty-vt` vs `alacritty_terminal` (Ghostty jako referencja
    renderowania/VT, bez zależności od webowego libghostty).
- [ ] **26. Autowykrywanie komend** (S/M)
  - package.json, Cargo.toml, Makefile, justfile, pyproject, skrypty;
  - panel „Run”, ostatnie komendy.

## M3b — Treść i rozszerzenia

- [ ] **21. Artefakty** (L)
  - „zapisz jako artefakt”, model i panel, Markdown/HTML/CSV/obraz/diff,
  - preview/source, open in browser, później share przez relay.
- [ ] **24. Katalog skills** (L)
  - discovery standardu, enable per projekt, add/edit/validate, hash,
  - materializacja do rootów harnessa, bezpieczeństwo.
- [ ] **27a. Markdown** (XL)
  - v1 źródło + preview + toolbar,
  - v2 blokowy edytor wizualny + przełącznik raw.
- [ ] **27b. Standalone chat** (M)
  - `ChatSpace`, scratch workspace, bez projektu, lokalnie.

## M4 — Zdalnie i autonomicznie

- [ ] **22. Automatyzacje** (XL)
  - cron/RRULE, precheck, worktree, runy, historia, usage, powiadomienia,
  - lokalny scheduler + LaunchAgent + relay scheduler.
- [ ] **29. SSH hosty** (XL)
  - OpenSSH ControlMaster, Keychain, known_hosts, bootstrap relay, tunel;
  - terminal i projekty zdalne.
- [ ] **23. Computer use** (XL)
  - Swift helper, TCC, AX/ScreenCaptureKit/CGEvent, allowlist, HITL, audit.
- [ ] **30. Pełne uprawnienia** (M)
  - integracja z helperem i onboardingiem.

## M5 — Wizualizacja i polish

- [ ] **17. Graf** (M)
  - pan/zoom, auto-layout, minimap, zwijanie, search.
- [ ] **18. Trace** (M)
  - search, collapse, klawiatura, sticky axis, wirtualizacja.
- [ ] **12. Budżety i alerty** (M)
- [ ] **21. Sharing artefaktów** (M)
- [ ] **20/13. Accessibility i gęstość UI** (M)
- [ ] **iOS** — sync grup/usage/artefaktów, powiadomienia, build z Xcode.

## M6 — Multi-agent orchestration

Cel: agenci nie tylko współistnieją w sesjach, ale przekazują sobie pracę przez
widoczny, kontrolowany graf. To rozwinięcie graph/trace i odpowiedź na potrzebę
orkiestracji znaną z Orca/Augment Intent, ale oparte o ACP i nasze worktree.

- [ ] **M6.1. Message bus** (L) — strukturalne wiadomości między sesjami,
  inbox człowieka, trwałość w SQLite, uprawnienia nadawcy/odbiorcy.
- [ ] **M6.2. Coordinator** (XL) — spec/task → dekompozycja → zależności →
  przydział do harnessów/modeli; każdy subtask w osobnym worktree.
- [ ] **M6.3. Handoff i review** (L) — zakończenie zadania automatycznie
  uruchamia review/merge; konflikty: auto → agent → człowiek.
- [ ] **M6.4. Model routing i budżet** (M) — model per rola, limity kosztu,
  usage attribution per task.
- [ ] **M6.5. Living spec / task board** (M) — spec aktualizowany przez agentów,
  widok zależności i statusów w grafie.
- [ ] **M6.6. Human gates** (M) — zatwierdzanie handoffów, merge i kosztownych
  decyzji; pełny audit trail.

Zależności: terminal, Git worktree, skills, automations, usage i graph.
