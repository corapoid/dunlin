# Dunlin — roadmapa

Legenda: `S` mały, `M` średni, `L` duży, `XL` osobny subsystem.

Statusy: `[x]` = zaimplementowany i automatycznie sprawdzony **opisany zakres**;
nie oznacza ukończenia całego nadrzędnego modułu ani odbioru ręcznego. `[ ]` =
pozostał kod, integracja lub odbiór. „v1” oznacza jawnie ograniczoną implementację.

Aktualny przegląd brakujących zakresów (2026-11-01):
[`REMAINING-WORK.md`](REMAINING-WORK.md). Duże niezaznaczone milestone'y poniżej
mogą mieć gotowe części — nie należy traktować ich jako całkowicie niewdrożonych.

Kolejność wdrożeń, zależności i kryteria odbioru:
[`IMPLEMENTATION-PLAN.md`](IMPLEMENTATION-PLAN.md).

## Stan implementacji

### Przegląd working tree po `0.1.0-beta.5` — 2026-11-03

**Nie wszystkie rozpoczęte elementy są skończone.** Wszystkie osiem obszarów
agent workbench oraz GitHub mają kod v1 i testy, ale pełny zaakceptowany zakres
i ręczna macierz odbioru nie są zamknięte. Nie ma podstaw do oznaczenia całych
M1–M6 jako gotowych ani do wyliczania procentu ukończenia z samych checkboxów.

| Rozpoczęty obszar | Co działa | Co blokuje zamknięcie pełnego obszaru |
| --- | --- | --- |
| Rozmowy / ergonomia M1 | Kolejka, copy/retry/edit, powiadomienia, watcher, brakujący folder, podstawowe menu | Branch od wiadomości, Undo projektu/sesji, menu wiadomości/tool calli, zawijanie composera |
| Ustawienia / grupy M2 | Lokalne modele/env, dziedziczenie i źródła wartości, jawne Apply model, hierarchia/drag & drop, jawne granty multi-root local/relay | Model/env relay dla memberów, dalsze capability adapterów, kolejność/kolory/sync grup i ręczna QA |
| Usage / budżety | Skanery Claude/Codex/OpenCode z checkpointami i dedupe, cennik cache/long-context/`unpriced`, przekroje i alerty w Usage, limity kont, reader OAuth Claude, trwały ledger żądań/czasu/raportowanego USD; remote account usage pokazuje `Admin-only` dla `member` | Attribution pracy wewnętrznej/subagentów, ręczna QA logów i ewentualny sync billing evidence przez relay |
| Git v2 | Lokalna/zdalna historia, checkout commita, zdalny branch create, stash/worktree, operacje i blame z ochroną buforów | Ręczny odbiór i docelowy relay |
| Markdown | Visual/Raw, edycja/struktura/selekcja/zasoby i regresje natywne | Ręczne GUI/IME/bidi, złożone przypadki graniczne i rzeczywisty relay |
| Agent workbench / GitHub | Osiem implementacji v1, lokalne CI/PR i zdalne checkpointy/compare/context/GitHub admin-only | Odbiór na prawdziwym GitHubie i relayu, dalsze capability, pełny billing i compiler/LSP impact; patrz M3c |
| Skills | Własna biblioteka, discovery/import, instalacja per projekt, hash/pochodzenie i bezpieczny eksport/usuwanie | Ręczna QA UI i obsługa kolejnych formatów frontmatter |
| Graph / Trace | Hierarchie, disclosure, filtry/timing, eksport grafu, search z przodkami, minimapa, zoom i skróty, sticky axis | Zaawansowany auto-layout i pełna wirtualizacja 10k+; ręczna QA fokusu/a11y |
| Design / onboarding / uprawnienia | Design settings, tokeny scale, Compact/Comfortable, Reduce motion, jednostajne płaskie Ustawienia, dwustopniowy onboarding, katalog ACP i centrum TCC | Pełna migracja pozostałych literalnych rozmiarów, SSH/trust po otwarciu projektu, ręczna QA klawiatury/a11y/TCC |
| iOS | Źródła klienta SwiftUI/ACP i konfiguracja projektu | Potwierdzony build Xcode, test urządzenia/relay, brakujący sync i powiadomienia; nie jest ukończonym klientem |
| Fundament / release / security | Feature-moduły, migracje, zweryfikowana beta.3, migracja bez wyjątków, reguły solo, zielone skany/main CI i atestacje; `app.rs` podzielony na `app/overlays.rs`, `app/helpers.rs` i `app/tests.rs` | Wspólne Services/event bus, odroczone R11; stabilne wydanie i notaryzacja wymagają osobnego odbioru |

