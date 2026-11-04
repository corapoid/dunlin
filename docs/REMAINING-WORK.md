# Pozostałe prace — przegląd 2026-11-03

Źródła: `ROADMAP.md`, `PLAN.md`, obie listy `TODO-*` i aktualny kod.
To lista brakujących zakresów, a nie kopia nieaktualnych większych checkboxów.

## Odpowiedź na pytanie o rozpoczęte prace

**Nie — nie wszystkie są skończone.** Po beta.3 wszystkie osiem obszarów workbench
i GitHub mają ograniczony kod v1 z testami, lecz nadal brakuje odbioru ręcznego
oraz części pierwotnego zakresu. Git/Markdown/ustawienia są zaimplementowane
w opisanych granicach, a nie w pełni odebrane; usage, skills, Graph/Trace,
ergonomia, design/onboarding i klient iOS mają również braki funkcjonalne.
Szczegółowa macierz „działa / pozostaje” jest na początku [roadmapy](ROADMAP.md).

Zamknięte opisane zakresy bazowe: kolejka, watcher, brakujący folder, podstawowe
powiadomienia/badge, import sesji i bulk import repozytoriów. Ich zakończenie
nie zamyka wszystkich rozszerzeń czata, projektów czy całych milestone'ów.

Walidacja źródeł beta.3: 487 testów OK / 4 opcjonalne pominięte, build/fmt/Clippy,
natywne regresje i loopback HTTP OK. CI/Release beta.3 zakończone sukcesem,
prerelease opublikowane; pobrane artefakty zweryfikowane bez instalacji.
Bieżący hardening nie jest częścią beta.3: migracja pięciu obsolete pakietów
bez wyjątków oraz audyt zatwierdzonych reguł solo przechodzą. 491 testów Rust
i 25 Python zaliczone; zielone PR/main CI/Security, podpisany merge `f61251f`
bez bypassu i zweryfikowane atestacje obu binariów.
**Bieżąca walidacja working tree po PR #9, zakresie beta.6 i nowym v1:
592 testów OK / 0 pominiętych**, build, fmt i Clippy `-D warnings` przechodzą.
README jest krótkim landing page'em, a pełny opis przeniesiono do
[`guide.md`](guide.md); lokalny baseline wydajności jest w
[`performance.md`](performance.md).
Ustawienia mają jedną płaską powierzchnię, dwustopniowy onboarding i katalog
ACP; ich ręczna QA pozostaje otwarta. Usunięto cztery
testy `#[ignore]`; `app.rs` dzielą teraz `app/overlays.rs`, `app/helpers.rs` i
`app/tests.rs`. Remote usage dla `member` jest jawnie `Admin-only` bez żądania
i bez błędu; admin nadal pobiera limity. CI odrzuca `todo!()`/
`unimplemented!()`. Manualna QA jawnie odroczona dla serii beta, nie zaliczona.
[Dowody i granice](release-acceptance.md).

## Częściowo wdrożone — czego jeszcze brakuje

1. **Rozmowy i ergonomia**: odgałęzianie od wiadomości (nie to samo co branch Git),
   Undo usuwania projektów/sesji, menu kontekstowe wiadomości i tool calli,
   dopracowanie zawijania composera w wąskim oknie.
2. **Design i dostępność**: tokeny semantyczne, Compact/Comfortable i Reduce
   Motion są wdrożone (najpierw Settings i nowe powierzchnie). Pozostają migracja
   literalnych rozmiarów w starszych widokach oraz kompleksowy ręczny audyt
   klawiatury, focusu, kontrastu i etykiet AX.
3. **Ustawienia projektu/grupy**: lokalne model/env per projekt i ustawienia grup z
   resolverem global → przodkowie → projekt, źródłami wartości i resetem
   dziedziczenia są wdrożone. Remote project settings są przechowywane na relay i
   administracyjnie edytowalne; zwalidowane env trafia do następnego procesu
   harnessa, a zapisany model jest aplikowany po pojawieniu się modeli ACP.
   Member widzi wartości, ale nie może ich zapisać. Group agents obsługują teraz
   jawne granty multi-root: lokalne katalogi przez ACP `additionalDirectories`
   oraz projekty relay na jednym hoście przez listę `roots` w połączeniu ACP.
   Relay weryfikuje każdy grant przy połączeniu i każdym żądaniu, a sandbox
   dopuszcza wyłącznie zatwierdzone rooty. Ręczna QA nowych dialogów nadal oczekuje.
4. **Grupy v2**: ręczna kolejność, wybór kolorów i sync relay/iOS. Hierarchia,
   List/Tree, sortowanie, drag & drop przypisań/przenoszenia grup oraz bezpieczne
   usuwanie/promowanie/Undo już działają.
