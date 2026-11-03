# Plan rozwoju — aktualizacja 2026-11-01 / beta.3

Zakres: ustawienia, Git, search, skills, Graph/Trace, design/a11y, onboarding,
terminal/Run, Markdown, standalone chat, SSH, computer use i orkiestracja.
Punkt wyjścia: [REMAINING-WORK.md](REMAINING-WORK.md). Ten dokument ustala
kolejność implementacji; nie oznacza dostarczenia całego backlogu.

Nowy zatwierdzony zakres: [Agent workbench i GitHub](agent-workbench-and-github.md).
Obejmuje wszystkie osiem propozycji i pełny cykl CI/PR. GitHub, inspektor,
pamięć decyzji, review/checkpointy, checks, trwałe budżety, porównanie A/B i mapa
Rust AST mają implementacje v1 z testami. Pełny billing, compiler/LSP/non-Rust
impact, ręczna akceptacja oraz pełna wirtualizacja grafów pozostają otwarte; nie oznaczać
całego rozszerzenia jako ukończonego. Aktualna macierz jest w [ROADMAP.md](ROADMAP.md).

Stan źródeł: `0.1.0-beta.5` + dalszy zakres, **531 testów OK / 0 pominiętych**; build, Clippy
`-D warnings`, fmt i izolowane regresje przeszły. Cztery opcjonalne testy
integracyjne `#[ignore]` zostały usunięte. `app.rs` jest podzielony na
`app/overlays.rs` (UI i overlaye), `app/helpers.rs` (funkcje pomocnicze) i
`app/tests.rs`; pozostają wspólne Services i event bus. Remote usage dla roli
`member` pokazuje jawny stan `Admin-only` bez żądania do relay i bez toastu
błędu; administrator nadal pobiera limity. CI odrzuca `todo!()` i
`unimplemented!()` w `apps`/`crates`. Starsze liczby poniżej opisują historyczne
etapy walidacji, nie bieżący zestaw. CI/Release beta.3 zakończone sukcesem;
opublikowane artefakty pobrano i zweryfikowano bez instalacji.
Hardening beta.4 dodaje fail-closed bramki, ale nie zamyka czerwonych audytów
zależności/polityk ani ręcznej QA: [stan odbioru](release-acceptance.md).

## Zasady wspólne

- Małe pionowe zakresy: model domenowy → storage/migracja → backend → UI → testy.
- Bez ukrytego uruchamiania skryptów, płatnych promptów, fetchy, restartów lub
  mutacji Git. Operacje destrukcyjne i rozszerzenie dostępu wymagają zgody.
- Lokalny i relay to jawne capability, nie pozorna zgodność ani fallback na Maca.
- Praca I/O poza wątkiem UI; limity rozmiaru/czasu i anulowanie dla nowych zadań.
- Każdy etap: kompatybilność starych danych, testy regresji, fmt/clippy,
  ręczny scenariusz klawiatury/focusu i jasne ograniczenia w dokumentacji.

## Etapy i kryteria odbioru

### 1. Ustawienia (M) — rozpoczęte

- **1a, zaimplementowane; testy automatyczne OK, QA GUI oczekuje:** lokalny model
  domyślny przypisany do harnessa, env per
  projekt, walidacja, zapis i edycja. Env wyłącznie lokalnego procesu potomnego,
  bez zmiany env aplikacji i bez wysyłania go przez relay. Model wybierany przez
  ACP przed udostępnieniem nowej sesji; wznowione sesje zachowują swój model.
  UI: Project settings → Model & environment…; wartości env nieszyfrowane w bazie,
  nie do sekretów. Model/env relay pozostają oddzielnym zakresem capability.
- **1b, zaimplementowane; testy automatyczne OK, QA GUI oczekuje:** osobne ustawienia grup; resolver global → przodkowie od korzenia →
  grupa → projekt. W UI źródło każdej wartości, reset do dziedziczenia,
  wykrywanie cykli i testy reparentingu/usuwania/Undo. Ikona nie jest dziedziczona.
  Kontekst agenta grupowego nadal używa najostrzejszej polityki członków.