Lokalna walidacja beta.3: **487 testów zaliczonych, 4 opcjonalne pominięte**;
build desktop/relay, Clippy `-D warnings`, fmt, natywne smoke dark/light-narrow
(Git, Markdown, GitHub, workbench) i sparowany loopback HTTP Git przeszły.
Nie zastępuje to manualnej QA ani testu wdrożonego hosta.

Bieżąca walidacja working tree po PR #9 (beta.4) i zakresie beta.5: **527 testów zaliczonych,
0 pominiętych**; build, fmt i Clippy `-D warnings` przechodzą. Cztery testy
`#[ignore]` usunięto, a remote usage dla roli `member` pokazuje jawny stan
`Admin-only` bez żądania do relay i bez błędu. CI odrzuca `todo!()`/
`unimplemented!()` w `apps`/`crates`.

### Zamknięte opisane zakresy bazowe

Kolejka promptów, file watching, obsługa brakującego folderu, bazowe powiadomienia
macOS/badge, import plików sesji, bulk import repozytoriów oraz obecne taby edytora/
podglądy mają implementację i testy. Ich ukończenie nie zamyka odpowiednio całych
rozmów, onboardingu, usage, Git ani pełnego warsztatu.

Zrealizowane funkcje szczegółowe:

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
- import plików sesji per harness z podglądem, dopasowaniem ścieżek i dedupe,
- lokalny graf Git/refs i branch z wybranego commita bez checkoutu,
- custom ikona projektu, lokalnoczasowe powitanie i natywne menu macOS,
- hierarchia grup/List/Tree, bulk import repozytoriów i badge monorepo,
- Design settings, edytor z tabami/minimapą, podgląd obrazów/PDF i scoped diffy,
- globalny katalog skills i instalacja własnych kopii do harnessów,
- ograniczony [agent workbench i GitHub](agent-workbench-and-github.md): CI/PR,
  context/decision memory, review/checkpoints/checks, trwałe budżety, porównanie
  A/B w worktree i inkrementalna mapa Rust AST. Pełny billing, semantyka LSP,
  zdalne nowe narzędzia i ręczna akceptacja pozostają otwarte,
- pierwsze tokeny skali w `crates/ui/src/scale.rs`.
- composer czata inspirowany VS Code: wyśrodkowany start rozmowy, rozwijany
  wybór środowiska (Mac / sparowane relaye), harnessa, trybu ACP/uprawnień i modelu;
  zmiana harnessa tworzy nową sesję, a poprzednia zachowuje swój harness.

Następne w M1: menu kontekstowe dla wiadomości/tool calls, branch od
wiadomości, zawijanie composera, `Undo` dla projektów i sesji, status
uprawnień powiadomień z linkiem do System Settings oraz tokeny typografii we
wszystkich widokach.

## Release 0.1 — public beta gate

