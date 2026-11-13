# Architektura i kontrakty

## Transport i prywatność

1. Dunlin Server binduje wyłącznie numeric loopback. Tailscale Serve publikuje
   prywatne HTTPS/WSS, nigdy Funnel. Domyślny port relay: 8787.
2. Web ładuje się z tego samego originu co backend. Tylko loopback HTTP lub
   HTTPS z poprawnym hostname `.ts.net`. Inne originy są odrzucane.
3. `.ts.net` jest kontrolą składni, nie dowodem prywatności ani tożsamości.
   ACL/grants, ustawienia Serve/Funnel i rejestracja hosta pozostają wymagane.
4. Lokalne WS jest dozwolone w release dla nowego browser API, nie zmienia to
   automatycznie dotychczasowych polityk klientów desktop/CLI.
5. Brak publicznego fallbacku, zewnętrznego reverse proxy, bezpośrednich IP LAN,
   HTTP poza loopback i połączeń wychodzących omijających tailnet.

## Live

WebSocket/WSS jest wybrany ze względu na wsparcie browserów i Tailscale Serve.
WebTransport nie jest wymagany: nie daje tu korzyści uzasadniających zależność
od HTTP/3 i dodatkową macierz kompatybilności. Nie zastępujemy push pollingiem.

Docelowo sesje używają wydanej rewizji AHP, SDK i testów conformance. Projektowe
zdarzenia live mają osobny kontrakt z `protocol_version`, `stream_id`, sekwencją,
snapshotem i komunikatem `resync_required`. Snapshot/replay jest autorytatywny;
reconnect nie może wznowić mutacji bez idempotency key. Heartbeat wykrywa martwe
połączenie, nie odświeża cyklicznie danych. Ograniczone kolejki i maksymalne frame
sizes; lag wymusza resync zamiast gubienia zdarzeń bez sygnalizacji.

Zweryfikowany baseline dla E3 (2026-11-11): **AHP 1.0.0**, opublikowany tag
`v1.0.0`; nie nieprzypięty draft `main`. Oficjalne SDK Rust
`ahp`/`ahp-types`/`ahp-ws` i TypeScript `@microsoft/agent-host-protocol`
mają wydanie 1.0.0. SDK client-side nie zastępuje implementacji serwera.
Implementacja i conformance E3 nadal są otwarte. Źródła:

- https://github.com/microsoft/agent-host-protocol/tree/v1.0.0
- https://github.com/microsoft/agent-host-protocol/blob/main/clients/rust/CHANGELOG.md
- https://www.npmjs.com/package/@microsoft/agent-host-protocol/v/1.0.0

Doprecyzowanie implementacji AHP (ponowny odczyt źródeł v1.0.0):

- serverSeq w AHP jest porządkiem serwera obejmującym kanały, podczas gdy
  sequence w obecnym journal jest lokalne dla jednej sesji. Nie wolno po prostu
  podstawić tych liczników. Projekcja potrzebuje trwałego outboxu/porządku,
  atomowego snapshot+subscription i spójnych fromSeq; luki po filtracji authority
  nie uprawniają do odczytu zdarzeń innych użytkowników.
- initialize negocjuje dokładnie oferowaną wersję zgodną z zakresem, a nie
  numer wersji SDK. Kanały mają jawne URI, parametry każdego polecenia zawierają
  channel. WebSocket przenosi jedną wiadomość JSON-RPC na text frame. Cookie,
  Origin/Host i autoryzacja upgrade pozostają warstwą Dunlin.
- reconnect zwraca pełne replay w odpowiedzi albo snapshot; protokolarnych
  notifications nie odtwarza się jako actions. Istniejący bounded journal wymaga
  mapowania do typed actions/reducers i cursorów, nie wysłania raw ACP jako AHP.
- dispatchAction wymaga trwałej deduplikacji według device/clientId/clientSeq
  i fingerprint, walidacji authority i allowlisty actions. Klient nie wybiera
  ścieżki checkoutu/command; createSession musi rozwiązać zatwierdzony binding.
  Akceptacje i cancel wymagają własnych warunków CAS, nie ponowienia mutacji.
