# Plany implementacji

Plany 01–07 wynikają z audytu Git v2, rozmów oraz agent workbench.
Plany 08–19 rozwijają porównanie obsługi Dunlina z VS Code, JetBrains i Orcą,
z naciskiem na narzędzie AI-first, nie kopiowanie klasycznego IDE.
Plany 20–21 uzupełniają audyt skills i ustawień relay/grup; nie powtarzają
istniejącej biblioteki skills, kolorów/kolejności grup ani lokalnego dziedziczenia.
Numeracja identyfikuje zakres; nie oznacza kolejności wykonania.

Status początkowy: **zaplanowane, nieodebrane**. Opisy stanu bazują na audycie
working tree z 2026-11-09, nie na samej dokumentacji ani ostatnim commicie.
Przed rozpoczęciem każdego planu ponownie sprawdzić źródła i trwające zmiany.
Istnienie testu lub wcześniejszego raportu nie oznacza świeżego wyniku testów.

## Lista planów

| Nr | Plan | Charakter | Priorytet |
| --- | --- | --- | --- |
| 1 | [Git v2 — odbiór ręczny i rzeczywisty relay](01-git-v2-odbior.md) | QA i naprawy wykrytych regresji | P1 |
| 2 | [Branch rozmowy od wiadomości](02-branch-od-wiadomosci.md) | Model historii, harness, UI, relay | P1 |
| 3 | [Undo usunięcia projektu i sesji](03-undo-usuniecia.md) | Storage, runtime, UI, relay | P1 |
| 4 | [Menu kontekstowe wiadomości i tool calli](04-menu-kontekstowe.md) | Wspólne akcje i UI | P1 |
| 5 | [Composer w wąskim oknie](05-composer-waskie-okno.md) | Reprodukcja, layout i regresje | P1 |
| 6 | [Billing i kompletność rozliczeń](06-billing.md) | Poprawność, rejestr zdarzeń i uzgadnianie | P1 poprawność / P2 rozszerzenia |
| 7 | [Semantyka LSP w agent workbench](07-lsp-agent-workbench.md) | Nowy, opcjonalny subsystem | P2 |
| 8 | [Search Everywhere i szybkie otwieranie](08-search-everywhere.md) | Wyszukiwanie, indeks i nawigacja | P1 launcher / P2 pełny indeks |
| 9 | [Kontekst z miejsca pracy](09-kontekst-z-miejsca-pracy.md) | Wersjonowane załączniki i provenance | P1 |
| 10 | [Akcje kontekstowe AI](10-akcje-kontekstowe-ai.md) | Małe interakcje i wspólne akcje | P1 v1 / P2 LSP |
| 11 | [Skrzynka „Wymaga uwagi”](11-skrzynka-wymaga-uwagi.md) | Agregacja decyzji, inbox i powiadomienia | P1 |
| 12 | [Kolejkuj, skoryguj, zatrzymaj](12-kolejka-korekta-stop.md) | Semantyka wysyłania i lifecycle ACP | P1 kolejka/stop / P2 steering |
| 13 | [Review zadania i aktualność dowodów](13-review-zadania.md) | Review, Checks i kryteria akceptacji | P1 review / P2 spec-aware |
| 14 | [Oś czasu i bezpieczne odzyskiwanie](14-os-czasu-i-odzyskiwanie.md) | Zdarzenia, checkpointy i restore | P2 |
| 15 | [Przywracanie workspace po restarcie](15-przywracanie-workspace.md) | Storage szkiców, kart i układu | P1 |
| 16 | [Rejestr akcji, skróty i fokus](16-akcje-skroty-i-fokus.md) | Wspólna infrastruktura obsługi | P1 |
| 17 | [Workspace zadania](17-workspace-zadania.md) | Model zadania i opcjonalny worktree | P2 |
| 18 | [Przepływ plan → wykonanie](18-plan-do-wykonania.md) | Rewizje planu, zgody i dispatch | P2 |
| 19 | [CLI/API sterujące aplikacją](19-cli-api-aplikacji.md) | Serwisy, capabilities i transport | P2 lokalnie / P3 relay |
| 20 | [Skills: frontmatter i odbiór](20-skills-frontmatter-i-odbior.md) | Parser, bezstratne metadane, migracja i QA istniejącego UI | P1 poprawność/QA / P2 metadane |
| 21 | [Relay: ustawienia i sync grup](21-relay-ustawienia-i-sync-grup.md) | Aktualność/CAS, organizacja per-user i zdalne Apply model | P1 poprawność/QA / P2 sync |

Priorytety nowych planów są propozycją kolejności produktowej; nie zastępują
blokad wydania P0 w `docs/project/roadmap.md`. W grupie większych inspiracji ujęto tylko
workspace zadania, plan → wykonanie i CLI/API. Design Mode, dyktowanie, daemon
utrzymujący procesy oraz pełny koordynator/DAG nie są nowymi zakresami tych planów.

## Zalecana kolejność

1. Wykonać odbiór Git z planu 1 oraz małe zakresy 5 i 4.
2. Naprawić poprawność obecnego billingu w pierwszym etapie planu 6.
3. Uzgodnić wspólny kontrakt historii z planów 2 i 3: stabilne identyfikatory,
   niezmienny prefiks, retencja i usuwanie rodzica odgałęzienia.
4. Dostarczyć lokalne Undo z planu 3, następnie lokalne branchowanie z planu 2;
   rozszerzenia relay odbierać osobno po wdrożeniu odpowiedniej wersji serwera.
5. Rozbudować billing z planu 6 o trwałe zdarzenia i korelację źródeł.
6. Dostarczyć lokalny Rust LSP v1 z planu 7; relay i kolejne języki później.

