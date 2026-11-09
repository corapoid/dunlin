# 19. CLI/API sterujące aplikacją jak w Orce

## Cel i stan obecny

Agent i człowiek mogą użyć tych samych bezpiecznych usług bez klikania w GUI.
Dunlin ma desktop CLI do startu/diagnostyki/smoke, relay API i dedykowane integracje
MCP. W working tree coordination zawiera Unix broker, sesyjne tokens, stdio MCP
oraz operacje peers/messages/tasks/continuations. Nie jest to jeszcze ogólne,
wersjonowane CLI/API workbench; sprawdzić również routing i walidację helper flags.

## Zakres v1 i miejsca integracji

- Lokalne API odczytowe: capabilities, właściwy scope, status sesji/zadań,
  raport Checks i metadata review; dane tylko w autoryzowanym zakresie.
- Następnie jawne propozycje mutacji: draft/feedback, przygotowanie checks/worktree.
- `apps/desktop/src/{cli.rs,main.rs,coordination.rs,remote.rs}` i serwisy domenowe.
- `crates/coordination/src/{service.rs,mcp.rs,store.rs}`; istniejące MCP i vault.
- `apps/desktop/src/git_history/{checks.rs,review.rs,subtasks.rs}` oraz registry (16).
- Nazwy nowych komend/endpointów w tym planie są propozycjami, nie istniejącym API.

## Etap A — granica usług i threat model

1. [ ] Zinwentaryzować istniejące CLI/helper flags, MCP, broker i relay.
   Potwierdzić ścieżkę od argumentów przez validate do handlera; brak obsługi
   flagi nie może być maskowany testem samego modułu MCP.
2. [ ] Wydzielać minimalne usługi używane przez GUI, bez przenoszenia całego
   `DunlinApp` do publicznego API. ActionId z planu 16 jest intencją, nie ogólnym
   poleceniem uruchomienia dowolnej funkcji wewnętrznej.
3. [ ] Ustalić allowlistę operacji i ich skutki: read, metadata write, propose,
   execute. Nie każda akcja UI musi być dostępna agentowi. Shell/SQL/Keychain,
   arbitralne file reads i „dispatch dowolnej akcji” nie należą do v1.
4. [ ] Zdefiniować principal, authority, host/project/root/session i capability
   expiry/revocation. Scope bierze się z uwierzytelnionego grantu, nie argumentu
   podanego przez model. Respektować trust, permission ceiling i budget.
5. [ ] Uwzględnić granice lokalnego bezpieczeństwa: Unix permissions i token
   ograniczają kanał, ale nie sandboxują złośliwego procesu o tych samych prawach OS.
   Nie przedstawiać IPC jako ochrony przed pełnym kompromisem konta użytkownika.

## Etap B — wersjonowany kontrakt i lokalny odczyt

1. [ ] Ustalić API version/capabilities, request ID, typowane schema, structured
   error codes, bounded pagination, timeout i cancellation. Reject unknown fields.
2. [ ] Rozwinąć lub współdzielić istniejący broker zamiast stawiać niezależny
   port HTTP. Prywatny Unix socket i credential provisioning bez tokenów w argv,
   repo, logach oraz historii shella; nie dziedziczyć grantów do terminala ogólnego.
3. [ ] CLI oferuje `--help`, wersję, stabilny `--json`, exit codes i czytelny
   output dla człowieka. Uruchomienie help/version nie startuje GPUI ani agenta.
4. [ ] Pierwsze operacje zwracają tylko przyznane metadata/status/raporty.
   Transcript/context/log to osobne uprawnienie do wrażliwej treści, nie domyślny dump.
5. [ ] Zwracać observed facts, agent claims i unknown osobno. Odczyt capabilities
   nie wywołuje płatnego testu modelu ani automatycznej instalacji harnessa.
6. [ ] Wyłączona aplikacja daje jawne unavailable; v1 nie uruchamia ukrytego
   daemona. CLI samo nie otwiera prywatnej SQLite, omijając serwis/grants.

## Etap C — propozycje i kontrolowane mutacje

1. [ ] Dodać prepare/confirm/execute dla wybranych efektów. Niektóre bezpieczne
   metadata writes można dopuścić capability policy, ale nie zmieniają one praw
   do promptu, komendy, worktree creation ani plan approval.
2. [ ] Propozycja ma ID, content hash, dokładny scope, expiry i preview skutków.
   Zatwierdzenie przez człowieka w GUI wiąże exact payload; agentowe CLI/MCP
   nie udostępnia metody wystawienia sobie zgody lub powiększenia capabilities.
3. [ ] Reuse istniejących dispatch, locks, confirmations i limits. Checks/run
   wykonują kod projektu, więc nie są read-only tylko dlatego, że służą walidacji.
4. [ ] Mutacje mają idempotency key z payload digest i trwały operation state.
   Duplicate zwraca ten sam wynik; inny payload pod tym kluczem jest odrzucany.
   Crash/timeout po skutku daje unknown i reconciliation, nie automatyczny retry.
5. [ ] Audit zapisuje aktora, operation/target, źródło zgody i rezultat; redaguje
   sekrety/payloady. Purge i retencja są jawne; agent nie usuwa śladów własnych zgód.
6. [ ] Te same serwisy obsługują GUI, CLI i allowlisted MCP tools. Annotation
   readOnlyHint nie zastępuje egzekwowania uprawnień na serwerze.

## Etap D — relay i udokumentowana integracja agentów

1. [ ] Osobno zaprojektować remote identity/grants/quotas i negotiate API version.
   Nie forwardować lokalnego session tokenu jako uprawnienia administratora relay.
2. [ ] Stary serwer jawnie odmawia operacji. Odczyt lub mutacja remote nigdy
   nie przechodzi na lokalny root/shell przy błędzie połączenia.
3. [ ] Opisać dostępne komendy/MCP i ograniczenia w pomocy oraz instrukcji dla
   agentów. Dokumentacja odpowiada dokładnie wersji binarki i jej capabilities.
   Nie uruchamiać general-purpose MCP connector host jako warunku tego planu.

## Testy i kryteria odbioru

- [ ] CLI help/JSON/exit codes i helper validation, IPC framing/oversize/timeout.
- [ ] Token brak/expired/revoked, root/trust changed, cross-session/project/host.
- [ ] Pole permission/approved przekazane przez klienta nie daje nowego grantu.
- [ ] Read nie wysyła promptu ani nie uruchamia kodu; brak capabilities jest jawny.
- [ ] Parity GUI/CLI/MCP: identyczne blokady, scope i skutki danej operacji.
- [ ] Duplicate key, zmieniony payload, restart po skutku i utracona odpowiedź.
- [ ] Nie ma sekretów w argv, stdout diagnostycznym, logach lub pakiecie eksportowym.
- [ ] Fake IPC/MCP i mock ACP działają bez konta/modelu; realny relay odbierany osobno.

## Zależności i granice

Akcje (16), domenowe task/plan (17/18) dla ich mutacji, dispatch (12), kontekst (9).
Read-only v1 nie czeka na wszystkie te funkcje, ale wymaga audytu istniejącego brokera.
Bez automatyzacji cyklicznych, computer use, mobilnego klienta, publicznego portu
sieciowego i autonomicznego tworzenia workerów. Nie kopiować całej Orki naraz.

Źródło: [Orca CLI](https://www.onorca.dev/docs/cli/overview).
