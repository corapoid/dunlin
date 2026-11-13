# E4.4 — plan trwałego wykonania na workerze

Status: **pion w realizacji, nieukończony**. Operator-local policy snapshot,
bounded execution DTO v3, ledgery35/3 oraz worker runtime są zaimplementowane.
Wybrane testy ledgerów/protokołu i rzeczywiste Linux worker/guardian/transport
PASS dla snapshotów wskazanych w [progress.md](progress.md). Pełny wewnętrzny
przekrój Controller service→TLS/WSS→ACP→trwały ACK PASS dla snapshotu e3d3429c,
z jednym procesem/efektem po identycznym retry. Nowsze cleanup/regression changes
oraz pełna produktowa sesja Remote nie są tym wynikiem odebrane. Dalsze sekcje
opisują wymagany kontrakt; nie każda z nich jest już odebrana. E4.4 i publiczny
Remote pozostają otwarte. Wyniki i nieudane próby zapisano w progress.md.

## Zakres pierwszego pionu

Jedna jawnie zatwierdzona próba (`Attempt`) wykonuje jeden ograniczony ACP turn:
rezerwacja kontrolera → trwała komenda → worker admission → rzeczywisty proces
w sandboxie → trwały wynik → potwierdzenie kontrolera. Sam receipt, heartbeat
lub symulowany proces nie wystarcza do odbioru tego pionu.

Pierwszy konsument może być wewnętrznym `ControllerExecutionService`, z pełnym
fixture obu ról i rzeczywistym procesem. To nie jest jeszcze produktowe
wykonanie Remote. Istniejący `CreateRequest::Remote` zachowuje501 do czasu
podłączenia admission i kolejki oraz zachowania właściwego kontraktu sesji.
Nie tworzymy osobnego browser prompt RPC zastępującego AHP. Jeden turn nie
uprawnia do deklarowania obsługi całej zdalnej sesji wieloturowej.

Początkowo Human only, stała allowlista harnessów, published agent version,
jawny model/mode i bounded plaintext input. Nieobsługiwane MCP, terminal tools,
A2A i human approvals mają jawną odmowę; worker nie odpowiada za człowieka.
Poświadczenia modeli wymagają odrębnej lokalnej konfiguracji/scopingu. Nie
wolno odziedziczyć przypadkowych hostowych credentials dla nowego principalu.

## Istniejące fundamenty i ich granice

- [worker.rs](../crates/relay-protocol/src/worker.rs) i
  [worker/exports.rs](../crates/relay-protocol/src/worker/exports.rs): bounded
  JSON64KiB, diagnostyczny v1 i metadata v2; brak poleceń wykonania.
- [worker_channel.rs](../apps/server/src/worker_channel.rs) oraz
  [worker_channel/exports.rs](../apps/server/src/worker_channel/exports.rs):
  prywatny WSS, reverse nonce proof, boot/channel fences i bounded korelacja.
- [store/worker_channels.rs](../apps/server/src/store/worker_channels.rs):
  trwały control-channel lease. Nie jest execution lease ani pozwoleniem FS.
- [worker_client.rs](../apps/server/src/worker_client.rs): jeden właściciel
  socketu, heartbeat i metadata jobs; brak reconnect/recovery wykonania.
- [worker_exports.rs](../apps/server/src/worker_exports.rs): osobna prywatna
  SQLite, dokładny controller/workspace/host/enrollment scope, export/grant CAS,
  tombstones i SQL→FS→fresh SQL. Grant `Execute` nie ma jeszcze wykonawcy.
- [store/remote_bindings/resolver.rs](../apps/server/src/store/remote_bindings/resolver.rs):
  dokładny ref, bieżący użytkownik i własny worker Read; wynik to obserwacja.
- [interactive.rs](../apps/server/src/interactive.rs),
  [session-runtime](../crates/session-runtime/src/lib.rs) i
  [agent.rs](../crates/domain/src/agent.rs): istniejące admission, journal,
  kolejka, profile i lokalny ACP. Nie są automatycznie runtime workera.

Ograniczenia sieci pozostają takie jak w [worker-channel.md](worker-channel.md):
wyłącznie prywatny Serve WSS przez zweryfikowany transport. Bez LAN, SSH,
publicznego fallbacku, cookies do workera i zaufania do samego `.ts.net`.

