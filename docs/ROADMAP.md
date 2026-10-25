# Kestrel — roadmapa

Legenda: `S` mały, `M` średni, `L` duży, `XL` osobny subsystem.

## Stan implementacji

Zrealizowane w pierwszym kroku M1:

- kolejka promptów z edycją, usuwaniem, zmianą kolejności i czyszczeniem,
- kopiowanie i ponawianie wiadomości użytkownika,
- powiadomienia macOS (best-effort) i badge w Docku,
- debounced file watching dla lokalnych projektów,
- wykrywanie brakującego folderu, badge i `Locate folder…`,
- toast z `Undo` po usunięciu grupy,
- kompaktowa szerokość sidebara, zwijanie `⌘B` i ukrywanie hintów w wąskim oknie,
- edycja i ponowienie promptu, podgląd unified diff przy tool callu,
- prawy klik na projektach i sesjach,
- inspector jako overlay w wąskim oknie,
- per-project override harnessu i permission mode,
- pierwsze tokeny skali w `crates/ui/src/scale.rs`.

Następne w M1: menu kontekstowe, pełne zwijanie sidebara, tokeny typografii w
wszystkich widokach, pełne uprawnienia powiadomień i akcje na odpowiedziach
agenta.

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
  - branch create/switch, log, stash, konflikty, worktree.
- [ ] **8. Global search** (M)
  - FTS5 transkrypty/sesje + pliki, filtry, snippety.
- [ ] **25. Terminal** (L)
  - PTY, VT, taby/splity, scrollback, search, linki, shell integration;
  - lokalny, SSH, relay; powiadomienie o końcu długiej komendy.
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