## Zależności

- Plan 4 może powstać przed planem 2, ale akcję branchowania należy dodać dopiero
  po implementacji jej semantyki i sprawdzania dostępności.
- Plany 2 i 3 muszą współdzielić reguły retencji: usunięcie rozmowy źródłowej
  nie może uszkadzać historii jej odgałęzień.
- Plan 2 korzysta z rozróżnienia fork/subagent w planie 6; historyczny prefiks
  nie może zostać ponownie policzony jako nowa płatna praca.
- Plan 7 nie blokuje obecnej leksykalnej impact map ani odbioru Git.

## Wspólne zasady realizacji

- Nie nadpisywać cudzych niezacommitowanych zmian. Aktualny working tree jest
  punktem odniesienia; konflikty zakresu wyjaśnić przed edycją.
- Nowe nazwy pól, tabel i endpointów w planach są propozycjami, nie istniejącym API.
- Migracje muszą zachować stare projekty, sesje, transkrypty i ustawienia.
- Nie uruchamiać agenta, nie wysyłać promptu i nie wykonywać mutacji Git jako
  ukrytego skutku otwarcia menu, przywrócenia danych lub utworzenia odgałęzienia.
- Brak danych pozostaje jawny; nie zastępować go kosztem zero, sukcesem ani
  obietnicą pełnej semantyki lub pokrycia testami.
- Zmiana endpointów nie wdraża relay. Osobno zapisać wersję klienta, serwera,
  protokołu i wynik odbioru na rzeczywistym hoście.
- Testy uruchamiać na izolowanych danych; prawdziwy host i płatne wywołania
  wymagają osobno uzgodnionego środowiska i zgody.
- Po implementacji uruchomić adekwatne testy pakietów, formatting, Clippy
  i `git diff --check`. Wyniki automatyczne i ręczne raportować osobno.

## Kolejność i kontrakty planów AI-first

1. Zacząć od minimalnego rejestru akcji i napraw fokusu z planu 16; nie czekać
   na kompletny edytor keymap. Małe akcje istniejącego planu 4 nie są blokowane.
2. Dostarczyć lokalny kontekst v1 (9), launcher (8), inbox (11), queue/stop (12)
   i trwałe szkice/układ (15) jako niezależne, stopniowe rozszerzenia.
3. W planie 17 najpierw uzgodnić wspólną tożsamość zadania i relacje ze spec,
   coordination, budżetem i worktree. Nie wymagać całego kreatora przed review.
4. Rozwijać review (13) oraz akcje kontekstowe (10) na istniejących sesjach;
   następnie podpinać zadanie, oś czasu (14) i zatwierdzanie planów (18).
5. API odczytowe z planu 19 może powstać po audycie brokera i wspólnych serwisów.
   Mutacje zadania/planów dopiero po ich kontraktach; remote odbierać osobno.

- Rejestr akcji (16) opisuje intencję i dostępność. Serwisy domenowe ponownie
  egzekwują uprawnienia; sam aktywny przycisk lub identyfikator akcji nie jest grantem.
- Kontekst (9), review (13), timeline (14) i plany (18) współdzielą identyfikatory
  źródeł oraz rewizje/hash. Stary dowód nie staje się aktualny po odtworzeniu UI.
- Task spec (`T###`), `coordination::Task`, budżet i worktree nie są dziś jednym
  obiektem. Plan 17 ustala mapowanie/migrację; pozostałe plany nie tworzą własnych,
  konkurencyjnych tabel zadań. Wczesne wersje mogą działać w scope sesji.
- `crates/coordination/` i `apps/desktop/src/coordination.rs` zawierają w working
  tree model run/task, wiadomości, continuations i broker MCP. Najpierw potwierdzić
  ich aktualną integrację, walidację CLI i testy; obecność plików nie dowodzi
  działającego wydania. Nie uruchamiać drugiego brokera lub inboxu bez uzasadnienia.
- Persistent draft/queue (15) przechowuje treść i stan dostarczenia; plan 12
  definiuje wysyłanie. Restart nie odtwarza automatycznie prawa do wykonania.
- Zgoda na plan (18), uruchomienie agenta, narzędzie i mutację Git to oddzielne
  decyzje. Agent nie może sam wystawić sobie zgody przez CLI, MCP ani wiadomość.
- Domyślnie bez nowej telemetrii i bez płatnych testów. Miary UX zbierać na
  kontrolowanych scenariuszach lokalnych; nie wysyłać historii do zewnętrznej usługi.

Każdy plan 08–19 ma oddzielny zakres v1, etapy i testy. Rozszerzenia oznaczone
jako późniejsze nie są warunkiem odbioru v1. Po wdrożeniu aktualizować roadmapę,
nie prowadzić drugiego, rozbieżnego rejestru statusów ukończenia.

## Domknięcie dokumentacji

Dla zakresów 20–21: najpierw odebrać istniejące UI/runtime i zgodność skills,
potem lokalne FTS5 historii (C1 planu 08), a pełny sync grup dopiero po kontrakcie
konfliktów/uprawnień. Nowe plany nie przesuwają blokad P0 ani wcześniejszych zadań P1.

Po wykonaniu danego zakresu zaktualizować odpowiadające mu pozycje
[roadmapy](../docs/project/roadmap.md), przewodnik i changelog.
Zgodnie z udokumentowanym wycofaniem PR/Actions usunąć sprzeczne aktywne wpisy
backlogu lub oznaczyć je jako historyczne. Żaden z tych planów nie przywraca
PR/Actions, nie rozszerza mutacji Git ani funkcji blame.