## Najpierw: rzeczywiście zastosowane ustawienia

Obecny export przypina `settings_digest` podany przez operatora. Nie przechowuje
konfiguracji, której executor rzeczywiście użyje; opisuje to
[remote-bindings.md](remote-bindings.md). Sam zgodny token nie pozwala uruchomić
procesu.

Proponowane rozszerzenie operator-local:

1. Operator dostarcza ograniczony dokument ustawień lub jawny lokalny plik.
   Żaden jego path nie pochodzi z kontrolera/browsera.
2. Worker czyta regularny plik z limitem, parsuje do walidowanej konfiguracji,
   odmawia nieobsługiwanych dodatkowych workspace roots i niedozwolonego env.
3. Hash dokładnie określonej reprezentacji musi odpowiadać zatwierdzonemu
   digestowi. Zamrożono SHA256 dokładnych bajtów pliku (maksymalnie32KiB),
   bez normalizacji JSON/newline. Istniejące metadata-only digesty pozostają
   zgodne; nie uzyskują automatycznie policy ani wykonania.
4. Ustawienia faktycznie użyte do budowy procesu są tym samym snapshotem.
   Zmiana polityki/exporta wymaga rebind i ponownego grantu, bez automatycznej
   aktywacji dotychczasowych metadata-only konfiguracji.
5. Lokalny allowlist harnessów i rzeczywisty adapter sandboxa ograniczają
   konfigurację dalej. Raport capabilities jest danymi, nie dowodem gotowości.

## Proponowany protokół wykonania

Nowy dokładnie negocjowany subprotocol, roboczo `dunlin.worker.v3`/Hello3.
V1/v2 nadal nie przyjmują komend wykonania; brak downgrade do metadata/local.
Wersja raportu HostCapabilities pozostaje odrębnym kontraktem.

Minimalne rodziny komunikatów:

| Komunikat | Znaczenie |
| --- | --- |
| `ExecuteAttempt` | Dokładny attempt/command ID, fingerprint, export i grant pins, principal, agent ref/body/digest, input i budżety |
| `AttemptReceipt` | Trwała pierwotna akceptacja/odmowa; nie ponawia procesu |
| `RenewExecutionLease` / ack | Dokładny execution epoch i rosnąca sekwencja; brak ponownego admission |
| `CancelAttempt` / receipt | Trwały intent, potem potwierdzenie rzeczywistego zatrzymania |
| `AttemptOutcome` / `OutcomeAck` | Bounded trwały wynik i potwierdzenie jego zapisu |
| `ReconcileAttempt` | Odczyt istniejącego stanu; nie wznawia ani nie przenosi lease |

Każda komenda przypina bieżący kanał, controller/worker boot i execution fence.
DTO mają deny-unknown, limity IDs/digestów/liczb/frame oraz zamknięte kody błędów.
Nie przesyłamy argv, root paths, dowolnego URL ani peerowej flagi `verified`.
Dokładne pola, nazwy i limity należy zamrozić wraz z testami parsera przed
równoległą implementacją obu ról.

Pierwszy wynik może zawierać tylko bounded raport/status. Nie wolno przecisnąć
większego raw ACP frame przez64KiB wire albo deklarować pełnego streamingu AHP.
Streaming/chunking i duża treść wymagają własnego kontraktu przed aktywacją.

## Trwałość i transakcje

### Kontroler — nowa migracja głównej Store

Proponowane logiczne tabele:

- `worker_attempts`: owner/device, workspace/project, dokładny binding, host
  epochs, export/grant pins, published agent/policy/input digest, attempt status,
  execution fence oraz jawny unknown reason.
- `worker_command_outbox`: command ID, immutable fingerprint/payload, attempt,
  stan doręczenia i pierwotny receipt. Brak osobnego commitu po enqueue.
- `worker_result_inbox`: unique attempt/event identity, fingerprint i wynik;
  dedupe wyniku jest atomowe z jego zastosowaniem.

Rezerwacja próby i komenda outbox powstają w jednym fresh SQL cut. Przy integracji
z sesją obejmuje on również odpowiednią rezerwację turn/journal; późny błąd SQL
nie może pozostawić uruchamialnej komendy bez zatwierdzonej próby.