5. **Usage**: dokładne skanery logów Claude/Codex/OpenCode z checkpointami i dedupe,
   rozliczeniowy cennik cache/long-context/`unpriced`, cache reuse rate, pełne
   przekroje global/projekt/model/harness, budżety i alerty. Są już recorded-data
   Dashboard, aktualizacje ACP i scope-aware limity kont; nie zastępują one
   pełnego billing usage. Trwałe limity żądań/czasu/raportowanego USD per zadanie
   są wdrożone; nie są gwarantowanym limitem rachunku ani pełnym rozliczeniem
   wewnętrznych retry/subagentów. Reader OAuth usage Claude jest włączony
   (odczyt limitów 5h/tygodniowych przez `api.anthropic.com/api/oauth/usage`).
   Fundament billingowy (`dunlin_usage::billing`) skanuje logi Claude Code JSONL,
   Codex rollout JSONL i OpenCode SQLite, deduplikuje po message id, checkpointuje
   pełne linie JSONL, liczy cache read/write i cennik znanych modeli (w tym
   long-context); nieznane modele pozostają `unpriced`. Dashboard Usage pokazuje
   przekroje global/provider/harness/model/projekt/dzień, a budżety alertów
   ($50/$100/$250) ostrzegają na 80% i 100% znanego kosztu.
   Remote usage jest admin-only po stronie relay: klient pokazuje `Admin-only`
   dla roli `member` bez wysyłania żądania i bez toastu błędu, a administrator
   nadal pobiera limity. To świadomy stan uprawnień, nie błąd połączenia.
   Attribution v1 (`Main/Subagent/Internal/Unknown`), źródła do ręcznej QA,
   kopiowalny raport i opcjonalny sync agregatów przez relay są wdrożone;
   pozostają ręczna QA prawdziwych logów, pełne rozliczenie providera i
   ewentualne dalsze adaptery. [Zakres](standalone-chat-worktree-usage.md).
6. **Onboarding i uprawnienia**: dwustopniowy kreator harness → projekt ma
   teraz opcjonalną kartę powiadomień (Allow/Open Settings), centrum statusów
   Notifications/Accessibility/Screen Recording/Automation/FDA, linki do System
   Settings, ponowne sprawdzanie oraz jawny stan **Not granted**. Chat pokazuje
   banner zaufania projektu. Pozostają ręczna QA TCC i pełny wizard relay/SSH.
7. **Git v2**: checkout commita z grafu i zdalny branch create są wdrożone;
     pozostaje ręczna QA mutacji/hoverów i rzeczywistego relay. Mutacje stash/worktree, merge/rebase/cherry-pick,
     continue/abort i inline blame są wdrożone local/relay z preflightem,
     jednorazowymi planami i ochroną buforów — [zakres](git-operations-and-blame.md).
     Bezpieczny switch, lokalna historia DAG/refs,
    tworzenie brancha z zaznaczonego commita oraz zakładka grafu w głównym panelu
    z listą plików/diffem commita są gotowe w kodzie. Porównania root/merge-parent,
    rename/delete/binary mają testy; ręczna QA nowego panelu nadal oczekuje.
    Read-only historia przez relay (graf/szczegóły/patche) jest wdrożona z grantami
     i przypięciem zakresu; zdalne checks działają przez `POST /v1/checks/run`
     (jawny argv, 120 s, 512 KiB, bez sandboxa, dla każdego z grantem), a impact
     przez `/v1/impact/build` i `/v1/impact/verify`. Zdalne checkpointy/restore,
     compare z jawnymi projektami worktree, GitHub (admin-only, konto `gh` hosta)
     i context inspector mają endpointy `/v1/review/*`, `/v1/compare/*`,
     `/v1/github/*` i `/v1/context`. Pozostają mutacje/blame poza istniejącym
     zakresem oraz ręczna QA na rzeczywistym relayu.
    Podglądy read-only stash/worktree (w tym blokady/prunable), stanu operacji
    i konfliktów indeksu są wdrożone lokalnie i przez relay; ich GUI nadal wymaga QA.
8. **Global search**: FTS5 transkryptów i sesji, wspólne filtry i snippety.
   Aktualne wyszukiwanie plików w projekcie nie realizuje całego zakresu.
