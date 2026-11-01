# Pozostałe prace — przegląd 2026-11-01

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
Manualna QA jawnie odroczona dla serii beta, nie zaliczona.
[Dowody i granice](release-acceptance.md).

## Częściowo wdrożone — czego jeszcze brakuje

1. **Rozmowy i ergonomia**: odgałęzianie od wiadomości (nie to samo co branch Git),
   Undo usuwania projektów/sesji, menu kontekstowe wiadomości i tool calli,
   dopracowanie zawijania composera w wąskim oknie.
2. **Design i dostępność**: pełna migracja widoków do tokenów typografii/spacing,
   Compact/Comfortable, kompleksowy audyt klawiatury, focusu i dostępności.
   Obecne Design settings, light-theme polish i zmiana szerokości sidebara są gotowe.
3. **Ustawienia projektu/grupy**: domyślne model/env przez relay i dalsze ustawienia
   specyficzne dla adapterów. Lokalne model/env per projekt i ustawienia grup z resolverem
   global → przodkowie → projekt, źródłami wartości i resetem dziedziczenia są
   wdrożone. Model per harness dotyczy nowych sesji, env następnego połączenia,
   polityka klienta kolejnych żądań. Jest jawne zastosowanie zapisanego modelu
   do bezczynnej sesji przez ACP; bez restartowania trwających agentów.
   Harness, permission mode i własna ikona projektu są gotowe. Ręczna QA nowych
   dialogów nadal oczekuje wykonania.
4. **Grupy v2**: ręczna kolejność, wybór kolorów i sync relay/iOS. Hierarchia,
   List/Tree, sortowanie, drag & drop przypisań/przenoszenia grup oraz bezpieczne
   usuwanie/promowanie/Undo już działają.
5. **Usage**: dokładne skanery logów Claude/Codex/OpenCode z checkpointami i dedupe,
   rozliczeniowy cennik cache/long-context/`unpriced`, cache reuse rate, pełne
   przekroje global/projekt/model/harness, budżety i alerty. Są już recorded-data
   Dashboard, aktualizacje ACP i scope-aware limity kont; nie zastępują one
   pełnego billing usage. Trwałe limity żądań/czasu/raportowanego USD per zadanie
   są wdrożone; nie są gwarantowanym limitem rachunku ani pełnym rozliczeniem
   wewnętrznych retry/subagentów. Lokalny reader OAuth usage Claude pozostaje niewłączony.
6. **Onboarding i uprawnienia**: pełny kreator harness/model/trust/relay/SSH,
   centrum statusów Notifications/Accessibility/Screen Recording/Automation/FDA,
   linki do System Settings i ponowne sprawdzanie. Podstawowy pierwszy ekran,
   wykrywanie adapterów, trust i powiadomienia już istnieją.
7. **Git v2**: ręczna QA mutacji/hoverów i rzeczywistego relay, checkout commita z grafu
     i zdalny branch create. Mutacje stash/worktree, merge/rebase/cherry-pick,
     continue/abort i inline blame są wdrożone local/relay z preflightem,
     jednorazowymi planami i ochroną buforów — [zakres](git-operations-and-blame.md).
     Bezpieczny switch, lokalna historia DAG/refs,
    tworzenie brancha z zaznaczonego commita oraz zakładka grafu w głównym panelu
    z listą plików/diffem commita są gotowe w kodzie. Porównania root/merge-parent,
    rename/delete/binary mają testy; ręczna QA nowego panelu nadal oczekuje.
    Read-only historia przez relay (graf/szczegóły/patche) jest wdrożona z grantami
     i przypięciem zakresu; nowe endpointy mutacji/blame wymagają aktualizacji relay.
    Podglądy read-only stash/worktree (w tym blokady/prunable), stanu operacji
    i konfliktów indeksu są wdrożone lokalnie i przez relay; ich GUI nadal wymaga QA.
