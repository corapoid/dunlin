# Pozostałe prace — przegląd 2026-10-30

Źródła: `ROADMAP.md`, `PLAN.md`, obie listy `TODO-*` i aktualny kod.
To lista brakujących zakresów, a nie kopia nieaktualnych większych checkboxów.

## Częściowo wdrożone — czego jeszcze brakuje

1. **Rozmowy i ergonomia**: odgałęzianie od wiadomości (nie to samo co branch Git),
   Undo usuwania projektów/sesji, menu kontekstowe wiadomości i tool calli,
   dopracowanie zawijania composera w wąskim oknie.
2. **Design i dostępność**: pełna migracja widoków do tokenów typografii/spacing,
   Compact/Comfortable, kompleksowy audyt klawiatury, focusu i dostępności.
   Obecne Design settings, light-theme polish i zmiana szerokości sidebara są gotowe.
3. **Ustawienia projektu/grupy**: model i env per projekt, ustawienia per grupa,
   stosowanie pozostałych zmian bez restartu harnessa. Harness, permission mode
   i własna ikona projektu są gotowe.
4. **Grupy v2**: ręczna kolejność, wybór kolorów i sync relay/iOS. Hierarchia,
   List/Tree, sortowanie, drag & drop przypisań/przenoszenia grup oraz bezpieczne
   usuwanie/promowanie/Undo już działają.
5. **Usage**: dokładne skanery logów Claude/Codex/OpenCode z checkpointami i dedupe,
   rozliczeniowy cennik cache/long-context/`unpriced`, cache reuse rate, pełne
   przekroje global/projekt/model/harness, budżety i alerty. Są już recorded-data
   Dashboard, aktualizacje ACP i scope-aware limity kont; nie zastępują one
   pełnego billing usage. Lokalny reader OAuth usage Claude pozostaje niewłączony.
6. **Onboarding i uprawnienia**: pełny kreator harness/model/trust/relay/SSH,
   centrum statusów Notifications/Accessibility/Screen Recording/Automation/FDA,
   linki do System Settings i ponowne sprawdzanie. Podstawowy pierwszy ekran,
   wykrywanie adapterów, trust i powiadomienia już istnieją.
7. **Git v2**: stash, obsługa konfliktów, zarządzanie worktree, merge/rebase/
   cherry-pick, inline blame, diff wybranego commita oraz docelowy graf w głównym
   panelu. Bezpieczny switch, lokalna historia DAG/refs i tworzenie brancha z
   zaznaczonego commita są gotowe. Nowy graf jest osobnym overlayem, tylko lokalnie;
   parity relay i dalsza integracja grafu nadal czekają.
8. **Global search**: FTS5 transkryptów i sesji, wspólne filtry i snippety.
   Aktualne wyszukiwanie plików w projekcie nie realizuje całego zakresu.
9. **Skills**: discovery istniejących zewnętrznych bibliotek, pełne włączanie
   per projekt, import z folderu/repo i audyt hash/pochodzenia. Globalna biblioteka,
   edycja/walidacja i bezpieczna instalacja/usuwanie własnych kopii do rootów
   harnessów już istnieją.
10. **Agent graph / Trace**: pan/zoom, zaawansowany auto-layout, minimapa,
    wyszukiwanie, rozszerzona klawiatura, sticky axis i wirtualizacja. Obecne
    hierarchie, zwijanie/rozwijanie, filtry, timing i eksport SVG to gotowa baza.

## Nowe podsystemy z planu

11. **Terminal**: PTY/VT, taby/splity, scrollback, search, linki, shell integration,
    tryb lokalny/relay/SSH; wybór silnika VT.
12. **Run**: wykrywanie komend z manifestów i skryptów, panel i historia uruchomień.
13. **Artefakty**: model/storage/panel, jawne „zapisz jako artefakt”, preview/source
    dla Markdown/HTML/CSV/obrazów/diffów; później share, wygasanie i revoke.
14. **Markdown**: dedykowane source/preview/toolbar, następnie natywny blokowy
    edytor AST z raw mode i undo/redo. Zwykła edycja plików już działa.
15. **Standalone chat**: ChatSpace/scratch workspace, izolowane historie/drafty,
    picker Chat/repo oraz jawnie zatwierdzany kontekst repozytorium.
16. **Automatyzacje**: cron/RRULE, precheck, worktree, runy/historia/usage/limity,
    LaunchAgent i scheduler relay, praca przy zamkniętej aplikacji.
17. **SSH**: ControlMaster, Keychain, known_hosts, bootstrap i tunel relay,
    projekty oraz terminal zdalny. Obecny relay nie oznacza gotowego SSH.
18. **Computer use**: osobny Swift/AppKit helper, TCC, AX/ScreenCaptureKit/CGEvent,
    allowlist, zatwierdzenia człowieka, globalny stop i audit trail.
19. **iOS**: działający build i sync grup/usage/artefaktów/powiadomień.
20. **Orkiestracja multi-agent**: trwały message bus/inbox, coordinator i DAG zadań,
    worktree per subtask, handoff/review/merge, routing modeli, budżety per rola,
    living spec/task board i human gates.

## Fundament i wydanie — oddzielnie od funkcji

- Dalszy podział dużego `app.rs`, wspólne Services, modele/migracje i event bus
  dla nowych subsystemów z `PLAN.md`.
- **R11 QA**: świeże konto macOS, brak harnessa, odmowa powiadomień, upgrade bazy,
  duży projekt i remote beta; pełna weryfikacja GUI pozostaje do wykonania.
- **R12 security/release**: dependency review, secret scanning, CodeQL/cargo-deny,
  branch protection, wymagane review wrażliwych ścieżek, podpisane commity/release;
  Developer ID i notaryzacja zależą od konfiguracji konta Apple Developer.

Import plików sesji jest gotowy, ale automatyczny discovery/sync źródeł, natywne
wznawianie sesji importowanych i relay/SSH import nie należą do obecnego zakresu.
`TODO-project-ui.md` nie ma już zaległych pozycji funkcjonalnych.