9. **Skills**: discovery istniejących bibliotek, import z folderu, instalacja
   per projekt i audyt hash/pochodzenia są wdrożone. Globalna biblioteka,
   edycja/walidacja i bezpieczna instalacja/usuwanie własnych kopii do rootów
   harnessów również działają. Dodano paczki wieloplikowe (współdzielone pliki,
   hash per plik, aktualizacje i usuwanie tylko własnych niezmienionych kopii)
   oraz katalog Cloudflare Security Audit w Settings → Skills i sekcji Security;
   Codex Security pozostaje zewnętrznym pluginem wymagającym harnessu Codex.
   `dunlin-audit` czyta i waliduje artefakty Cloudflare/Codex/SARIF, pokazuje
   findings, coverage, raport i eksport SARIF w Security, a Settings → Security
   pozwala globalnie ukryć workspace bez usuwania plików. Triage lokalny,
   licznik wystąpień fingerprintu, eksport SARIF/JSON/CSV/Markdown, świeży
   verifier, fazy runu, opcjonalny Codex CLI scan/patch/verify-fix, lokalny
   sandbox seatbelt dla jawnych checks oraz zdalny start i odczyt artefaktów
   audytu przez relay oraz adaptery Semgrep/Snyk/Trivy/Gitleaks/Bandit są
   wdrożone. Pozostaje ręczna QA UI, zdalne uruchamianie Codex CLI na hoście
   relay i kolejne formaty skanerów.
10. **Agent graph / Trace**: pan/zoom, minimapa, wyszukiwanie z zachowaniem
    przodków, skróty `⌘+`/`⌘−`/`⌘0`, sticky axis, zoom gęstości wierszy,
    tidy-tree bez limitu głębokości, wirtualizacja viewportu grafu/minimapy,
    wirtualne wiersze trace, drzewo ARIA i fokus ze scrollowaniem do wyboru są
    wdrożone. Automatyczne fixtures 10k+ i macierz manualna opisuje
    [graph-trace-virtualization.md](graph-trace-virtualization.md); pozostaje
    ręczna QA VoiceOver/klawiatury w obu motywach i wąskim oknie.

## Dalszy backlog — nowe podsystemy i już rozpoczęte rozszerzenia

Poniższa lista **nie** oznacza, że każdy moduł jest niewdrożony. Markdown,
workbench, źródła iOS oraz lokalny terminal/Run v1 już istnieją; FTS5, scheduler
i koordynator nie wynikają automatycznie z istniejących checks/search plików/A/B.

11. **Terminal**: lokalne PTY (`portable-pty`) + VT (`vt100`), taby, bounded
    scrollback, resize, copy/paste, podstawowe klawisze, wyszukiwanie widocznego
    buforu z podświetleniem, klikalne linki HTTP(S)/mailto/OSC 8, zaznaczanie
    myszą (słowo/linia/Shift), splity z przeciąganym dividerem oraz shell
    integration OSC 133/633/7/1337 są wdrożone. Transport relay przez
    uwierzytelniony WebSocket i SSH v1 przez OpenSSH również działają. Pozostają
    ręczna QA GUI/IME/bidi/splitów oraz pełny bootstrap/tunele SSH i Keychain.
    Szczegóły: [terminal i Run](terminal-and-run.md).