- Minimalny przekrój musi przejść oficjalny SDK/reducer oraz rzeczywisty WS
  z dwoma obserwatorami, reconnect/replay, revoke i lag. Dopiero taki dowód
  pozwala włączyć odpowiednią capability dla odebranego zakresu. Obecny opt-in
  realizuje queued text, replay/history, queue controls, cancel i prywatne
  współdzielenie odczytu; część ma rzeczywisty SDK/WS/Chromium PASS. Pełny
  conformance pozostaje otwarty. Pole `chat` nadal jest false; `ahp_observer`
  zależy od działającego opt-in runtime i włączonych transkryptów. Aktualne
  dowody i niewykonane bramki opisują [progress.md](progress.md) i
  [plan](README.md#e3--serwerowy-runtime-i-ahp).

Źródła przypiętego kontraktu:
[transport](https://github.com/microsoft/agent-host-protocol/blob/v1.0.0/docs/specification/transport.md),
[lifecycle](https://github.com/microsoft/agent-host-protocol/blob/v1.0.0/docs/specification/lifecycle.md),
[subscriptions](https://github.com/microsoft/agent-host-protocol/blob/v1.0.0/docs/specification/subscriptions.md),
[versioning](https://github.com/microsoft/agent-host-protocol/blob/v1.0.0/docs/specification/versioning.md).

Pierwszy bootstrap może obsługiwać read-only katalog przez własny protokół
`dunlin.catalog.v1`; to nie jest implementacja AHP ani serwerowych sesji.

### Kontrakt pierwszego katalogu

- `POST /v1/web/pair`: `{code, device_name}` → `{user, device_id}` + cookie.
- `GET /v1/web/session`: bieżąca tożsamość i capabilities. Chat/workflow są false.
- `POST /v1/web/logout`: odwołanie browser device i usunięcie cookie.
- `GET /v1/web/live`: WebSocket upgrade, bez tokenu w query/subprotocol.
- Komenda: `{type: "subscribe", protocol_version: 1, project_id: null | id}`.
- Snapshot: `{type: "snapshot", snapshot: {protocol_version, stream_id, sequence,
  workspace_revision?, server_name, user, projects, project_id, sessions}}`.
- `workspace_revision` jest JS-safe fingerprintem widocznych zdarzeń zespołu,
  a nie globalnym/monotonicznym numerem audytu. Zmiana wymusza odczyt bieżącego
  uprawnionego katalogu metadanych. Nie ujawnia liczby cudzych mutacji.
- Dodatkowo: `resync_required` albo `error` z ograniczonym kodem, bez szczegółów
  poświadczeń i wewnętrznych błędów.
- Maksymalnie 64 WebSockety, 1024 browser sessions, 4 KiB wejścia, 2 MiB snapshot,
  500 projektów i istniejący limit 1000 sesji użytkownika.
- Ping/pong co 15 sekund; kontrola wygaśnięcia, revocation i dostępu do wybranego
  projektu. Limit komend 60/min i deadline wysyłki 5 sekund.
- `stream_id` zmienia się po reconnect; sekwencja jest lokalna dla połączenia.
  Każdy komunikat danych jest pełnym snapshotem, nie trwałym replay logiem AHP.
- Invalidation jest payload-free i pochodzi z zapisów w bieżącym procesie Store.
  Zapisy innym procesem do tej samej bazy nie emitują jeszcze zdarzeń: wymagają
  reconnect. To jawna luka dalszej implementacji, nie gwarancja pełnego live.

## Uwierzytelnienie browsera

Pierwszy przekrój wykorzystuje istniejący jednorazowy kod pairing. Wymiana
zwraca HttpOnly/SameSite=Strict cookie, nigdy token w JSON. Dla Tailscale cookie
jest Secure; HTTP jest dozwolone wyłącznie lokalnie. Origin musi odpowiadać Host,
również podczas WebSocket upgrade i logout. Nie ufamy dowolnym forwarded headers.
Browser sessions mają serwerowy termin ważności; revocation device sprawdzana
także na istniejącym WebSockecie. Brak cookie na starych endpointach bearer API.
Migracja 12 zapisuje wyłącznie hash browser tokenu, device id i absolutny expiry
w osobnej tabeli SQLite. Nieprzeterminowane cookie działa po restarcie; limit
8 godzin nie jest automatycznie odnawiany. Native bearer API nie czyta tej tabeli.
Dotychczasowe memory-only cookie po pierwszym upgrade wymagają nowego pairing.

Opcjonalny OIDC code flow (PKCE/state/nonce) mapuje skonfigurowany issuer/sub
na istniejącego lokalnego użytkownika. Role i uprawnienia nie pochodzą z claims;
bootstrap admina pozostaje lokalną operacją operatora. Migracja 14 zapisuje
provenance browser devices i umożliwia odwołanie ich authority po zastosowaniu
zmienionej konfiguracji. OIDC jest oddzielony od tożsamości workera, kont integracji
i autoryzacji modeli. Instrukcja: [konfiguracja aplikacji](../apps/web/README.md). Tailscale network identity nie daje automatycznie grantów projektowych.

## Dane i execution

- Project jest logiczny; ProjectBinding określa host/repo/root.
- Session ≠ Turn ≠ WorkflowRun ≠ StepRun ≠ Attempt.
- WorkflowVersion i AgentVersion są niezmienne po publikacji.
- Approval obejmuje digest konfiguracji, artifact version/SHA, osobę i zakres.
- Dostęp do prywatnych sesji nie wynika z samej znajomości project id.
- Agent process należy do runtime'u; połączenia UI są subskrybentami.
- Rezerwacja przed dispatch, outbox/inbox, fencing i jawne unknown outcomes.
- `end_turn` to koniec tury, nie dowód sukcesu. Koszty niewidoczne dla klienta
  pozostają unknown; brak deklarowania twardego limitu dolarowego bez pomiaru.

### Docelowe encje (E2–E7, implementacja częściowa)

Migracje 9–17 implementują workspaces/memberships/audit, logiczne team projects,
revision-bound project grants, immutable workflow versions/CAS drafts i trwałe
browser sessions, recipient-bound invitations, provenance OIDC devices i dedykowane scoped
service principals. Service credentials nie są user/device/browser credentials;
obecny bearer endpoint czyta wyłącznie opublikowane workflow. Nie ma jeszcze
wykonania grafu ani nadania host authority przez te tokeny.
To historyczny zakres tych migracji. Późniejszy kod dodaje local/remote
bindings, opt-in serwerowy execution runtime oraz ACL współdzielonego odczytu
historii. Worker persistent sessions i kontrakty admission/stream/recovery
opisują [remote-session-plan.md](remote-session-plan.md) oraz
[remote-bindings.md](remote-bindings.md); ich otwarte odbiory nie wynikają z
samej obecności encji w bazie.
Natywne projekty i prywatne sesje nie są automatycznie przenoszone do workspace.

| Encja | Tożsamość i podstawowe relacje |
| --- | --- |
| Workspace / Membership | workspace_id, user_id, role; brak dziedziczenia grantów z sieci |
| Project | workspace_id, nazwa, metadane; niezależny od ścieżki |
| ProjectBinding | project_id, host_id, repo identity, root, revision konfiguracji |
| Host / HostGrant | tailnet node identity, endpoint Serve, capabilities, scope, revocation |
| Session / Turn | właściciel, project/binding, runtime id, kolejka, model/mode, wersja polityki |
| SessionShare | wskazany projekt/użytkownik/rola, bez automatycznego upublicznienia historii |
| AgentDefinition / AgentVersion | ACP/A2A, instructions/skills digest, tools, model, result schema |
| WorkflowDefinition / WorkflowVersion | projekt, typed graph, trigger policy, immutable digest |
| WorkflowRun / StepRun / Attempt | workflow version, wejścia, host, lease/fence, status, wynik |
| ApprovalRequest / Decision | scope, input/artifact/SHA digest, reviewer, expiry, decyzja |
| IntegrationConnection | workspace/user owner, provider identity/scopes, secret reference, revision |
| ExternalEvent / Outbox | provider event id, idempotency key, retry/reconciliation status |
| Artifact | content hash, owner/scope, wersja, MIME/size, provenance i retencja |
| AuditEvent | actor, target, operation, revision, timestamp; bez sekretów/raw promptów |

Minimalne stany StepRun: queued, dispatching, running, waiting_input,
waiting_approval, succeeded, failed, cancelled, interrupted, unknown. Stanów
boardu Linear nie utożsamiamy ze stanami wykonania ani opinią modelu o sukcesie.
Retry tworzy nową Attempt. Nowa wersja workflow nie modyfikuje aktywnych runs.
Arbitralne cykle są odrzucane; poprawki używają jawnego bounded Repeat.

## Bezpieczeństwo

Nie usuwamy administrator-only boundary hostowych kont Git. Potrzebne są
izolowane credentials/workspaces i jawnie opisana siła sandboxu. Role UI nie
izolują nieufnego kodu. Wyniki agentów, MCP/A2A metadata, Linear opisy i GitHub
komentarze są danymi nieufnymi, a nie źródłem uprawnień. Dla konektorów:
allowlisty celu/operacji, limity, ochrona SSRF, bez wyłączania TLS validation.

Przeglądarka nie odtwarza lokalnych ścieżek z relay jako ścieżek własnego
komputera. Historyczne sesje desktopu nie stają się współdzielone po migracji.
API v1 zostaje; nowe capabilities muszą jawnie odróżniać stare i nowe runtime'y.

### Wewnętrzny dziennik sesji interaktywnych

`dunlin-session-runtime` przechowuje nowe interactive_sessions/turns/events.
To odrębne tabele od indeksu i transkryptów legacy ACP. Przed zewnętrznym zapisem
workera kolejka przechodzi atomowo do dispatching; restart oznacza niepewne
dispatching/running jako unknown i wymaga jawnego ponownego acquire sesji.
Queued nie oznacza promptu wysłanego do harnessu. Completed oznacza zakończenie
tury protokołu, bez stwierdzenia sukcesu zadania lub wdrożenia.

Sam journal nie uruchamia ACP. Eksperymentalny actor w apps/server/src/interactive.rs
jest właścicielem procesu/grupy procesów i lease niezależnie od obserwatorów.
Start wymaga jawnego opt-in experimental_interactive_runtime, transkryptów,
sandboxu (poza fixture testowym) i bieżącego local bindingu. Actor ponownie czyta
device/OIDC/host policy, logiczny i native grant oraz binding/settings; sprawdza
checkout poza blokadą Store w ograniczonej puli filesystem workers. Guardian
co 200 ms obsługuje revoke, timeout i stop. Weryfikuje profil model/mode po
handshake ACP i zapisuje bounded updates oraz końcowy stan tury. Czas/rozmiar
wyjścia nie stanowią limitu kosztu modeli. ReadOnly wymaga osobnego adaptera
kernelowego i obecnie blokuje start; client tools pozostają niedostępne, a ACP permission requests wymagają
jednorazowej decyzji człowieka opisanej poniżej. Zakończenie aktora oznacza interrupted/unknown
bez ponowienia niepewnego promptu. SIGINT/SIGTERM zatrzymuje actorów i blokuje
nowe uruchomienia; SIGKILL/crash wymaga startup recovery.

REST obsługuje jedynie eksperymentalny create, metadata i stop. Nie wystawia
prompt endpoint ani zastępczego live protokołu jako AHP. Kolejka jest wewnętrzna;
pozostałe client actions, pełny AHP conformance, workflow approvals, host routing i pełne UI
rozmowy pozostają otwarte. Obserwator AHP tylko do odczytu jest opisany poniżej. Legacy ACP per-WebSocket nie został przeniesiony.
Global recover jest wyłącznie startupową operacją koordynatora.

### Wspólny porządek dzienników sesji

Migracja 18 dodaje interactive_stream_clock i indeks interactive_stream_events.
Trigger SQLite przy każdym zapisie interactive_events nadaje trwałą sekwencję
wspólną dla wszystkich nowych sesji. Indeks zawiera referencję do oryginalnego
zdarzenia, nie kopię promptu/wyjścia. Błąd indeksu lub wyczerpanie zakresu
JavaScript-safe integer cofa całą transakcję wraz z kolejką i lokalną sekwencją.
Usuwanie źródłowego zdarzenia z aktywną referencją blokuje foreign key RESTRICT.

snapshot_stream odczytuje kursor i wskazane prywatne sesje w jednej transakcji.
replay_stream ponownie autoryzuje każdą subskrypcję przed odczytem; posiadanie
kursora nie nadaje grantu. Zwraca pełny wycinek dla tych sesji do wspólnego
high-water mark albo SnapshotRequired, nigdy pozornie pełną część odpowiedzi.
Nie odczytuje raw danych innych sesji do filtrowania w aplikacji. Maksimum 32
subskrypcje, 128 rekordów replay i 1 MiB serializowanego budżetu. Zbyt duży
snapshot wymaga przyszłego adaptera paginacji; nie jest automatycznie obcinany.

Dawne zdarzenia v17 nie miały porządku między sesjami. Migracja tworzy bazę
snapshotu i replay_floor, poniżej którego wymagany jest nowy snapshot; kolejność
backfill nie jest historycznym replay. Kursor zawiera trwałą tożsamość strumienia,
a luka po odfiltrowaniu innych sesji nie ujawnia ich payloadów. Odczyt globalnego
kursora może wskazać aktywność poza bieżącymi subskrypcjami; ten wewnętrzny licznik
nie zastępuje prywatnych rewizji katalogu i nie został wystawiony przez HTTP.
Tożsamość zapisana wewnątrz backupu nie wykryje sama rozgałęzienia historii po
restore tej samej bazy; odbiór E10 musi wymusić nowy snapshot/generację po restore.

To źródłowy log do budowy osobnej projekcji AHP opisanej poniżej. Jedno zdarzenie
ACP może utworzyć kilka typed actions; projekcja nadaje im własny porządek i zapisuje
je atomowo. Osobne warstwy transportu i kolejki zapewniają subskrypcję przed
snapshotem, snapshoty reducerów, bounded replay i dedupe kolejki tekstowej.
Biblioteka wymaga od adaptera świeżych AccessContext pod blokadą Store;
nie utrwala authority wewnątrz kursora.

### Typowana projekcja ACP do AHP chat

Migracja 19 dodaje interactive_ahp_clock, interactive_ahp_chats i
interactive_ahp_actions. Przy zapisie zdarzenia źródłowego Journal buduje typed
StateAction z przypiętego ahp-types 1.0.0 i stosuje oficjalny reducer ahp 1.0.0.
Źródło, jego sekwencja, wszystkie akcje i wynikowy ChatState commitują razem.
Każda akcja otrzymuje własny globalny serverSeq; nie podstawiamy lokalnego
licznika sesji ani pojedynczej sekwencji raw ACP jako numeru wielu akcji.

Kolejka tworzy pendingMessageSet, dispatching tworzy turnStarted i usuwa queued
message. Kolejne części tekstu/rozumowania mają responsePart przed delta/reasoning;
przeplot z innymi częściami zachowuje kolejność. Agent-managed tool reports mają
start/ready/content/complete; wynik failed nie staje się success. Potwierdzenie
`not-needed` oznacza brak dodatkowego AHP client confirmation dla już raportowanej
operacji; metadata confirmationEvidence=agent-report-only zachowuje granicę dowodu.
Nie powstaje user-action/setting ani zgoda/wykonanie narzędzia przez projekcję.
Diff/resource content pozostaje w structuredContent z pełnym raportem ACP, bez
wymyślania wire variants lub hostowanych artefaktów. Ta projekcja raportów nie tworzy zgody; odrębny flow permission request
wiąże konkretną decyzję człowieka z nonce. Client tools nadal są niedostępne.

Completed jest tylko zakończeniem tury protokołu. Interrupted/recovery tworzy
chat/error z unknown_outcome i resumable=false. Czas pochodzi z monotonicznego
zegara actora; gdy nieznany, duration=0 ma jawny _meta durationUnknown. Nowe
zdarzenia mają zapisany timestamp UTC; stare brakujące timestampy oznaczane są
jako imported, zamiast udawać odzyskaną datę wykonania.

Backfill działa w partiach 64 rekordów wewnątrz migracji. Stare dane, których
adapter nie potrafi przedstawić, pozostają w oryginalnym dzienniku; projekcja
takiej sesji ma jawny stan unavailable i odmawia odczytu zamiast zwracać pusty
transkrypt jako sukces. Backfill ustala replay_floor. Snapshot AHP chat i jego
fromSeq pochodzą z jednej transakcji; per-session odczyt akcji jest wewnętrzną
stroną diagnostyczną, nie pełnym reconnect response.

Limity projekcji: 4096 akcji/16 MiB na sesję, 8 MiB stanu; zwykłe zapisy rezerwują
miejsce dla terminal recovery. SDK src/ pozostają niezmienione względem archiwum
v1.0.0, a manifesty lokalne i całe drzewa sprawdza vendor/provenance.json przez
istniejący CI verifier. Client action dedupe/origin, pełna paginacja, nowe typy
ACP content i pełny conformance pozostają otwarte. Capability chat jest false.

### Lokalny checkout binding

Logiczny team project nie jest ścieżką hosta. Migracja 17 przechowuje jawne
powiązanie z istniejącym native project wraz z CAS revision, canonical root,
pinem device/inode i digestem project settings. Tworzenie bindingu nie kopiuje
ani nie nadaje native grantów. Resolver wymaga równocześnie aktualnej authority
logicznego projektu oraz legacy native grantu użytkownika.

JSON konfiguracji pomija physical paths i inode identity. configuration_current
oznacza jedynie zgodność zapisanej konfiguracji z lokalnym katalogiem, nie
przyznanie execution authority ani gotowość agenta. Podłączenie aktora wymaga
binding revision guard, device/host policy, budżetów i runtime guardian. Migracja25
wersjonuje natywne granty z trwałymi tombstones; actor/ACP, przygotowane operacje
Git, pliki, PTY, jobs oraz prepared restore/comparison/checks przypinają konkretną
epokę. Legacy sesje i konfiguracja projektu sprawdzają proof przy operacji Store.
Host local nie zastępuje enrollmentu
Tailscale. Linked worktrees/commondir/external Git metadata nie są automatycznie
dopuszczane jako dodatkowe sandbox roots.


### Transport obserwatora AHP

Migracja 20 rozszerza globalny porządek o kanały session/chat i utrwala
SessionState oraz negocjowane wersje klientów per browser device. Źródłowy
zapis i wszystkie projekcje mają wspólną transakcję. Powiadomienie bez payloadu
następuje dopiero po commit; odbiorca subskrybuje invalidation przed snapshotem.
Root katalog jest budowany z bieżących grantów i prywatnych sesji właściciela.

`/v1/web/ahp` wymaga obu flag experimental_interactive_runtime/store_transcripts,
ważnego browser cookie, same-origin oraz tych samych granic hostowych co bootstrap.
Initialize/listSessions/subscribe/unsubscribe/reconnect korzystają z wydanych
kontraktów SDK. Klient negocjuje wersję raz; restart serwera wymusza snapshot.
Każdy RPC i push ponownie sprawdza authority. Nieudane RPC nie zmienia kursora
ani subskrypcji, dzięki czemu oczekujące delty istniejących kanałów nie giną.

Snapshot wielu kanałów jest atomowy i ograniczony do 32 kanałów/900 KiB.
Reconnect zwraca cały bounded replay (128 akcji/900 KiB), a przy utracie historii
lub przekroczeniu granicy przechodzi na spójny snapshot. Nie wystawia częściowej
strony jako kompletnego replay. Bardzo duże snapshoty odmawiają odczytu do czasu
implementacji historii stronicowanej. ListSessions ma opaque cursors per connection.
Negocjowane client records wygasają po 8h i mają limity per device/global.

React pokazuje tekst, rozumowanie, raporty narzędzi i błędy bez REST pollingu.
Queued text dispatch do istniejącego actora opisany jest poniżej; one-use tool approvals mają odrębny flow. Create pozostaje otwarte. Finalne rawInput
ACP jest pełną wartością, nie deltą tekstową: ostatnia wersja trafia do standardowego
inline toolInput. Snapshot i replay zachowują ten sam input. Dane resource są
opisem tekstowym, bez automatycznego pobierania zasobów.

Ograniczenie wydanego reducera: chatUpdated nie agreguje parent SessionState.status.
Adapter oblicza aktualny status i wysyła root sessionSummaryChanged; panel konsumuje
te aktualizacje. Sam generic SDK mirror subskrybujący wyłącznie session może różnić
się od nowego snapshotu. Dlatego zgodność dwóch obserwatorów i test oficjalnego
klienta nie zamykają pełnego conformance E3.3 ani interaktywnego E3.5/E3.7.


### Trwały dispatch kolejki tekstowej

Migracja 21 dodaje origin/rejection do action log, interactive_client_dispatch
oraz interactive_message_ids. Typed payload jest kanonizowany i hashowany z kanałem.
Klucz device/clientId/clientSeq nie zawiera kanału: ponowne użycie sekwencji dla
innej operacji jest konfliktem. Ledger nie jest usuwany wraz z ośmiogodzinnym
rekordem negocjacji. Ma stały limit; bezpieczna retencja i operator cleanup pozostają
oddzielnym przyszłym zadaniem, nie wygasza się ochrony przed duplikatem.

Nowe accepted dispatch przechodzi fresh owner/CreateSession oraz existing actor
gate: starter device, OIDC/host policy, harness, binding/native grant i pełny lease
(epoch/runtime instance/authority stamp). Actor nadal wykonuje guardian i checkout
validation przed wysłaniem do ACP. Actor stop/recovery nie zmienia accepted na
wykonany ani nie powoduje powtórzenia. Create/steering nie powstają przez ten transport; tool permission
decisions korzystają z osobnej walidacji opisanej poniżej. Root summary reklamuje możliwość queue per session/user.

Source turn_queued, server turn, client message mapping, typed projection,
ActionOrigin oraz outcome ledger należą do jednej transakcji. Rejection ma własny
globalny serverSeq, lecz nie zmienia snapshotu ani kolejki. Retry po świeżej
owner/CreateSession auth zwraca zapisany wynik przed aktualnym actor gate.
Dzięki temu zatrzymanie actora nie zamienia potwierdzonej akceptacji w rejection.
Payload mismatch i trwałe capacity failures kończą automatyczne retry, zamiast
zapętlać reconnect. Błąd odczytu replay nadal wymusza świeży snapshot.

Po stronie UI confirmed state i optimistic overlay są oddzielne od pamięciowego
outboxu komend. Reconnect odbudowuje baseline; pozostałe niepotwierdzone komendy
wysyłane są z identycznym clientId/clientSeq/channel/action. Re-emit starego outcome
rozlicza origin przed serverSeq dedupe, bez stosowania starej akcji do nowego
snapshotu. Nie utrwala się promptów w browser storage ani nie ponawia po reload.
Nieznana tożsamość klienta z niepewnym outboxem wymaga ręcznej oceny historii.


### Kontrola kolejki i zatrzymanie tury

`CONTROL_SCHEMA` (22) dodaje interactive_cancellation_intents z epoch i stanem
requested/confirmed/unconfirmed. Wspólny dispatch_action rozszerza istniejący
ledger o queued upsert/remove/reorder oraz turnCancelled. Raw JSON i typed
semantyka są walidowane przed zmianą kolejki. Accepted akcja, źródłowy client_control,
zmiana SQL, canonical projection i origin/ledger commitują atomowo.

FIFO ma jawne pozycje; nowy enqueue używa max(position)+1 po reorder. Edycja
przyjmuje wyłącznie nadal queued wiadomość. Usunięcie nie tworzy terminalnego
wpisu transkryptu dla nigdy niewysłanego promptu. Host normalizuje reorder do
pełnej kolejki, ponieważ SDK 1.0.0 sortuje pominięte IDs leksykograficznie zamiast
zachować kolejność z wymagania specyfikacji. NoOp validowanego client_control
może mieć trwały outcome bez udawanej zmiany snapshotu. Vendor jest niezmieniony.

Cancel jest najpierw sekwencjonowany jako koniec przetwarzania chatu przez host.
Intent nie zwalnia SQL running/dispatching, więc actor nie może pobrać następnej
tury. Actor odczytuje intent pod fresh lease, wysyła pojedynczy ACP session/cancel
bez id, a potem czeka na odpowiedź oryginalnego session/prompt. Deadline anulowania
to min(dotychczasowy deadline, teraz+5s). Przed wysłaniem promptu intent może
zakończyć turę jako not_sent. Ten dowód ma prywatny typ RequestReply; nie pochodzi
z pola JSON kontrolowanego przez agenta. Elapsed duration mierzy monotoniczny
Instant actora, nie klient ani różnica timestampów UTC.

Terminal transition i cancel dispatch linearizują się pod Store/SQLite. Jeśli
cancel wygra, późniejszy prompt response potwierdza settlement i SQLCancelled,
bez drugiej terminalnej akcji chatu. Jeśli complete wygra, stale cancel otrzymuje
rejection. Settlement korzysta z recovery reserve; brak miejsca na zwykłe kolejne
zdarzenie nie zamienia otrzymanego ACK w nieznany wynik.

Raw późne updates są zatrzymywane w źródle, lecz po canonical cancel nie redukują
historycznej tury. Warunek skip uwzględnia projected active turn, aby rebuild
zachował pre-cancel historię. Crash/revocation/timeout zamienia requested intents
na unconfirmed i aktywne SQL turns na Unknown, nie odtwarza ich automatycznie.
Session metadata/UI pokazują osobno pending i niepotwierdzony wynik, a status
sesji sygnalizuje błąd nawet bez aktywnego chatu. Zakończenie host-side processing
nie jest dowodem cofnięcia narzędzi ani dowodem sukcesu zadania.


### Jednorazowe decyzje narzędzi

PERMISSION_SCHEMA (23) utrwala interactive_permission_challenges. Każde żądanie
wiąże nonce z runtime/epoch, authority stamp, starter device, binding/harness,
turn/tool, pełnym raportem operacji, opcjami i expiry. UI otrzymuje standardowy
ToolCallReady pending-confirmation oraz opaque IDs mapujące wyłącznie ACP
allow_once/reject_once. Brak persistent allow policy. Zgoda w prywatnej sesji
wymaga human owner/CreateSession; nie omija osobnej polityki Review/SoD workflow.

Minimalny client ToolCallConfirmed jest sprawdzany względem bieżącego nonce
i opcji. Pierwsza decyzja atomowo przechodzi pending→decided z wpisem source,
canonical reducer, urządzeniem/klientem i ledgerem. Retry nie tworzy nowej zgody.
Modelowe raporty agent-report-only nigdy nie stają się user-action. Canonical
potwierdzenie ma pochodzenie human-one-use; nonce i host metadata pozostają
przy późniejszych raportach, a odmowa nie zmienia się w success od tekstu agenta.

Aktor utrzymuje bounded pending tickets i nadal odczytuje ACP podczas oczekiwania
na człowieka. Przed wysłaniem decyzji ponownie sprawdza urządzenie decydenta,
OIDC/host policy, lease/stamp, expiry, aktywną turę i konkretny consumed nonce.
SQL decided→consumed poprzedza zewnętrzny zapis, delivered następuje po flush.
Consumed bez potwierdzenia delivery jest unknown po restarcie i nie jest odtwarzany.
Nie jest to obietnica atomowej transakcji z procesem zewnętrznym ani dowód wykonania.

Terminal/cancel/recovery unieważnia nierozwiązane wyzwania w tej samej transakcji.
Stare nonce nie unieważnia nowszego wyzwania tego samego narzędzia. Delivery oraz
invalidation korzystają z recovery reserve; normalny source budget rezerwuje
16 events/8KiB przy niezmienionym total1024/8MiB, bo osiem oczekujących zgód wymaga
miejsca na lifecycle i terminal. Uprawnienia hosta/sandbox nie są rozszerzane
przez przycisk, raportowane ścieżki są danymi agenta. Jednorazowa decyzja nie jest
zgodą na inne wejście, nową turę, nowy proces lub kolejne żądanie z ponownie użytym
peer toolCallId/requestId.

Session metadata permissionPending/permissionUnconfirmed i UI pokazują odpowiednio
oczekiwanie na decyzję/dostarczenie oraz nieznany wynik. Reducer frontendu zastępuje
canonical metadata po revoke, obsługuje konkurencyjne decyzje i starszy ACK po
nowym snapshocie bez zmiany nowszego wyzwania. Brak credential/prompt persistence
w browser storage; pozostałe pełne conformance i integracje operatora
pozostają osobnymi bramkami.


### Jawne współdzielenie odczytu sesji interaktywnych

Sesja jest prywatna do jawnego zapisu listy odbiorców przez właściciela z
ShareOwnSession. Odbiorca musi być człowiekiem z bieżącym ViewProject oraz
ViewSharedSession w tym samym workspace/projekcie. Globalny admin nie ma
niejawnego prawa do transkryptu. Historyczne sesje desktopu pozostają prywatne.
Udostępnienie obejmuje pełną historię oraz przyszłe parametry/wyniki narzędzi;
nie nadaje praw promptów, zatrzymania aktora, kolejki ani zgód.

Migracja 24 zapisuje listę, stampy authority obu stron, rewizję per grant i audyt.
Każdy odczyt sprawdza aktualne uprawnienia odbiorcy i właściciela. Zmiana grantu
lub członkostwa unieważnia zapisany stamp; powrót uprawnień wymaga jawnego
odnowienia wpisu. PUT używa CAS; niezmienione wpisy zachowują klucz dostępu.
AHP zapisuje visibility keys przy durable client, sprawdza je także przed wysłaniem
ramki i wymusza świeży snapshot po revoke/regrant pomiędzy invalidacjami.
Samo odwołanie zamyka stary strumień kodem 1008; zmiana klucza przy ponownie
ważnym dostępie zamyka go kodem 1013. UI czyści odebrany stan po odwołaniu.
Ponowne nadanie udostępnia całą historię, także z okresu bez dostępu.

Właściciel widzi ostatnie 20 wpisów audytu: rewizja, autor, czas oraz IDs dodanych,
usuniętych i odnowionych odbiorców, bez kopii promptów/poświadczeń. Limit to 32
odbiorców i 1024 zwykłe zmiany na sesję plus 32 zarezerwowane cofnięcia dostępu.
Katalog pokazuje do 256 kandydatów z informacją o ograniczeniu; istniejące wpisy
pozostają widoczne i odwoływalne. Brak automatycznego odnowienia nieaktywnych
wpisów przy zapisie innych zmian.


### Wersje natywnych grantów (migracja25)

project_grants zachowuje aktywność i revision także po odwołaniu. Pierwsze nadanie
ma revision1; no-op nie zmienia revision ani nie publikuje invalidacji. Revoke i
regrant zmieniają epokę. CAS używa None dla nieistniejącego wiersza; CLI
`--expected-revision 0` oznacza brak wcześniejszego grantu. Bez tej flagi lokalne,
zaufane `grant` i nowe `ungrant PROJECT USER` zachowują prostą administrację.
CLI `revoke DEVICE_ID` nadal dotyczy urządzenia. Logical grant ani binding nie
nadają automatycznie natywnego grantu. Ostatnia reprezentowalna revision jest
zarezerwowana na terminalne cofnięcie, którego nie można ponownie aktywować.

Actor i Admission przypinają native stamp obok binding/lease. Stary actor i jego
zgody nie odżywają po revoke/regrant; jawne utworzenie nowej sesji przechwytuje
nową authority. Stary most ACP przypina wszystkie roots, sprawdza je przed startem,
wejściem/wyjściem i przez invalidację/guardian oraz kończy całą grupę procesów.
Guard brokera coordination/vault przypina te same granty/path/device; nie tworzy
cyklu Arc przez przechwycenie całego AppState. Vault stale denial jest trwałe dla
danego tokenu; brak rozszerzenia polityki użycia sekretów ani scope hostowych.

Historia Git wiąże opaque scope z użytkownikiem i epoką. Przygotowany plan mutacji
przechwytuje stamp oraz ewentualny konkretny covering grant WorktreeRemove; nowy
alternatywny grant nie zastępuje starego potwierdzenia. Rewalidacja odbywa się przed
mutacją i odpowiedzią; nie oznacza atomowej transakcji SQL z zewnętrznym procesem.
Zakończonych skutków zewnętrznych nie cofamy i nie ponawiamy automatycznie.

Nowe jobs zapisują native stamp w wewnętrznym Task. Dotychczasowe payloads bez
stampa są zachowane i fail-closed: wymagają nowego przeglądu/nadania zadania;
upgrade nie dorabia im współczesnej authority. Rejestracja pochodnego checkoutu
sprawdza stamp pod tym samym Store lockiem. Linear status delivery sprawdza epokę
przed zewnętrzną mutacją i po odpowiedzi; zmiana podczas odpowiedzi wymaga
ręcznego uzgodnienia wyniku bez automatycznego retry.

Prepared comparison/restore/checks, impact, GitHub metadata i prywatne artefakty
korzystają z tej ochrony. Administracyjne clone/copy mają osobne AdminAuthority,
ponieważ nie konsumują istniejącego native grantu. Legacy sesje/transkrypty
i konfiguracja projektu są chronione przy operacjach Store. Pozostałe procesy
administracyjne korzystają z odrębnego kontraktu bieżącej authority opisanego
poniżej. Nie zamyka to całego E2.3, E4 ani izolacji kont Git.


### Native I/O i lifecycle terminala

NativeIo obejmuje 29 tras FS/explorer/audit/search i starsze trasy Git. Przechwytuje
path, native grant, user i device. Sprawdza tę samą epokę oraz bieżące urządzenie,
OIDC i host account policy przed blocking work i przed odpowiedzią. Permit pozostaje
przy pracy także po anulowaniu HTTP future. Scope zawiera AppState i żyje wyłącznie
przez request/worker; nie należy zapisywać go w rejestrach AppState (cykl Arc).
Prepared workflows przechowują oddzielny NativeProof bez AppState. Przy wznowieniu
odtwarzają scope i sprawdzają dokładny grant, urządzenie i projekt. Checks mają
osobny guardian oraz anulowanie po HTTP drop; permit i registry żyją do zakończenia
workera. Rejestracja comparison projects sprawdza source proof pod tym samym
Store lockiem co tworzenie derived grantów.

Odmowa przed I/O zachowuje 403/401. Jeśli mutacja zaczęła się przed cofnięciem
uprawnień, odpowiedź 409 wskazuje możliwy częściowy skutek: wymagany jest odczyt
stanu, bez automatycznego powtarzania. Błędy mtime, quota i wewnętrznego zapisu
zachowują odpowiednio 409, 429 i 500. Nie jest to atomowa transakcja SQLite z FS.
Upload sprawdza authority także przed tworzeniem katalogów, publikacją attachmentu
oraz zmianą Git exclude; preview ponownie sprawdza ją po kodowaniu odpowiedzi.

Project PTY przypina grant/path/user/device pod jednym Store lockiem. Admin setup
wymaga aktualnej tożsamości administratora i urządzenia. Osobny watchdog reaguje
na invalidacje i co 1 s kontroluje authority. Cofnięcie daje close 1008, bez
kolejnego output/exit. Spawn odbywa się przy sprawdzonym Store cut; relay owner
RAII kończy sesję również po anulowaniu bridge future i przy retained Arc.

Relay-only tryb PTY ustawia O_NONBLOCK, ponawia częściowe zapisy/WouldBlock z
kontrolą closing co 5 ms i ma dwusekundowy budżet zapisu pojedynczej ramki.
Zablokowane wejście nie utrzymuje procesu i permitu po disconnect. Czytnik obsługuje
WouldBlock, bez fałszywego EOF. Desktopowe write/kill pozostają bez zmian.
hard_terminate sprawdza właściciela sesji przez zachowany master PTY i sygnalizuje
potwierdzone grupy shell/foreground. Świadomie odłączone grupy/sesje nie są objęte
tą gwarancją: nieufne workloady nadal wymagają adaptera izolacji systemowej.
Weryfikacja Linux nie oznacza odbioru macOS ani wszystkich rzeczywistych harnessów.

Migracja 26 zapisuje checkout_grant oddzielnie od zatwierdzonego Task/fingerprint.
Rejestracja checkoutu wiąże branch/project/status ze stampem w jednej transakcji.
Runner sprawdza zarówno źródłowy, jak i pochodny grant oraz oczekiwany serwerowy
path. Revoke/regrant checkoutu nie przywraca starego joba. Stare checkouty bez
stampa pozostają nieautoryzowane, zamiast otrzymywać współczesny pin przy upgrade.

### Publikacja artefaktów i administracyjnych checkoutów

Prywatny upload używa create_new i staging z RAII cleanup. Publikacja bez nadpisania
oraz wpis metadata mają wspólny cut z NativeProof. Błąd po publikacji oznacza
niepewny skutek 409; nie wyzwala automatycznego retry. Download ma limit 32 MiB,
permit i ponowną kontrolę proof po odczycie/kodowaniu oraz świeży wpis metadata.
Zmiana metadata wymaga właściciela i świeżej authority w tej samej blokadzie Store.
Publiczny token udostępnienia jest niezależną capability; samo cofnięcie native
grantu nie cofa świadomie utworzonego publicznego linku. Jawne revoke linku
pozostaje oddzielną operacją. Odczyt publiczny ma limit i permit oraz ponownie
sprawdza ten sam token i wiersz artefaktu po pobraniu bajtów. Revoke podczas
odczytu powoduje 404 bez zwrócenia zawartości.

Clone/copy przygotowują prywatny staging poza blokadą Store. AdminAuthority zawiera
wyłącznie identyfikatory użytkownika i urządzenia, a przy publikacji sprawdza ich
bieżący stan, rolę, OIDC i host policy. Copy wiąże również urządzenie rozpoczynające
operację. Rename bez nadpisania, rejestracja projektu i grant są wykonywane przy
świeżej kontroli. Późny błąd SQL nie usuwa już opublikowanych danych; zwraca
niepewny wynik i wymaga odczytu stanu. Pozostałe administracyjne instalacje/probes
nie są objęte tym kontraktem clone/copy.

### Dowód crash/relaunch runtime Linux

Suite runtime uruchamia rzeczywisty binarny relay z produkcyjnym bubblewrap,
kontrolowanym ACP fixture i trwałą SQLite. Zabija tylko serwer po przyjętym
prompcie, wykonanym efekcie i oczekiwaniu na zgodę; druga tura pozostaje w kolejce.
Przed cleanupem sprawdza zakończenie rzeczywistych potomków przez PID/start time.
Restart tej samej bazy daje Interrupted/Unknown, bez ponownego dispatchu kolejki.
AHP reconnect zwraca snapshot nowego bootu; retry starego żądania zwraca trwałe
potwierdzenie, a stary nonce nie daje zgody. Nowa jawna sesja wykonuje pojedynczy
nowy efekt. Szczegóły odtwarzania środowiska opisuje [runtime-checks.md](runtime-checks.md).
To dowód jednego rzeczywistego punktu awarii Linux; okna dispatching przed pipe
oraz consumed permission przed flush mają oddzielne testy jednostkowe. Nie oznacza
odbioru macOS, prawdziwego dostawcy ani operatorskiego reconcile/resume.

### Legacy sesje i konfiguracja projektu

NativeIo::capture_in_store przypina projekt/grant/urządzenie pod istniejącą blokadą
Store. Właściciel legacy sesji i jej project ID są rozwiązywane w tym samym cut,
a następnie sprawdzane ponownie razem z proof przy odczycie transkryptu, revision,
CAS save, delete i takeover. Znane ID nie zastępuje natywnego grantu. Brak lub
obcy właściciel daje 404; właściciel bez grantu dostaje 403. Revoke/regrant nie
przywraca rozpoczętej operacji. Retarget sesji nie pozwala użyć proof starego projektu.

Sessions/context/settings wykonują kontrolowany odczyt pod Store i sprawdzają
proof przed odpowiedzią. Settings PUT weryfikuje aktualnego administratora przy
zapisie oraz przed odpowiedzią, zamiast ufać roli wyekstrahowanej wcześniej.
Delete/takeover mają porządek Store→leases i używają nieblokującego try_send;
ścieżki ACP nie trzymają leases podczas ponownego wejścia do Store. Utrata dostępu
po rzeczywistej mutacji daje niepewny wynik 409; takeover bez zmiany nie oznacza
nieistniejącego skutku. Globalne wyłączenie przechowywania transkryptów nadal działa.

Normalny exit PTY przechowuje status, wyłącza ponowny odbiór exit i dalej przekazuje
wyjście aż do EOF czytnika lub stałego dwusekundowego deadline. Dopiero wtedy
kończy sesję i raportuje status. Chroni to końcowe bajty przed wyścigiem niezależnego
waitera z czytnikiem nonblocking. Aktywność nie przedłuża deadline; revoke,
disconnect i błędy nadal przerywają pracę. Każdy output ponownie sprawdza authority.

### Bieżąca authority operacji administracyjnych i usage

Management zapisuje w Running wyłącznie wartościowy AdminAuthority urządzenia
zlecającego. Job/output/cancel wymagają tego samego aktywnego urządzenia i użytkownika,
aktualnej roli admina, OIDC i host policy. Worker pozostaje serwerowy po rozłączeniu
HTTP; niezależny guardian sprawdza authority po invalidacji i co100ms. Start procesu
odbywa się przy świeżej kontroli Store. Limit pracy wynosi15min; zbierany output
ma32KiB plus krótki komunikat zakończenia. Niepewny rezultat nie jest ponawiany.
Logowanie hostowe dodatkowo wiąże wymóg relay zawierającego wyłącznie adminów;
dodanie membera po starcie przerywa operację. Nie rozszerza to uprawnień członków.

Lider grupy jest obserwowany przez waitid WNOWAIT, bez reaping przed sprzątaniem
potomków. Guardian i Drop kończą potwierdzoną grupę przed child.wait; po reaping
nie wolno sygnalizować zapamiętanego PID. Sprawdzenie własności odrzuca ECHILD,
a serializacja zapobiega ponownemu groupkill po zakończeniu.

Usage oraz agregat billing wymagają świeżego admina również przy cached response
i publikacji cache. Własny billing PUT pozostaje dostępny użytkownikowi jego
urządzenia, sprawdzanemu w tym samym Store cut co zapis. Requestdrop/deadline
sygnalizują cancellation sondy; permit pozostaje przy workerze do cleanup.
ProbeProcess ma nonblocking pipes, fixed deadline, ograniczony rozmiar i RAII
kill/reap. Credentials fallback otwiera O_NONBLOCK i dopuszcza wyłącznie regular
file z limitem64KiB. Security probe ma3s, Codex6s, HTTP Claude15s/64KiB. Trwającego
zewnętrznego requestu nie da się cofnąć; po odwołaniu nie publikuje się wyniku.
To nie jest odbiór realnego keyringu/providerów ani izolacja zdalnych hostów.

### Różnicowy test adaptera AHP browsera

Wspólne traces wejściowe są wykonywane przez niezmodyfikowany, przypięty reducer
Rust SDK. Checked-in wynik ma54 stany w6 traces; zwykły test Rust weryfikuje jego
pochodzenie, a testy Node porównują projekcję browsera po każdym kroku pięciu
traces. Obejmują metadata stream/ready/content/completion, dwie jednoczesne zgody,
revoked receipt, terminal tools, NoOp oraz modifiedAt/status flags. Snapshot bez
interactivity używa domyślnego full jak SDK, ale zapis nadal wymaga jawnych capability.

Jeden trace jawnie dokumentuje sprzeczność przypiętego SDK z dokumentacją partial
queue reorder: SDK sortuje pominięte ID, a dokumentacja i kolejka wykonawcza
zachowują ich kolejność. Test nie przedstawia tego jako parity. Backend już
normalizuje przyjęty partial reorder do pełnej kolejności canonical action,
bez patchowania vendora i bez zmiany kolejności wykonania obiecanej przez API.
Regresje porównują SQL, live SDK, snapshot i durable retry po zmianie kolejki.
Szósty trace obejmuje starsze strony, overlap/dedupe, cursor i bieżącą turę.
Aggregate status i pozostały conformance E3 nadal otwarte.

### Okno historii AHP i reconnect (migracja27)

SubscribeView.turns jawnie prosi o ostatnie0–64 ukończone tury. Brak tego pola
zachowuje wszystkie retained turns albo zwraca jawny błąd budżetu; serwer nigdy
nie zamienia domyślnego widoku na cichy tail. Snapshot zawiera bieżącą turę i kolejkę,
a tail dostosowuje się do wspólnego limitu900KiB. Zamrożona historia używa Arc;
łączne limity połączenia wynoszą8MiB/512tur/32windows i są sprawdzane przy dodawaniu
kolejnych buforów. Kursor wygasa po5min; niezależny tick usuwa też bufor idle peer.

fetchTurns sprawdza bieżącego user/device, visibility read-key oraz losowy cursor
właściwy dla tego połączenia i kanału. Strona ma do32tur/512KiB, oldest-first.
Najpierw wysyłane są zaległe canonical actions, następnie requester-only
chat/turnsLoaded z nowym serverSeq i dopiero wynik RPC{}. Page payload/cursor nie
wchodzi do globalnego canonical journalu ani cudzych połączeń. Sekwencja jest
atomowo rezerwowana w clock; luki wynikające z nieobserwowanych kanałów i view
notifications są dozwolone. Ponowiony, obcy lub wygasły cursor daje InvalidParams.

W interactive_ahp_clients.views zapisany jest wyłącznie requested tail size,
oddzielnie od security visibility pins. Reconnect paged subscription zawsze
zwraca świeży ograniczony snapshot z nowym kursorem, również zwykłemu SDK klientowi.
Przeglądarka scala starsze turns z dedupe, zachowując live turn/queue. Nie pobiera
kolejnych stron automatycznie; po błędzie kursora oferuje jawne odświeżenie.
Selection generation odrzuca spóźnione A→B→A snapshoty/pages. Revoke usuwa
chronione strony i nie pozwala odżywić starego frozen cut po revoke/regrant.

Shared chat/session/list snapshots jawnie mają interactivity read-only i wyłączone
control flags. Ta projekcja jest per-user i nie zmienia canonical state właściciela.
Pojedyncza ukończona tura lub aktywny payload większy niż dozwolona ramka nadal
wymaga zewnętrznych content resources w E3.7; obecnie kończy się jawnym błędem.
REST create session z Idempotency-Key korzysta z trwałego przyjęcia opisanego
poniżej. Legacy request bez klucza oznacza oddzielny zamiar utworzenia sesji;
klient potrzebujący bezpiecznego retry musi zachować tożsamość zamiaru.

### Trwałe przyjmowanie sesji REST (migracja28)

POST interactive-sessions przyjmuje opcjonalny Idempotency-Key (16–128 ASCII
letters/digits/underscore/hyphen, jedna wartość nagłówka). Przy kluczu bieżące
user/device/OIDC/host policy, logical grants, binding/native grant i profil
wyznaczają fingerprint. Ledger(device,key) zapisuje owner/workspace/project,
fingerprint i pierwotny publiczny Session receipt. IMMEDIATEtransaction obejmuje
session, lease, projections i ledger; późny błąd SQL cofa wszystkie te zmiany.
Ledger ma limit1024global/128device i nie usuwa akceptacji dla odzyskania miejsca.

Dokładny retry zwraca pierwotny receipt oraz replayed=true, z leaseNone. Nie startuje
procesu, nie rejestruje actora ani nie przejmuje lease, także po recovery. Lookup
poprzedza limity nowych wykonań/session/ledger i sprawdzenie wyłączenia runtime;
pozostaje jednak ograniczony pool HTTP/blocking I/O, który może zwrócić429.
Ten sam klucz z innym payloadem lub epoką authority daje409. Własność/uprawnienia
są weryfikowane na nowo; stara akceptacja nie ożywia cofniętego dostępu.

Response202 zawiera session oraz admission.replayed/session_is_acceptance_snapshot;
Idempotency-Replayed header musi odpowiadać metadata. Publiczny receipt nie
zawiera runtime/authority i nie odtwarza ich z JSON. Historyczne phase/epoch/sequence
opisują pierwotne przyjęcie, a bieżący AHP/GET opisuje aktualną sesję. Po restarcie
receipt może nadal mówić active, gdy bieżąca sesja jest Interrupted/Unknown.
Legacy unkeyed create używa wewnętrznego jednorazowego klucza dla atomowości,
ale każde takie żądanie pozostaje oddzielnym zamiarem zgodnie z API v1.

Nowy browser helper zamraża UUID/body/scope w pamięci przy jawnym zamiarze i nie
ponawia automatycznie POST. Wysyła przypięte X-Dunlin-Expected-User/Device; serwer
sprawdza tę opcjonalną parę przed przyjęciem i ponownie w workerze. Zmiana cookie
pomiędzy preflight a POST nie przenosi zamiaru pod inną tożsamość. Niepewny wynik
zachowuje klucz/treść do jawnego retry. Helper wymaga zatwierdzonego profilu;
produkcyjny formularz i katalog modeli/trybów pozostają E3.7/E5.1. Test Chromium
uruchamia rzeczywisty helper source przeciwko produkcyjnym routes, bez twierdzenia,
że istnieje formularz lub że nieużywany moduł znalazł się w bundle aplikacji.

Service wykonuje dwa świeże Storecuts z permit acquisition pomiędzy nimi, ponieważ
connection_permit bierze Management i obowiązuje Management→Store. Drugi cut
ponownie sprawdza ledger/fingerprint przed commit i rejestracją. Runtime handle,
settings/catalog oraz fallible locks są przygotowane przed commit; Store/actors
zostają zwolnione przed spawn/ExitGuard. Crash po commit nie daje prawa do
ponownego uruchomienia: przy kolejnym starcie recovery oznacza niepewny stan.
Rzeczywisty binary+bubblewrap SIGKILL test sprawdza originalreceipt/no-secondpeer
po restarcie tej samej DB oraz działanie oddzielnego świeżego zamiaru.