### Worker — migracja prywatnej WorkerStore

ACL/export i admission muszą korzystać z tej samej SQLite. Osobna execution DB
bez wspólnej transakcji zostawia lukę między sprawdzeniem grantu a rezerwacją.
Proponowane tabele:

- `execution_inbox`: unique command identity, fingerprint, immutable receipt;
- `execution_attempts`: dokładny scope/pins, queue position, phase, start intent,
  execution epoch/boots, lease counters, outcome i recovery reason;
- `execution_outbox`: trwałe receipts/outcomes do potwierdzenia przez kontroler;
- `execution_policy`: zatwierdzony snapshot ustawień/polityki związany z exportem.

Nazwy i numery migracji są propozycją. Migracja odbywa się lokalnie podczas
operator/startup, nie jako efekt otrzymania DescribeExport. Istniejący
`open_existing` nie może zacząć tworzyć katalogów lub migrować pod peer request.

Worker SQL→FS poza blokadą→fresh SQL obejmuje aktualny własny Execute grant,
export/settings identity, dedupe i rezerwację. Zapis start/dispatch intent jest
przed spawn/ACP write. Niepewnego pipe write nie ponawiamy.

Identyczna komenda zwraca canonical receipt, bez nowego lease/procesu. Ten sam ID
z innym fingerprintem jest konfliktem. Historyczny receipt nie stanowi authority
renewal. Politykę odczytu receipts po revoke trzeba zdefiniować osobno od admission.

## Dokładne authority cuts

Kontroler sprawdza przed outbox commit, dispatch i każdym renewal:

- aktualnego właściciela/urządzenie/OIDC/host-account policy;
- bieżące logical ViewProject/CreateSession i captured revisions;
- dokładny binding ref, host/enrollment/credential, politykę drain;
- published agent version/digest i dozwolony harness/model/mode/budżety;
- bieżący proved kanał i oba boot IDs.

Worker nie traktuje przesłanego AuthorityStamp jak lokalnego grantu. Zaufanie do
bieżącej decyzji kontrolera pochodzi wyłącznie z uwierzytelnionego kanału, a
lokalny operator nadal musi nadać własnemu principalowi Execute na dokładny export.
Nie pożyczamy grantu administratora zapisującego binding. Nie mapujemy principalu
kontrolera na root lub dowolnego użytkownika OS.

Fresh worker cut przed admission, renewal, spawn i dalszym I/O weryfikuje grant,
export, settings, sandbox policy oraz execution fence. Regrant/rebind nie odnawia
starej próby. Zmiana reported capabilities nie rozszerza lokalnej polityki.

## Execution lease, guardian i recovery

Execution lease jest oddzielny od istniejącego10s control-channel lease. Ma własny
monotoniczny deadline, epoch i rosnący renewal sequence. Duplikat renewal nie
przedłuża czasu. Deadline nie wykracza poza pierwotny maksymalny budżet próby.
Remote wall clock nie jest lokalnym zegarem bezpieczeństwa.

Limits są rezerwowane transakcyjnie między procesami. Pierwszy pion powinien
mieć maksymalnie jedną aktywną próbę na export i ograniczony globalny parallelism.
Unknown nadal blokuje konfliktujący zasób, dopóki nie udowodniono zatrzymania.

Kontrakt obu ledgerów rozdziela trwały wynik od potwierdzenia zatrzymania.
Worker po start-intent zachowuje reservation także dla Completed/Failed/Cancelled/
Unknown, aż zaufany lokalny actor/operator dostarczy dowód zakończenia całej
namespace/grupy. Sam wynik ACP, SQL terminal, restart lub ACK nie jest takim
dowodem. Pristine Accepted, dla którego nigdy nie utrwalono start-intent, może
zwolnić slot atomowo bez dowodu procesu, bo nie przyznano zezwolenia na start.

Terminal record na wire (outbox, reconcile i retry Accepted z terminalnym record)
jest dostępny dopiero po potwierdzeniu zatrzymania. Niepotwierdzony proces daje
zamknięty błąd Unavailable/Conflict; nie wolno udawać braku rekordu ani redagować
plaintext pod oryginalnym hashem. Odczyt wyniku wymaga nadal świeżego własnego
Human Read na tym samym export revision/capture/settings. Dokładny ACK może
zwolnić outbox po świeżym channel/scope nawet po Read revoke, lecz nie usuwa
lifetime inbox ani reservation. Inbox zachowuje niezmienny wynik do reconcile.