- **1c, lokalna macierz wdrożona; QA GUI oczekuje:** uprawnienia lokalnego klienta od następnego żądania;
  model/tryb przez ACP, jeśli capability pozwala; harness/env przy następnym
  połączeniu. Nigdy automatyczny restart trwającej tury. Jawna akcja zastosowania
  modelu do bieżącej bezczynnej sesji i obsługa odmowy/busy/parked runtime.
  Akcja **Apply saved model to current session** nie zmienia innych/parkowanych
  rozmów, respektuje filtry providera/modelu i nie wysyła promptu. Relay settings
  oraz dalsze ustawienia specyficzne dla adapterów pozostają osobnym zakresem.
- Odbiór 1a: stare projekty bez zmian, env nie wycieka między projektami,
  niepoprawny/niedostępny model nie wysyła promptu z innym modelem.
- **1d, zaimplementowane; QA oczekuje:** grupy agentów mogą jawnie dodać rooty
  ACP (`additionalDirectories`) lokalnie lub na jednym relayu. Relay weryfikuje
  każdy grant przy połączeniu i każdym żądaniu; MCP servers i ścieżki poza grantem
  są odrzucane, a sandbox dopuszcza wyłącznie zatwierdzone rooty.

### 2. Git (L) — po ustaleniu wspólnego kontraktu operacji

- **2a, zaimplementowane; testy automatyczne OK, QA GUI oczekuje:** historia jako zakładka głównego panelu, lista plików i diff commita
  (root/merge/rename/binary/duże pliki); zachowanie obecnego bezpiecznego branch create.
  Zakres i ręczna QA: [Git history](git-history.md). Snapshot jest ograniczony,
  niezależny od Chat/plików; transport read-only relay dostarcza etap 2e.
- **2b, read-only wdrożone; QA GUI oczekuje:** lista stash/worktree, blokady/prunable,
  stan merge/rebase/cherry-pick/revert/sequencer i konflikty indeksu, lokalnie i przez
  relay. Sam odczyt nie mutuje repozytorium; akcje apply/remove/continue/abort
  dostarcza etap 2c. [Zakres i QA](git-repository-state.md).
- **2c, zaimplementowane; ręczna QA oczekuje:** stash push/apply/pop/drop, worktree
  add/remove/prune, merge/rebase/cherry-pick, continue/abort; preflight, potwierdzenia,
  globalna blokada UI i wspólna blokada Git dir, zachowanie buforów. Local i relay
  (jednorazowe plany serwera). Bez force/reset/auto-stash. Szczegóły granic bezpieczeństwa,
  ograniczone przypadki i kontrakt: [operacje Git](git-operations-and-blame.md).
- **2d, zaimplementowane; ręczna QA oczekuje:** inline blame linii kursora w edytorze
  źródła, cache HEAD/plik/backend, hover + przejście do commita, bez przypisywania
  autora zmienionym/niezapisanym liniom. Local/relay, ograniczone odczyty.
- **2e, read-only wdrożone; QA GUI oczekuje:** wersjonowane,
  uwierzytelnione endpointy z grantem projektu, timeoutami i limitami. Testy
  autoryzacji/path traversal oraz parity z lokalnym backendem; bez fallbacku.
  Graf, metadane i patch są zdalne; branch create nadal lokalny. Ograniczenie
  współbieżności 4 odczytów utrzymuje slot do końca rzeczywistej pracy Git.
- Odbiór: operacje mutujące wyłącznie w izolowanych testowych repozytoriach;
  dirty buffers, hook failure, konflikt i worktree innego procesu pokryte testami.
   Historyczna walidacja etapu Git: 450 testów OK, 4 opcjonalne pominięte; build/fmt/Clippy OK,
  natywny smoke Git dark/light-narrow, regresja Markdown i sparowany loopback HTTP
  OK. Ręczne dialogi/hover/a11y i docelowy zdalny host nadal oczekują.

### 3. Global search (M) — równolegle funkcjonalnie z Git, po storage spike

- FTS5 sesji/transkryptów: wersjonowany indeks, backfill poza UI, incremental
  update/delete/import w transakcji. Nie indeksować kluczy/env ani ukrytego reasoning.
- Jeden model zapytania i wyników: tekst, projekt, grupa, harness, daty, typ;
  snippety, ranking, stronicowanie, anulowanie i otwarcie właściwej wiadomości.
- Odbiór: stare bazy, Unicode/znaki FTS, filtry łączone, usuwanie i duże historie;
  błąd indeksu nie niszczy źródłowego transkryptu. Relay jako osobna capability.