8. **Global search**: FTS5 transkryptów i sesji, wspólne filtry i snippety.
   Aktualne wyszukiwanie plików w projekcie nie realizuje całego zakresu.
9. **Skills**: discovery istniejących zewnętrznych bibliotek, pełne włączanie
   per projekt, import z folderu/repo i audyt hash/pochodzenia. Globalna biblioteka,
   edycja/walidacja i bezpieczna instalacja/usuwanie własnych kopii do rootów
   harnessów już istnieją.
10. **Agent graph / Trace**: pan/zoom, zaawansowany auto-layout, minimapa,
    wyszukiwanie, rozszerzona klawiatura, sticky axis i wirtualizacja. Obecne
    hierarchie, zwijanie/rozwijanie, filtry, timing i eksport SVG to gotowa baza.

## Dalszy backlog — nowe podsystemy i już rozpoczęte rozszerzenia

Poniższa lista **nie** oznacza, że każdy moduł jest niewdrożony. Markdown,
workbench i źródła iOS już istnieją; Run/terminal, FTS5, scheduler i koordynator
nie wynikają automatycznie z istniejących checks/search plików/A/B.

11. **Terminal**: PTY/VT, taby/splity, scrollback, search, linki, shell integration,
    tryb lokalny/relay/SSH; wybór silnika VT.
12. **Run**: wykrywanie komend z manifestów i skryptów, panel i historia uruchomień.
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
15. **Standalone chat**: ChatSpace/scratch workspace, izolowane historie/drafty,
    picker Chat/repo oraz jawnie zatwierdzany kontekst repozytorium.
16. **Automatyzacje**: cron/RRULE, precheck, worktree, runy/historia/usage/limity,
    LaunchAgent i scheduler relay, praca przy zamkniętej aplikacji.
17. **SSH**: ControlMaster, Keychain, known_hosts, bootstrap i tunel relay,
    projekty oraz terminal zdalny. Obecny relay nie oznacza gotowego SSH.
18. **Computer use**: osobny Swift/AppKit helper, TCC, AX/ScreenCaptureKit/CGEvent,
    allowlist, zatwierdzenia człowieka, globalny stop i audit trail.
19. **iOS — rozpoczęty, nieodebrany**: źródła SwiftUI/ACP i konfiguracja projektu
    istnieją; build Xcode i działanie na urządzeniu/relay nie są potwierdzone.
    Pozostają również sync grup/usage/artefaktów i powiadomienia.
20. **Orkiestracja multi-agent**: trwały message bus/inbox, coordinator i DAG zadań,
    worktree per subtask, handoff/review/merge, routing modeli, budżety per rola,
     living spec/task board i human gates.
     Osobne porównanie A/B już tworzy dwa worktree z jednego commita, uruchamia
     identyczny zatwierdzony prompt ze wspólnym budżetem i zapisuje preferencję
     bez merge. Nie zastępuje koordynatora/handoffów.
21. **Agent workbench/GitHub**: osiem obszarów ma ograniczone implementacje;
    pozostają ręczna QA dialogów/klawiatury/a11y, prawdziwy GitHub i wdrożony relay,
    pełny billing, semantyka compiler/LSP/non-Rust oraz zdalne checks/comparison/index.
    Mapa wpływu działa jako Rust AST/lexical references i kandydaci testów, nie
    dowód pokrycia. [Dokładny zakres i walidacja](agent-workbench-and-github.md).

## Fundament i wydanie — oddzielnie od funkcji

- Dalszy podział dużego `app.rs`, wspólne Services, modele/migracje i event bus
  dla nowych subsystemów z `PLAN.md`.
- **R11 QA**: świeże konto macOS, brak harnessa, odmowa powiadomień, upgrade bazy,
  duży projekt i remote beta; pełna weryfikacja GUI pozostaje do wykonania.
  Użytkownik zaakceptował odroczenie GUI/TCC/realnego hosta tylko dla `0.1.0-beta.N`.
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