Wewnętrzny kontrakt v3 wymaga teraz odpowiedzi `OutcomeAcknowledged` po
trwałym SQL ACK workera, również przy identycznym ponowieniu. Sam zapis ramki
do socketu nie potwierdza usunięcia outbox: pierwsza ramka lub odpowiedź mogła
zaginąć, a zadanie SQL mogło się nie powieść. Kontroler czyści `ack_pending`
wyłącznie po dokładnej odpowiedzi związanej z bieżącym kanałem, niezmienną
identity, oryginalnym attempt fence oraz outcome ID/digest. Prywatny cache
potwierdzeń żyje w aktualnym, opaque slocie v3 i ma limit 32; wymiana slotu lub
ewikcja oznacza bezpieczne ponowienie tego samego negatywnego ACK.
`OutcomeAckRejected` przenosi te same identyfikatory i zamknięty kod błędu,
nie tworzy potwierdzenia i nie trafia do korelatora Renew/Cancel. Oba wyniki
ACK mają osobny bounded korelator; historyczny attempt fence jest tu wyłącznie
uprawnieniem do negatywnego cleanup, nigdy do odnowienia lub wykonania pracy.
Nie ma AckAck: zasób workera jest zwolniony przed odpowiedzią, a utrata odpowiedzi
pozostawia kontrolerowi trwałe `ack_pending` do jawnego identycznego retry.

Kontroler zwalnia swoją reservation dopiero po uwierzytelnionym, fenced terminal
reconcile spełniającym powyższą semantykę workera; nie dodajemy peer boolean
„process stopped”. Terminal Unknown oznacza niepewne skutki, choć namespace musi
być już potwierdzona jako zatrzymana przed dostarczeniem terminal record.
Callbacki w testach ledgerów są tylko zaufanymi atrapami tej granicy, bez dowodu
kernel containment. Prawdziwy runtime musi uzyskać rzeczywisty stop proof przed
wywołaniem lokalnego stop-confirmation i publikacją terminalnego wyniku.


Guardian działa niezależnie od blocked stdin/stdout, DB/FS i klienta browsera.
Sprawdza SQL epochs, FS identity, kanał oraz monotoniczny lease. Kończy rzeczywistą
grupę procesu i potomków przed reap lidera; nie sygnalizuje odzyskanego z DB PID
bez dowodu aktualnej własności.

In-process async watchdog nie przeżywa SIGKILL supervisora. Potrzebny jest
potwierdzony mechanizm kernel parent-death/containment lub niezależny guardian
z prywatnym pipe i deadline. **Dokładny mechanizm i gwarancje Linux/macOS są
bramką research/probe**, nie potwierdzonym tutaj faktem. Test ma zabić supervisora
i wykazać zatrzymanie rzeczywistego potomka. Sama poprawna flaga argv nie wystarcza.

Po restarcie zmieniają się boots; stare active/dispatching/consumed bez dowodu
wyniku przechodzą w Unknown. Reconnect służy reconcile i odtworzeniu receipts,
nie automatycznemu nowemu dispatch ani odnowieniu starego lease. Nowa próba
wymaga jawnego nowego zatwierdzenia po rozliczeniu niepewnego wyniku.

## Budżety i backpressure

Przed implementacją zamrozić limity payloadu, liczby/rozmiaru inbox/outbox,
kolejki, równoległości i czasu. Wymagane właściwości:

- brak usuwania zaakceptowanej tożsamości dedupe po TTL;
- bounded lifetime ledger/tombstones, fail-closed po capacity;
- rezerwa na terminal outcome/revoke mimo pełnego zwykłego logu;
- bounded pending commands i jeden właściciel WSS socketu;
- heartbeat/renewal nie czekają na FS ani strumień output;
- permit trwa do końca rzeczywistego blocking job również po timeout/drop;
- brak automatycznego replay mutacji po utracie ACK lub wyniku.

## Testy i kolejność prac

1. Zamrozić DTO, state machine, fingerprint i actual-settings contract.
2. Równolegle: controller migration/outbox; WorkerStore admission/inbox/outbox;
   protocol parser tests. Bez uruchamiania procesów przez niedokończony dispatcher.