### 4. Skills (M/L) — po resolverze ustawień

- Read-only discovery znanych rootów bibliotek; import folderu/repo jako preview
  z listą plików i ograniczeniami, bez wykonywania kodu. Git ref przypięty do SHA.
- Pochodzenie, hash, wersja, ownership i różnica przed aktualizacją; odrzucanie
  symlinków/path traversal; zewnętrzne pliki nigdy nie są nadpisywane/usuwane.
- Włączanie per projekt, jawna instalacja do zgodnego rootu harnessa i cofnięcie.
- Odbiór: konflikty nazw, zmiana zewnętrzna, brak trust i import offline.

### 5. Graph/Trace (M/L)

- Wspólny viewport pan/zoom i stabilne ID, potem deterministyczny layout,
  minimapa, search z reveal i zachowaniem zaznaczenia po aktualizacji.
- Klawiatura: wybór/wchodzenie/wychodzenie z hierarchii, collapse, fit, reset;
  Trace: sticky axis i wirtualizacja wierszy/spans. Eksport niezależny od viewportu.
- Odbiór: fixture 10k elementów, ograniczona praca renderowania, brak utraty focusu
  i stabilny layout podczas streamingu, testy transformacji i hit testingu.

### 6. Design i dostępność (M, ciągłe przez wszystkie etapy)

- Inwentaryzacja widoków → semantyczne tokeny typografii/odstępów → migracja.
  Compact/Comfortable jako ustawienie, bez zmniejszania obszarów trafienia.
- Audyt Tab/Shift-Tab/Esc/Enter, focus trap/restore dialogów, kontrastu,
  etykiet AX i reduce motion; checklisty obu motywów i najmniejszego okna.

### 7. Onboarding i uprawnienia (M)

- Wznawialny kreator: harness → model/auth → projekt/trust → opcjonalny relay/SSH.
  Pominięcie i powrót bez utraty danych; żadnego promptu w trakcie konfiguracji.
- Centrum macOS: Notifications/Accessibility/Screen Recording/Automation/FDA,
  rozróżnienie denied/not determined/unknown, linki i recheck po aktywacji.
  Nie obiecywać automatycznego nadania TCC; nie odczytywać chronionych danych
  wyłącznie w celu sprawdzenia uprawnienia.

### 8. Terminal i Run (L/XL) — lokalny v1 wdrożony

- Wybrano `portable-pty` + `vt100` (bez webowego libghostty). Lokalny PTY, tabs,
  5 000-liniowy scrollback, resize/SIGWINCH, copy/paste, ANSI/RGB i podstawowe
  klawisze są wdrożone. Pozostają search z podświetleniem, zaznaczanie myszą,
  linki, splity, shell integration oraz transport relay/SSH.
- Run: statyczne wykrywanie manifestów bez ewaluacji; preview argv/cwd/env,
  potwierdzenie, PTY, exit/sygnał, historia 50 i rerun są wdrożone.
- Szczegóły i granice: [terminal-and-run.md](terminal-and-run.md).
- Odbiór ręczny: Unicode/IME/bidi, paste wielowierszowy, SIGINT, duży output,
  zamykanie procesów i rzeczywisty host relay/SSH.

### 9. Markdown i standalone chat (M → XL)

- **Markdown wizualny — pierwszy zakres wdrożony; QA GUI oczekuje:** domyślnie
  renderowany dokument z edycją tekstu bloków/formatowania i przełącznikiem Raw.
  Wspólny source/undo/save, bez stratnego round-trip. Stałe pola bez zmiany rozmiaru
  przy kliknięciu; tabele/kod edytowalne, pozostała składnia w polach wewnątrz Visual.
  [Zakres i ograniczenia](markdown-editor.md). Operacje wierszy/kolumn tabel,
  wyrównanie i Tab/Shift+Tab są wdrożone; ręczna QA oczekuje.
  Gotowy jest też podstawowy Enter/Backspace/Delete akapitów/prostych list
  i kompaktowa typografia. Wdrożono też selekcję/edycję między blokami, strzałki
  z obsługą grafemów, zagnieżdżone listy/cytaty i Tab/Shift+Tab poddrzew,
  właściwości linków/obrazów i jawne, ograniczone zasoby projektu local/relay.
  Automatyczny smoke przeszedł w ciemnym i jasnym/wąskim oknie; ręczna QA,
  IME/bidi, złożone przypadki graniczne i rzeczywisty sparowany relay oczekują.