12. **Run**: statyczne wykrywanie package.json/Cargo/Make/just/pyproject/go/
    deno/composer/Rakefile/mix/gradle/maven oraz scripts/*; preview argv/cwd/env,
    potwierdzenie, PTY, exit code/sygnał, historia i rerun są wdrożone. Pozostają
    ręczna QA, kolejne formaty manifestów i zdalny Run na relayu.
13. **Artefakty**: model/storage/panel, jawne „zapisz jako artefakt”, preview/source
    dla Markdown/HTML/CSV/obrazów/diffów; później share, wygasanie i revoke.
14. **Markdown**: domyślny Visual, stałe pola edycji bez zmiany typografii przy focus,
    edytowalne komórki tabel/kod/proza i pola składni pozostałych bloków + Raw są
    wdrożone, podobnie operacje wierszy/kolumn tabel, wyrównanie i Tab/Shift+Tab;
    podstawowe Enter/Backspace/Delete prozy/list i kompaktowa typografia też gotowe.
    Selekcja/edycja między blokami, strzałki/grafemy, zagnieżdżone listy/cytaty,
    Tab/Shift+Tab poddrzew, właściwości linków/obrazów i jawne zasoby local/relay
    również wdrożone. Smoke przeszedł w ciemnym i jasnym/wąskim oknie.
    Pozostają ręczna QA GUI/IME/bidi, rzeczywisty sparowany relay i złożone
    przypadki graniczne; [dokładny zakres i ograniczenia](markdown-editor.md).
15. **Standalone chat**: ChatSpace/scratch workspace, izolowane historie,
    jawny picker Chat/repo i zatwierdzany kontekst repozytorium są wdrożone jako
    v1. Pozostają ręczna QA GUI, zmiana kontekstu bez tworzenia nowej przestrzeni
    oraz ewentualny wspólny picker ostatnich repozytoriów.
    [Zakres](standalone-chat-worktree-usage.md).
16. **Automatyzacje**: cron/RRULE, precheck, worktree, runy/historia/usage/limity,
    LaunchAgent i scheduler relay, praca przy zamkniętej aplikacji.
17. **SSH**: terminal SSH v1 (OpenSSH PTY, `ControlMaster`, ssh-agent,
    known_hosts, jawny target/ścieżka) jest wdrożony. Pozostają Keychain,
    bootstrap relay, tunel loopback, reconnect i cleanup procesów; obecny relay
    nie oznacza pełnego SSH.
18. **Computer use**: osobny Swift/AppKit helper, TCC, AX/ScreenCaptureKit/CGEvent,
    allowlist, zatwierdzenia człowieka, globalny stop i audit trail.
19. **iOS — rozpoczęty, nieodebrany**: źródła SwiftUI/ACP i konfiguracja projektu
    istnieją; build Xcode i działanie na urządzeniu/relay nie są potwierdzone.
    Pozostają również sync grup/usage/artefaktów i powiadomienia.
20. **Orkiestracja multi-agent**: trwały message bus/inbox, coordinator i DAG zadań,
    handoff/review/merge, routing modeli, budżety per rola, living spec/task board
    i human gates. Worktree per subtask ma wdrożony v1: jawny branch i worktree z
    commita plus trwały rejestr, bez auto-startu agenta/merge/push.
    [Zakres](standalone-chat-worktree-usage.md).
     Osobne porównanie A/B już tworzy dwa worktree z jednego commita, uruchamia
     identyczny zatwierdzony prompt ze wspólnym budżetem i zapisuje preferencję
     bez merge. Nie zastępuje koordynatora/handoffów.
21. **Agent workbench/GitHub**: osiem obszarów ma ograniczone implementacje;
    pozostają ręczna QA dialogów/klawiatury/a11y, prawdziwy GitHub i wdrożony relay,
    pełny billing, semantyka compiler/LSP/non-Rust oraz ręczna QA zdalnych checkpointów/comparison/GitHub/context; checks, impact i wspomniane endpointy są już dostępne.
    Mapa wpływu działa jako Rust AST/lexical references i kandydaci testów, nie
    dowód pokrycia. [Dokładny zakres i walidacja](agent-workbench-and-github.md).

## Fundament i wydanie — oddzielnie od funkcji

- `app.rs` jest podzielony na `app/overlays.rs`, `app/helpers.rs` i
  `app/tests.rs`; pozostają wspólne Services, modele/migracje i event bus dla
  nowych subsystemów z `PLAN.md`. `app.rs` nadal zawiera stan i logikę aplikacji.
- Brak `todo!()`/`unimplemented!()` w źródłach; niedokończone ścieżki relay
  kończą się jawnym błędem. CI egzekwuje tę regułę dla `apps`/`crates`.
- **R11 QA**: świeże konto macOS, brak harnessa, odmowa powiadomień, upgrade bazy,
  duży projekt i remote beta; pełna weryfikacja GUI pozostaje do wykonania.
  Użytkownik zaakceptował odroczenie GUI/TCC/realnego hosta tylko dla
  `0.1.0-beta.N`. [Manualna macierz R11](r11-qa.md); Settings → Advanced →
  Release QA generuje kopiowalny raport środowiska, a `--onboarding-smoke`
  pokrywa świeży katalog bez projektu/harnessa i centrum uprawnień bez
  zastępowania odbioru TCC.
- **R12 security/release — zamknięty zakres solo/beta**: dependency review/Dependabot, secret
  scanning/push protection i reguły PR/statusów/no-force dla `main` już są.
  Kod CodeQL/cargo-deny oraz bramek metadanych/artefaktów i audytu reguł jest już
  przygotowany lokalnie. Migracja wszystkich pięciu obsolete pakietów bez wyjątków
  oraz integralność dziewięciu forków przechodzą. Za zgodą użytkownika reguły solo
  wymagają PR, ośmiu ścisłych app-bound checków i podpisanych squash merge bez
  bypassów; audyt reguł zaliczony, zero niezależnych aprobat nie udaje review.
  Świeże workflow, podpisany merge i mainline provenance potwierdzone;
  zero otwartych alertów security, sześć udokumentowanych false positive.
  Developer ID/notaryzacja zależą od konfiguracji Apple.

Import plików sesji jest gotowy, ale automatyczny discovery/sync źródeł, natywne
wznawianie sesji importowanych i relay/SSH import nie należą do obecnego zakresu.
`TODO-project-ui.md` nie ma już zaległych pozycji funkcjonalnych.