3. Po stable API: worker actor, guardian i v3 multiplex. Stały harness, model/mode
   potwierdzone przez ACP, jeden turn i bounded raport.
4. Złożyć pion na prawdziwym TLS/WSS fixture obu ról oraz realnym sandbox/process.
5. Dopiero potem włączyć konsumenta Remote w istniejącym admission/queue/AHP.

Obowiązkowe failure tests:

- crash kontrolera po commit outbox przed wysłaniem;
- utrata ACK po worker admission/spawn/prompt write, bez drugiego procesu;
- crash workera przed/po każdym SQL/pipe cut i Unknown zamiast replay;
- duplikat ID z innym payloadem, reopen i równoległe admissions dwóch connections;
- device/OIDC/project grant/export/Execute revoke i ABA podczas blocked I/O;
- replacement kanału, oba restarty, monotoniczna expiry i spóźnione renewal;
- silent child, zachowane pipe FD i SIGKILL supervisora;
- pełna kolejka/outbox/dyskowy budżet, terminal reserve i brak wycieku permitów;
- odmowa hosta/ref/profile/settings/FS bez fallbacku na inny cel;
- dedupe wyników wraz z controller journal/projection w jednej transakcji.

To testy do napisania i wykonania, nie wyniki. Kontrolowane ACP fixtures nie
zastępują realnych modeli/credentials, macOS, dwóch hostów ani operator acceptance
z E4.7/E10. Po złożeniu jednego turn nadal należy jawnie wyliczyć brakujące
możliwości E4.4 i pełnej sesji zamiast oznaczać cały etap jako gotowy.

## Wewnętrzny codec fingerprintów pierwszego pionu

Zaimplementowany helper [worker_execution.rs](../apps/server/src/worker_execution.rs)
koduje i hashuje dane. Nie aktywuje negocjacji v3 ani wykonania; samo uzyskanie
hasha nie jest walidacją DTO, dowodem opublikowania profilu czy uprawnieniem.
Kontroler i worker muszą niezależnie sprawdzić dokładne wartości oraz bieżące
fences i trwały stan przed działaniem.

Algorytm jest następujący, z lowercase hex SHA-256 wyniku:

- Komenda: sklonować typed `ExecutionCommand`, wyczyścić wyłącznie
  `identity.fingerprint` do pustego stringa, zakodować `serde_json::to_vec`
  bezpośrednio z typed struct, a następnie hash z konkatenacji bajtów
  `dunlin.worker.execution.command.v1\0` i zakodowanego JSON. `\0` oznacza jeden
  bajt NUL. Wszystkie pozostałe pola, w tym channel/boot IDs, immutable identity,
  cały descriptor, principal/grant/export revisions, profil/definicja, input
  i requested budgets, pozostają w preimage.
- Profil: SHA-256 dokładnego `serde_json::to_vec(&AgentDefinition)`, bez prefiksu.
  To istniejący codec `Store::publish_agent`, którego `serde_json::to_string`
  daje te same bajty UTF-8. Executor porównuje wynik z przypiętym published
  `AgentProfileRef.digest`; nie wystarcza poprawny format lowercase hex.
- Wynik: sklonować typed `ExecutionOutcome`, wyczyścić wyłącznie `digest`,
  zakodować `serde_json::to_vec` i zahashować prefiks bajtowy
  `dunlin.worker.execution.outcome.v1\0` oraz ten JSON. Outcome ID, terminal
  status i pełny bounded plaintext są częścią preimage. Durable ACK musi
  dodatkowo zgadzać się z zachowanym original attempt fence i identity w SQL.

Kolejność pól typed struct i kolejność elementów tablic jest częścią tego codec.
Nie hashuje się przepisanej `serde_json::Value`, dowolnego JSON odebranego z sieci,
JSON z formatowaniem ani danych z pominiętymi polami innymi niż opisane
self-digest fields. Zmiana definicji codec wymaga jawnej migracji/wersjonowania;
nie wolno potajemnie przeliczać historycznych komend jako nowych intencji.
Helpery nie pobierają danych, nie czytają plików i nie odnawiają lease.

### Probe subreapera — 2026-11-12