- ChatSpace: niezależne ID/storage/drafty i scratch cwd, bez sztucznego projektu
  w sidebarze. Opcjonalny kontekst repo z pickerem i zgodą, wpis zmiany w historii;
  istniejące sesje projektowe nigdy nie zmieniają po cichu cwd.
- Odbiór: migracje/izolacja historii, dirty source przy zmianie widoku,
  cofnięcie dostępu i wznowienie czatu bez repo.

### 10. SSH (XL) — po terminalowym kontrakcie transportu

- Hosty z konfiguracji OpenSSH, ControlMaster, klucze w Keychain/ssh-agent,
  jawna akceptacja known_hosts i blokada zmienionego host key.
- Potwierdzany bootstrap kompatybilnego relay, tunel ograniczony do loopback,
  zdalne projekty/terminal, reconnect i cleanup procesów. Bez wysyłania lokalnego
  env/tokenów do hosta. Odbiór: offline, MFA, zły klucz i przerwany bootstrap.

### 11. Computer use (XL) — po centrum uprawnień

- Osobny podpisywany helper AppKit/Swift (AX/ScreenCaptureKit/CGEvent),
  wersjonowane IPC i least privilege; allowlista aplikacji, preview i zgoda
  na akcje wrażliwe, ograniczenia czasu/liczby akcji, audit bez sekretów.
- Niezależny globalny awaryjny stop, odwołanie zgody w trakcie działania,
  brak samodzielnego nadawania uprawnień i maskowanie wrażliwych pól.

### 12. Orkiestracja multi-agent (XL, końcowy etap)

- Najpierw trwały message bus/inbox z autoryzacją, dedupe i idempotencją.
  Potem coordinator, DAG i retry/cancel, worktree per zadanie (zależność: Git 2c).
- Handoff → review → human gate → merge; konflikty kierowane do człowieka,
  pełny audit, recovery po restarcie i task board w grafie.
- Model per rola i budżety wymagają wiarygodnego usage/cennika: unknown/unpriced
  nie oznacza zera. Limity kosztu/współbieżności obowiązują także retry i subtaski.
- Odbiór: bezcykliczność, duplikaty wiadomości, crash recovery, cofnięcie trust,
  konflikt merge i brak samoczynnego przekroczenia zatwierdzonego budżetu.

## Najbliższe zadania — rekomendacja domknięcia rozpoczętych zakresów

1. CI/Release beta.3 i artefakty oraz automatyczny hardening już potwierdzone:
   migracja bez wyjątków, reguły solo, skany, podpisany merge i provenance.
   Nie retagować beta.3 ani dopisywać jej nowych wyników walidacji wstecz;
   ewentualne nowe wydanie wymaga osobnej decyzji.
2. Wykonać ręczny odbiór [ustawień/grup/Apply model](project-runtime-settings.md),
   [Git local/relay](git-operations-and-blame.md) i [Markdown GUI/IME/bidi](markdown-editor.md).
   Testy/smoke nie zastępują prawdziwego hosta, dialogów i przeglądu dostępności.
   Użytkownik jawnie odroczył tę macierz tylko dla `0.1.0-beta.N`, bez zaliczenia.
3. Odebrać [workbench/GitHub](agent-workbench-and-github.md) na izolowanych danych
   i realnym koncie: wszystkie osiem v1 jest w kodzie, nie ma już osobnego zadania
   „zaimplementować od zera budżety/porównanie/impact”. Pozostają jawne rozszerzenia.
4. Dokończyć małe luki M1; zakres R12 solo/beta i zdalne dowody są zamknięte.
   Za zgodą użytkownika ustawiono
   PR, osiem app-bound checków i podpisany squash bez bypassów w trybie solo.
   Nie udawać niezależnego code-owner review w jednoosobowym repozytorium.
5. Wybrać następny pionowy zakres (FTS5, billing usage lub skills), a przed
   terminalem wykonać spike VT. Standalone chat, scheduler i M6 są osobnymi etapami;
   istniejące checks i porównanie A/B nie oznaczają ich ukończenia.