Status źródeł: `0.1.0-beta.3`, commit `36398df`, tag wypchnięty 1 listopada 2026.
[CI](https://github.com/corapoid/dunlin/actions/runs/41291827294) i
[Release](https://github.com/corapoid/dunlin/actions/runs/41291827251) zakończyły się
**success**. Prerelease opublikowano `2026-11-01T09:19:50Z`; pobrane ZIP/DMG/relay
mają zgodne SHA-256, wersję/tożsamość/arm64, poprawny podpis ad-hoc i weryfikację DMG.
Nie instalowano artefaktów. Bieżące zmiany hardening są częścią
**0.1.0-beta.5**: nie były częścią tamtych runów i nie zmieniają istniejącego taga.
[Dowody i blokery domknięcia](release-acceptance.md).

Zakres 0.1: macOS 15+, Apple Silicon, projekty lokalne i remote relay w wersji
beta. Beta.3 zawiera także wizualny Markdown, własną bibliotekę skills oraz
ograniczony agent workbench/GitHub. Poza ukończonym zakresem pozostają iOS,
terminal, computer use, scheduler automatyzacji, artefakty i pełna orkiestracja.

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
- [x] **R9. GitHub Release — mechanizm** — workflow `v*`, źródła + binaria;
  aktualny skrypt publikuje release (prerelease dla wersji beta), nie draft.
  Powodzenie mechanizmu nie oznacza potwierdzenia bieżącego runu beta.3.
- [x] **R10. Dokumentacja** — README ze screenshotami i porównaniem z Orca,
  CHANGELOG, CONTRIBUTING, SECURITY, issue/PR templates.
- [ ] **R11. QA** — świeże konto macOS, brak harnessa, odmowa powiadomień,
  upgrade bazy, duży projekt, remote beta. Ręczny GUI/TCC/realny host jawnie
  odroczone decyzją użytkownika wyłącznie dla `0.1.0-beta.N`, nie zaliczone.
- [x] **R12. Bezpieczeństwo publicznego repo — zatwierdzony zakres solo/beta**
  - [x] workflow dependency review i konfiguracja Dependabot,
  - [x] GitHub secret scanning i push protection włączone (odczyt API 2026-11-01),
  - [x] reguły `main`: PR, osiem ścisłych checków przypiętych do aplikacji Actions,
    podpisane squash merge, brak bypassów, zakaz usuwania/non-fast-forward;
    jawnie zatwierdzony tryb solo bez fikcyjnego review drugiej osoby,
  - [x] kod workflow CodeQL (Rust/web/Python/Actions), cargo-deny, walidatora artefaktów,
    raportów QA i audytu reguł; potwierdzone runy PR/main i walidacja tokena workflow,
  - [x] migracja pięciu obsolete pakietów z całego lockfile, dziewięć utrzymywanych
    forków i integralność/provenance; cargo-deny bez wyjątków, 491 testów Rust,
    25 Python, audyt reguł solo i dziewięć natywnych smoke zaliczone lokalnie,
  - [x] świeże skany zmigrowanych/poprawionych źródeł, podpisany squash `f61251f`,
    main CI/Security i atestacje obu binariów; zero otwartych alertów security.
    Sześć false positive udokumentowano indywidualnie, bez wyłączania reguł
    ani wykluczania źródeł Rust. Reguły merge: pass, bez bypassu.
    [Pełna macierz i odroczenia](release-acceptance.md).

Bez konta Apple Developer pierwszy build będzie ad-hoc signed i oznaczony jako
unsigned beta. Po dodaniu konta workflow podpisze i znotaryzuje artefakt.

## Pozostałe kierunki po beta.3

- dokładne usage scanners/cennik i pełne przekroje (podstawowy Dashboard już działa),
- terminal + autowykrywanie komend,
- brakujące operacje/QA Git v2 + global search FTS5,
- odbiór i przypadki brzegowe istniejącego Visual/Raw Markdown,
- standalone chat,
- artefakty,
- discovery/import/per-project skills (własny katalog i eksport już działają),
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
  - [x] plik/folder (tworzenie, rename/delete, Reveal in Finder),
  - [ ] wiadomość, tool call.
- [ ] **20. Wąskie okno** (M)
  - [x] kompaktowy sidebar i ukrywanie hintów w titlebarze,
  - [x] zwijany sidebar (`⌘B`, przycisk w titlebarze),
  - [x] inspector jako overlay poniżej 1100 px,
  - [ ] zawijanie composera.

## M2 — Projekty, statystyki, onboarding

- [ ] **10. Ustawienia per projekt** (M)
  - [x] override harness i permission mode, modal w menu projektu,
  - [x] precedence global → projekt,
  - [x] własna ikona projektu (embedded image, powrót do automatycznej),
  - [x] lokalne domyślne modele per projekt/harness i env lokalnego procesu,
  - [x] ustawienia per grupa, dziedziczenie global → przodkowie → projekt i źródła wartości,
  - [ ] domyślne model/env przez relay,
  - [ ] pozostałe ustawienia stosowane automatycznie bez restartu harnessu;
    wybór harnessa z composera już tworzy nową rozmowę, a domyślne ustawienia
    globalne/projektowe dotyczą nowych sesji.
- [ ] **11. Grupy v2** (M)
  - [x] drag & drop przypisań/reparenting, zagnieżdżanie, List/Tree i sortowanie,
  - [ ] ręczna kolejność, wybór kolorów, sync relay/iOS.
- [ ] **12/28/31. Usage i statystyki** (XL)
  - [x] recorded-data Dashboard i scope-aware limity kont (nie pełny billing),
  - skanery Claude/Codex/OpenCode, incremental checkpoints, dedupe,
  - cennik z cache i long-context, `unpriced`, cache reuse rate,
  - dashboard global/projekt/model/harness, sesje, tokeny, koszt, czas,
  - oddzielny „context estimate” od „billing usage”.
- [ ] **19. Onboarding** (M)
   - [x] podstawowy pierwszy ekran, wykrywanie adapterów, projekt i trust,
   - [ ] pełny wznawialny kreator model/auth/relay/SSH i odbiór.
- [ ] **30. Centrum uprawnień macOS** (M)
  - Notifications, Accessibility, Screen Recording, Automation, FDA;
  - statusy, otwieranie System Settings, ponowne sprawdzenie.

## M3a — Warsztat

- [ ] **7. Git v2** (L)
  - [x] bezpieczny branch switch oraz lokalny log/DAG/refs i branch create z commita,
  - [x] read-only stash/worktree oraz stan operacji i konfliktów, lokalnie/przez relay (QA GUI oczekuje),
  - [x] mutacje stash/worktree, merge/rebase/cherry-pick i continue/abort z preflightem/potwierdzeniami, local/relay (ręczna QA oczekuje),
  - [x] **git graph w głównym panelu**, szczegóły/lista plików i diff commita
    (root, jawny wybór rodzica merge, rename/delete/binary; QA GUI oczekuje),
  - [x] read-only historia przez relay: graf, szczegóły, pliki i patche z grantami projektu,
  - [x] wskazanie commita w grafie → formularz merge/rebase/cherry-pick i potwierdzenie,
  - [x] inline git blame na linii kursora z hoverem i przejściem do commita, cache HEAD/plik/backend oraz brak autorstwa niezapisanych linii,
  - [ ] checkout commita bezpośrednio z grafu i zdalny branch create,
  - [ ] ręczny odbiór mutacji, hoverów i rzeczywistego relay — [zakres i ograniczenia](git-operations-and-blame.md).
- [ ] **8. Global search** (M)
  - FTS5 transkrypty/sesje + pliki, filtry, snippety.
- [x] **25. Terminal — lokalny v1** (L)
  - PTY (`portable-pty`) + VT (`vt100`), taby, bounded scrollback, resize,
    copy/paste, podstawowe klawisze i ANSI/RGB są wdrożone;
  - pozostają search/linki/splity/shell integration oraz relaya/SSH i ręczna QA;
  - szczegóły: [terminal-and-run.md](terminal-and-run.md).
- [x] **26. Autowykrywanie komend — v1** (S/M)
  - package.json, Cargo.toml, Makefile, justfile, pyproject, go, deno, composer,
    Rakefile, mix, gradle/maven i scripts/*;
  - panel „Run”, preview/potwierdzenie, PTY, status/exit, historia i rerun.

## M3b — Treść i rozszerzenia

- [ ] **21. Artefakty** (L)
  - „zapisz jako artefakt”, model i panel, Markdown/HTML/CSV/obraz/diff,
  - preview/source, open in browser, później share przez relay.
- [ ] **24. Katalog skills** (L)
  - [x] globalna biblioteka, add/edit/validate, bezpieczna instalacja i usuwanie
    własnych kopii do rootów harnessa,
  - [ ] discovery zewnętrznych bibliotek, enable per projekt, import folder/repo,
    pełny audyt hash/pochodzenia.
- [ ] **27a. Markdown** (XL)
  - [x] domyślny Visual + Raw, stała typografia przy kliknięciu, edytowalne komórki tabel/kod/proza i pola składni pozostałych bloków (QA GUI oczekuje),
  - [x] operacje wierszy/kolumn tabel, wyrównanie i Tab/Shift+Tab między komórkami (QA GUI oczekuje),
  - [x] kompaktowa typografia, Enter dla akapitów/prostych list oraz Backspace/Delete na zgodnych granicach (QA GUI oczekuje),
  - [x] selekcja/edycja między blokami, strzałki i grafemy, zagnieżdżone listy/cytaty i wcięcia poddrzew (z walidacją i ograniczeniami),
  - [x] właściwości linków/obrazów, jawny podgląd obrazów projektu local/relay i potwierdzane linki HTTP(S),
  - [x] automatyczne QA: unit/transport/security oraz natywny smoke ciemny + jasny/wąski,
  - [ ] ręczny odbiór GUI/IME/bidi i rzeczywistego sparowanego relay; dalsze złożone przypadki strukturalne — [zakres](markdown-editor.md).
- [ ] **27b. Standalone chat** (M)
  - `ChatSpace`, scratch workspace, zwykła rozmowa bez przypisanego projektu,
  - główny composer z wyborem środowiska, harnessa, modelu i trybu pracy;
    wspólny wzorzec UI z czatem projektowym,
  - **picker kontekstu rozmowy „Chat / repozytorium”** jak w VS Code:
    wyszukiwanie po nazwie i ścieżce, lista skonfigurowanych/ostatnich lokalnych
    repozytoriów, `Open Folder…` i powrót do rozmowy bez repozytorium;
    bez automatycznego skanowania całego dysku,
  - repozytorium dołączane jako jawny kontekst zwykłego czata; zmiana kontekstu
    widoczna w historii, potwierdzenie dostępu narzędzi do nowego folderu,
  - ten picker jest przeznaczony **tylko dla standalone chat**. Sesje projektowe
    pozostają przypisane do swojego projektu — bez cichej podmiany `cwd`,
  - izolowana historia i drafty zwykłych rozmów, niezależne od sesji projektów.

## M3c — Agent workbench i GitHub — v1 w kodzie, pełny odbiór otwarty

Wszystkie poniższe pozycje mają implementację oraz automatyczne testy. Nie są
mockupami, ale nie oznaczają też pełnej realizacji pierwotnych propozycji.
Wspólny brak odbioru: ręczne dialogi/focus/klawiatura/a11y; dla GitHuba także
rzeczywiste konto/CI/PR. Dokładne limity i potwierdzone scenariusze:
[agent workbench i GitHub](agent-workbench-and-github.md).

| Obszar | Dostarczony zakres v1 | Granice v1 / niezakończone rozszerzenia |
| --- | --- | --- |
| Review zadania | Baseline, obserwowane diffy, komentarz/evidence → draft | Brak automatycznego autorstwa per tura i dowodu pokrycia testami |
| Context inspector | Rzeczywista koperta klienta, provenance, MCP/skills | System/history/context należące do harnessa pozostają jawnie nieznane |
| Checkpointy | Nazwane snapshoty zapisanych plików, selektywny restore z potwierdzeniem | Nie snapshot RAM/index/refs; zdalne checkpointy/restore niewdrożone |
| Checks / problems | Potwierdzany JSON argv, wynik/exit/logi, diagnostyka Cargo; zdalne uruchamianie dla każdego z grantem | Dalsze adaptery testów i sandbox/limity per użytkownik |
| Budżety zadania | Trwała rezerwacja żądań, czas, raportowane USD, stop/zgody/retencja | Nie limit rachunku providera; wewnętrzne retry/subagenci mogą mieć nieznany koszt |
| Porównanie A/B | Dwa detached worktree z jednego HEAD, identyczny prompt, wspólny budżet, diff/checks/preferencja | Local-only, bez sandboxa i auto-merge; nie zastępuje koordynatora M6 |
| Impact map | Inkrementalny Rust AST, zależności leksykalne, kandydaci testów i zdalny indeks przez relay | Nie compiler/LSP/type/cfg-complete; non-Rust pozostaje poza zakresem |
| Decision memory | Jawne lokalne wersje, stale, evidence i wybrany wpis → draft | Brak sync; nie automatyczne wstrzykiwanie instrukcji ani magazyn sekretów |
| GitHub CI/PR | GitHub.com + `gh`, polling job/step, logi zakończonych jobów, potwierdzane operacje PR/CI | Brak relay/Enterprise, fork-head creation, reviewer/assignee/labels, merge queues i pełnego live log stream |

„Nieznane” koszty/context/coverage są celowymi granicami bezpieczeństwa, nie
wynikiem równym zero ani obietnicą pełnej funkcji. Wyłączenie auto-merge,
auto-send i cichego restartu jest zamierzonym zachowaniem, nie zaległym zadaniem.

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
  - [x] hierarchia agentów, zwijanie/rozwijanie i eksport SVG,
  - [ ] pan/zoom, zaawansowany auto-layout, minimap, search.
- [ ] **18. Trace** (M)
  - [x] hierarchia, collapse/expand i filtry typów,
  - [ ] search, rozszerzona klawiatura, sticky axis, wirtualizacja.
- [ ] **12. Budżety i alerty** (M)
  - [x] trwałe limity żądań/czasu/raportowanego USD per zadanie (także w tle),
  - [ ] pełne rozliczanie providera/subagentów i alerty kont/globalne.
- [ ] **21. Sharing artefaktów** (M)
- [ ] **20/13. Accessibility i gęstość UI** (M)
- [ ] **iOS** — sync grup/usage/artefaktów, powiadomienia, build z Xcode.
  Źródła SwiftUI/ACP już istnieją, ale build i działanie na urządzeniu nie są
  potwierdzone; to rozpoczęty, nieodebrany obszar, a nie gotowa aplikacja.

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
  Osobne porównanie A/B ma już wybór modeli, wspólny ledger i potwierdzenia;
  nie oznacza wdrożenia routingu ról ani gwarantowanego limitu rachunku.
- [ ] **M6.5. Living spec / task board** (M) — spec aktualizowany przez agentów,
  widok zależności i statusów w grafie.
- [ ] **M6.6. Human gates** (M) — zatwierdzanie handoffów, merge i kosztownych
  decyzji; pełny audit trail.

Zależności: terminal, Git worktree, skills, automations, usage i graph.

## Rekomendowana kolejność domknięcia

1. Beta.3 i automatyczny hardening są potwierdzone: migracja bez wyjątków,
   reguły solo, skany, podpisany merge i mainline provenance. Nowe wydanie wymaga
   osobnej decyzji; nie retagować beta.3 ani udawać odroczonego odbioru R11.
2. Odebrać rozpoczęte zakresy: ustawienia/grupy, Git, Markdown i workbench/GitHub
   na izolowanych danych; osobno realny relay, GitHub i GUI/IME/a11y.
3. Domknąć małe braki M1 (branch rozmowy, Undo, menu wiadomości/tool calli,
   composer) oraz wskazane luki R12, bez automatycznej zmiany zasad repo.
4. Potem wybrać następny zakres funkcjonalny: FTS5, billing usage albo skills
   discovery/per-project. Nie rozpoczynać jednocześnie wszystkich dużych modułów.
5. Terminal/Run, standalone chat, artefakty, scheduler, SSH, computer use i M6
   pozostają osobnymi etapami; bazowe checks, relay i A/B nie realizują ich za nas.