[Notatka i źródła pierwotne](evidence/2026-11-12-worker-subreaper/design-note.md)
opisują PASS hipotezy na rzeczywistym jądrze w dwóch małych sandboxach 128 MiB:
SIGKILL guardian przed/po init-pidfd handoff oraz po starcie cichego potomka z FD.
Końcowy dowód braku dzieci wymaga `P_ALL/WNOWAIT -> ECHILD`; puste `/proc/children`
nie wystarcza. Integracja usługi Rust pozostaje niezweryfikowana. Jednoczesna
śmierć Main i guardian nadal wymaga cgroup/external owner i osobnego odbioru.
E4.4 oraz E10 pozostają otwarte.

Transport v3 ma wspólny limit 1024 odebranych frames/minutę w obu rolach,
liczony zegarem monotonicznym. Legalna execution lease1s wymaga świeżego
Describe przed każdą renewal; limit v1/v2=120 nie mieści takiego ruchu. V1/v2
zachowują120. Pozostałe granice nie rosną: frame64KiB, execution queued/pending8,
worker jobs8, producer events32. Zwiększenie limitu ramek nie włącza kolejnych
wykonawców ani nie zastępuje osobnych admission/lease/authority checks.

## Następna integracja: sesja Remote w istniejącym AHP

Po odbiorze wewnętrznego pionu pozostaje produktowa sesja Remote. Nie dodajemy
osobnego browser prompt RPC. Istniejące tworzenie sesji z Idempotency-Key i
AHP dispatchAction muszą używać typed adaptera Local/Remote, bez podszywania
Remote pod NativeCheckoutAuthority ani lokalny harness kontrolera.

Wymagane kolejne granice transakcyjne:

1. Trwałe przypięcie session target oraz turn→attempt: session/lease epoch,
   turn ID, pierwotny binding/profile/fingerprint, komenda i worker receipt/fence.
   Utworzenie sesji nie wysyła Execute. Przyjęcie Remote wymaga opublikowanego
   profilu i rzeczywistego Describe poza Store, potem świeżego końcowego SQL cut.
2. Claim queued turn i rezerwacja jego komendy/mappingu w jednej transakcji.
   Obecne Journal::claim_next oraz admission kontrolera otwierają własne TX;
   potrzebują wariantów in_transaction i powiadomień dopiero po COMMIT.
   Sekwencja claim→Service.submit w dwóch commitach nie spełnia tego wymagania.
3. Canonical terminal controller inbox oraz dokładny mapped journal/AHP outcome
   muszą zostać zapisane atomowo przed worker ACK. Trwały marker dedupe zapobiega
   ponownej projekcji. Unknown blokuje kolejną turę także po recovery/cancel;
   sam optymistyczny stan browsera nie zwalnia rezerwacji wykonania.
4. Limity input/capabilities muszą być sprawdzane przy akceptacji kolejki,
   z trwałym AHP rejection dla tego samego originu. Remote nie dziedziczy zgód
   narzędziowych lokalnego actora. Cancel/reconcile używają tej samej tożsamości;
   recovery nigdy nie tworzy nowego Execute, actora ani odnowienia lease.

Obecny worker rozpoczyna nowy proces i sesję ACP dla każdej komendy, zna tylko
bieżący input/profil i zwraca bounded final plaintext. Nie zapewnia trwałej
rozmowy ACP, streamingu, human approvals ani dużych zasobów. Pełna integracja
sesji wymaga rozszerzenia tego kontraktu i runtime; nie wolno przedstawić serii
niezależnych komend jako istniejącej wieloturowej rozmowy. Remote501 pozostaje
przed odbiorem tych warunków.

Odbiór wymaga rzeczywistego browser create→AHP queue/reconnect/revoke przez
TLS/bwrap/ACP oraz crash cuts przy claim/admit/Execute/terminal-journal/ACK:
brak drugiego efektu, utraty zatwierdzonego wyniku i przejścia kolejki przez
Unknown. Dotychczasowe lokalne immutable admission receipts zachowują zgodność.

Szczegółowy projekt trwałej rozmowy, oddzielonej od terminalnego jednego turnu
v3: [remote-session-plan.md](remote-session-plan.md). To wymagania następnej
integracji i jej testów; dokument nie oznacza implementacji protokołu v4.
