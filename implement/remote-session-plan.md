# Trwała zdalna sesja ACP — projekt następnego przekroju

Stan: schema37/recovery mają pełny PASS snapshot4ee7ae75 (539 relay,
67 Journal,14 kernel). Snapshotde20fff5 ma540 relay/14 kernel PASS i obejmuje
kompilację actora, source receipt cuts oraz cancellation fences. Nowszy kod
podłącza opt-in producenta v4, Remote/Profile REST i AHP do trwałego Journalu;
rzeczywisty dwuturowy pion REST→AHP→WSS→sandbox→Journal ma focused PASS
snapshot91480377 (6,61s, bez Chromium). Pełna regresja snapshota4061d4f
ma PASS545 relay/67 Journal/15 kernel checks, także worker ACK5/reservation0.
Produkcyjny formularz w Chromium ma osobny PASS snapshot8c61eef4 (12,99s),
obejmujący Remote pin, utratę202/retry, reload/context i końcowy worker ACK.
Snapshot0fa2d97a ma rzeczywisty packet loss/reorder, lost Submit/ACK reply i
Chromium PASS16,02s oraz all-targets Clippy PASS. Nowszy snapshotcf346502
ma Chromium PASS17,43s z aktywną cancellation, edycją/reorder kolejki i
confirmed stop przy Unknown wyniku (worker sessions2/turns5/ACK12).
Snapshotb528fbb4 ma nowszy pełny PASS548 relay/67 Journal/189 workspace/15 kernel,
w tym negative terminal/read-revocation regressions i wspólny service startup.
Snapshot8f8a9dcb ma actual SIGKILL/relaunch obu service i SIGSTOP/CONT kontrolera
PASS20,96s: stare tury Unknown/Queued bez replay, nowe boots, jawna świeża sesja
oraz source stop/ACK9/reservation0. Snapshotd3f24933 ma również dwa dokładne
Journal/ACK crash cuts PASS42,14s (przed ACK wire i po worker SQL ACK, przed
controller confirmation), każdy z restartem i SIGSTOP/CONT. Ten sam snapshot ma final all-targets Clippy
`-D warnings` PASS1m53s. Wraz z pełnym backend/kernel i Chromium przyjęto zakres
E4.4; platformy i rzeczywiste konta/tailnet pozostają w swoich bramkach. Wyłączona
konfiguracja zachowuje Remote501. Dokładne dowody i granice: progress.md;
E4.4 odebrano w opisanym zakresie Linux/opt-in. Cały cel do E10 pozostaje otwarty.

## Cel i obecna granica

Zdalna sesja ma używać jednego żywego procesu harnessu i jednego rzeczywistego
ACP `session/new` dla kolejnych uporządkowanych promptów. Następna tura zachowuje
kontekst harnessu z poprzedniej. Ponowne przesłanie całego transkryptu do nowego
procesu nie jest równoważnym zamiennikiem. Restart nie obiecuje wznowienia
wewnętrznego stanu modelu bez osobnego, rzeczywiście obsługiwanego ACP resume.

Punkty odniesienia w kodzie:

- [interactive.rs](../apps/server/src/interactive.rs): `CreateRequest`,
  `fresh_create_scope`, `Admission`, `Task`, `context_in_transaction`,
  `queue_text_allowed` i lokalna pętla `claim_next`/`session/prompt`. Lokalny
  actor utrzymuje jeden `Acp.remote` po `session/new` i negocjacji modelu/mode.
- [runtime.rs](../apps/server/src/worker_execution/runtime.rs): `Active`
  przypina jedną `ExecutionCommand`, `ExecutionLease`, `Connected` i grupę
  procesu. `Runtime::handle` obsługuje obecny kontrakt jednej próby v3.
- [runtime/acp.rs](../apps/server/src/worker_execution/runtime/acp.rs):
  `conversation` buduje `Acp`, `Acp::run` inicjalizuje harness, tworzy ACP session,
  ustawia model/mode i wysyła jeden prompt. Wynik jest bounded plaintext;
  reasoning nie jest przekazywany, narzędzia/broker i human approvals są odmawiane.
- [runtime/actor.rs](../apps/server/src/worker_execution/runtime/actor.rs):
  zakończenie bieżącej próby przechodzi przez zatrzymanie procesu i `StopProof`.
- [worker execution ledger](../apps/server/src/worker_exports/execution.rs):
  niezmienny inbox, lease oraz reservation pojedynczej próby i terminal outbox.
- [remote_execution.rs](../apps/server/src/remote_execution.rs): wewnętrzna
  orkiestracja jednej komendy z nowym Describe przed dispatch/renew, niezależnym
  monitorem, trwałym wynikiem i dokładnym ACK. Nie jest jeszcze mostem AHP.
- [session-runtime](../crates/session-runtime/src/lib.rs),
  [admissions.rs](../crates/session-runtime/src/admissions.rs),
  [controls.rs](../crates/session-runtime/src/controls.rs),
  [projection.rs](../crates/session-runtime/src/projection.rs): trwała kolejka,
  session lease, origin/rejection i projekcja oficjalnym reducerem AHP.

W obecnym v3 terminalny record implikuje potwierdzone zatrzymanie lokalnej
rezerwacji/procesu. Ta semantyka uniemożliwia potraktowanie końca tury jako
końca sesji przy zachowaniu żywego ACP. Nie wolno zmienić tego znaczenia po cichu.

## Jawne wersjonowanie

Oddzielny kontrakt `dunlin.worker.v4`/Hello4 ma własny moduł protokołu.
Negocjacja wymaga obu operator flags i osobnego żywego producenta v4;
nowszy kod podłącza producenta w startup; jego pion protokołu ma focused PASS, a pełny odbiór jest w toku. V1 diagnostyki, v2 metadata i v3 pojedynczej próby zachowują dokładne
dotychczasowe parsowanie, fingerprinty, reply correlation i terminal semantics.
Hello4 wymaga jawnej konfiguracji operatora, walidacji rzeczywistego runtime
sesyjnego i zarejestrowanego producenta kontrolera; brak downgrade/fallback.
Sama zmiana raportu capabilities nie daje uprawnienia do utworzenia sesji.

Dodano [worker/session.rs](../crates/relay-protocol/src/worker/session.rs)
i testy kontraktu. Publiczny moduł obsługuje DTO/Hello4; sam nie tworzy runtime.
`SessionOpenCommand`/`TurnCommand`
mają oddzielne tożsamości, receipt i fence. `SessionEvent` zawiera dokładną
referencję `(session, sequence, digest)` do poprzedniego zdarzenia; pierwsze ma
sequence1 i brak poprzednika. `TurnCompleted` niesie tylko receipt/status,
bez ponownego final text. `TurnRecord` i `SessionRecord` walidują oddzielne
referencje do terminalnego eventu, a tylko `SessionStopped` opisuje zatrzymanie
sesji. Runtime musi osobno potwierdzić StopProof, hashe i spójny durable stream.

Zamrożony codec helperów v4: SHA256 prefiksu bajtowego
`dunlin.worker.session.v4.open\0`, `dunlin.worker.session.v4.turn\0` lub
`dunlin.worker.session.v4.event\0` oraz
`serde_json::to_vec` właściwego typed obiektu w kolejności pól struktury.
Wyłącznie własny `identity.fingerprint` albo `event.digest` jest czyszczony;
digest poprzedniego zdarzenia, wszystkie boots/epochs/profile/descriptor/input
i ordered arrays pozostają w preimage. Helper najpierw waliduje obiekt wejściowy
(własny placeholder musi mieć poprawny format). Golden SHA256 dla TurnCommand
obliczono niezależnie Python hashlib; testy mutacji chronią pozostałe piny.
To codec wewnętrzny v4, nie zmiana istniejącego v3 i nie dowód authority.

DTO bounds: frame64KiB, input16KiB UTF-8, chunk4KiB UTF-8, retained do8MiB,
max128 tur, lifetime do min(profile.max_seconds,3600s), idle do30s i lifetime,
lease1–10s. Sam DTO nie zna aktualnego session remaining/cumulative output ani
worker policy: każdy runtime admission/renew musi je dodatkowo ograniczyć.
`validate_for_channel` zabrania Submit/Renew/Cancel/Close na innym boot/channel;
wyłącznie jawne reconcile i negatywny stream ACK mogą odnosić się do starej sesji.

Oddzielne typy powinny rozróżniać:

| Tożsamość | Rola |
| --- | --- |
| Controller interactive session ID + Journal lease | Właściciel i kolejka widoczne w AHP |
| Immutable session-open intent + fingerprint | Idempotencja utworzenia zdalnego procesu |
| Worker-issued remote session ID + session epoch | Konkretny proces i ACP session, nie konstruktor z JSON użytkownika |
| ChannelFence + host enrollment/credential epochs | Aktualny zweryfikowany kanał i scope |
| Immutable turn ID + command ID/fingerprint + turn epoch | Dokładny prompt wewnątrz konkretnej sesji |
| Worker session stream sequence | Kolejność i deduplikacja danych wyjściowych |

Polecenia proponowane: OpenSession, SubmitTurn, RenewSession, RenewTurn,
CancelTurn, CloseSession, ReconcileSession, ReconcileTurn i trwały ACK streamu.
Każdy mutujący komunikat ma zamknięty typed payload i dokładne przypięcie
session/channel/turn; żadnych ścieżek, argv, sekretów, arbitralnych JSON settings
ani flag „verified”. Historyczny fence służy tylko do read/reconcile/negative
cleanup, nigdy do dodatniego wznowienia. Stare v3 DTO nie są wrapperem v4.

## Worker: dwa poziomy życia i uprawnień

Nowy actor sesji jest właścicielem `Child`, guardian/namespace, stdin, jednego
czytnika stdout, ACP session ID, rosnącego JSON-RPC request ID oraz mailboxu
o ograniczonej pojemności. Wykonuje initialize/session-new/negocjację raz i nie
oddaje socketu ani publicznego konstruktora opaque authority do actora.

Sesja przypina niezmienny export identity/settings, własny Human Read+Execute,
published AgentProfileRef, dokładną definition/digest, zaufany lokalny harness
i izolację credentials. Kolejna tura nie może zmienić tych pinów. Zmiana profilu,
bindingu lub scope wymaga nowej jawnej sesji. Zmiana/revoke+regrant dowolnego
epoch kończy stare uprawnienie, również gdy proces jest bezczynny.

Istnieją dwa niezależne bounded lease:

- Session lease obejmuje cały czas życia procesu, również bezczynność między
  turami. Ciągła kontrola lokalnego grantu, FS identity, polityki, kanału i czasu
  nie może zależeć od nadejścia następnego promptu ani powodzenia write do stdin.
- Turn lease obejmuje jedną przyjętą turę, jej wall/output budgets i najwyżej
  jeden prompt write. Renew wymaga żywego session lease i świeżych pinów;
  duplikat renew nigdy nie przesuwa terminu od chwili odbioru duplikatu.

Guardian dostaje konserwatywny termin właściwy dla aktualnej fazy: podczas tury
minimum session deadline i turn deadline, podczas idle session deadline oraz
ustalony idle cap. Sesja ma również bezwzględny lifetime cap; nowe prompty ani
heartbeat nie mogą odnawiać jej bez końca. Limity idle/lifetime/retention i
zużycia host slotu trzeba zamrozić w kontrakcie przed implementacją.

Host reservation obejmuje żywą sesję przez cały jej czas, także po poprawnie
ukończonej turze. TurnCompleted oznacza odpowiedź ACP, a nie dowód braku żywych
potomków. Dopiero SessionStopped z rzeczywistym StopProof zwalnia reservation
sesji. Nie przenosimy osłabionej semantyki do v3 `ExecutionRecord`.

CancelTurn nie jest jeszcze dowodem przerwania skutków. Pierwszy bezpieczny
przekrój może zatrzymywać całą sesję przy cancellation i jawnie oznaczać utratę
ACP context; nie może obiecywać dalszych tur w tym samym procesie po kill.
Zachowanie żywej sesji po ACP cancel wymaga osobnego kontraktu i odbioru
in-flight notifications/background work. Nie wolno uznać samego CancelAck za
StopProof ani odblokować następnej tury przy niepotwierdzonym stanie.

## Trwałość i atomowe granice

Nowe tabele sesji/turn inbox/stream outbox powinny być oddzielone od v3 worker
execution inbox, aby nie reinterpretować historycznych rekordów. W SQL zapisujemy
stan historyczny i piny; monotonic lease/opaque FS authority pozostają wyłącznie
w pamięci. Stan SQL `active` nie umożliwia odtworzenia żywego pozwolenia po boot.

Kontroler potrzebuje durable session-target/profile pins i unikalnej relacji
Journal(session ID, lease epoch, turn ID) -> worker session/turn intent. Po stronie
Journalu i kontrolerowego ledgeru potrzebne są publiczne, jasno nazwane
`*_in_transaction` warianty z odmową autocommit oraz oddzielnym notify po commit.
Dodano już `Journal::claim_next_in_transaction`/`notify_committed_claim` oraz
Store `admit_remote_execution_in_transaction` i
`record_remote_execution_in_transaction`; ich nowe regresje czekają na wynik VM.
Własne wrappery transakcyjne pozostają dla dotychczasowych callerów.
Pełny atomowy consumer Journalu i remote ledgeru nadal nie istnieje;
`transition_timed` i dispatch wymagają dalszej jawnej kompozycji. Nie składa się
tych operacji przez nested transaction ani przez dwa niezależne commity.

Wymagane cięcia:

1. Create: przygotowanie scope w SQL, prawdziwy Describe poza Store, świeża
   IMMEDIATE transakcja current device/OIDC/host policy + ViewProject/CreateSession
   + exact binding/profile/v4 proof, Journal admission+lease+remote target pin+
   immutable OpenSession outbox. Brak promptu w create. Historyczny identyczny
   HTTP receipt nie tworzy nowego procesu ani nie odświeża authority.
2. Claim: kolejny queued turn, jego niezmienny fingerprint, unikalny remote intent
   i stan dispatching powstają w jednej transakcji. Przed SubmitTurn ponownie
   Describe poza Store i świeży końcowy cut; dispatch-intent przed pipe/network.
   Brak automatycznego nowego intent po niepewnym send/receipt.
3. Worker: SessionOpen i SubmitTurn najpierw trwały inbox/dedupe, później dokładnie
   jedno spawn/session-new lub prompt write. Duplikat odpowiedzi zwraca zachowany
   receipt, nie odnawia lease i nie uruchamia ponownie efektu.
4. Output: canonical worker event jest zapisywany do worker outbox przed wysłaniem.
   Kontroler w jednej transakcji deduplikuje (worker session, epoch, seq), zapisuje
   source event Journalu, oficjalną projekcję AHP i ewentualny turn terminal.
   Dopiero po commit potwierdza workerowi trwały contiguous stream watermark.
5. ACK: worker odpowiada po swoim SQL commit; kontroler czyści ack-pending tylko
   po dokładnej odpowiedzi i opaque aktualnym slocie. Utrata ACK/reply oznacza
   retry identycznego negatywnego ACK, nie ponowienie promptu.

Powolne SQL/FS/pipe/output/send nie zatrzymuje niezależnego guardiana. Permit
blokującej pracy żyje do rzeczywistego jej końca, także po timeout waitera.
Zachować brak silnego cyklu AppState -> Service -> task -> AppState.

## AHP i dostarczanie strumienia

Worker przekazuje zamknięte typed zdarzenia dozwolonego ACP subset, nie dowolne
agent JSON i nie gotowe `serverSeq` AHP. Kontroler mapuje do istniejących
`Journal::agent_update`/projection; globalny AHP serverSeq nadaje wyłącznie
kontroler. Worker seq i ACK streamu są odrębnymi licznikami.

Final turn record określa ostatni wymagany event sequence. Nie można zatwierdzić
Completed w AHP z brakującymi wcześniejszymi deltami. Replay starych chunków nie
duplikuje tekstu, a terminal/reconcile nie wstrzykuje całego final text drugi raz
po już utrwalonych deltach. Worker retention jest bounded; przy braku miejsca
stop/Unknown i zachowany gap/error, nigdy ciche pomijanie outputu. Zasady replay,
watermark, max frame/chunk/retained bytes i backpressure wymagają dokładnych DTO
oraz testów przed włączeniem streamingu.

Jeśli zweryfikowany StopRecord wyprzedził ostatnią stronę streamu, kontroler
zachowuje cleanup hint do jego final_sequence. Przyjmuje tylko łańcuch do
dokładnego końcowego digest/reason; spóźniony tekst zachowuje wyłącznie jako
digest, bez treści lub SDK projection. Prefix nie podnosi ACK przed dojściem
do tego końca. Poprawny końcowy event umożliwia zwykły, potwierdzony ACK,
bez skoku po samym DTO StopRecord, przywracania lease lub zmiany Unknown
na Completed. Źródłowe pochodzenie i pełny receipt nadal są obowiązkowe.

Prawdziwa semantyka sesji jest w metadanych: immutable remote target/profile,
worker session epoch, continuation available/interrupted, obsługiwane text,
reasoning/tool/permission capabilities. Root summary gates muszą wynikać z
żywego konkretnego Remote adaptera, nie z ogólnego `execution_ready` hosta.
Nie dodajemy `approveTools=true` bez rzeczywistego nonce-bound human approval
roundtrip. Poświadczenia człowieka nie są zastępowane decyzją modelu.

Istniejący AHP origin/clientSeq queue acceptance pozostaje jedyną ścieżką
browser promptu. Limity wejścia muszą być spójne: dziś Journal przyjmuje32KiB,
worker v3 input16KiB. Remote adapter odrzuca za duży tekst trwale na queue
admission, zamiast akceptować, claimować i dopiero potem odkrywać niedostępny
wire. Queue gating/cancel/reorder/read-shared muszą rozróżniać adaptery; shared
reader nie otrzymuje pozytywnych capability.

## Recovery i połączenia

Browser reconnect do kontrolera nie kończy sesji ACP: wraca przez AHP
replay/snapshot, zachowując origin ACK przed filtrem serverSeq. To inna granica
niż utrata kanału controller-worker.

Pierwszy przekrój v4 zachowuje obecną zasadę fail-closed: utrata zweryfikowanego
worker kanału/replacement/boot/lease kończy sesję i process group. Nowy kanał
może tylko odczytać historyczny stan i ACK/reconcile; nie przejmuje żywego ACP
na podstawie JSON i nie odnawia starego session/turn lease. Przyszły handoff
żywej sesji wymaga osobnego jawnego kontraktu; nie jest częścią tej propozycji.

Controller lub worker restart oznacza Interrupted i Unknown dla wysłanej lub
uruchomionej tury; brak automatycznego OpenSession/SubmitTurn/renew. Niewysłany
queued prompt pozostaje w kolejce, ale nie może ruszyć sam po recovery.
Unknown jest barierą dla następnego claim — obecne `claim_next` blokuje tylko
dispatching/running, więc Remote adapter wymaga dodatkowego trwałego warunku.
Jawny reconcile może ustalić historyczny wynik po sprawdzeniu aktualnego Read;
nie odtwarza ACP context. Nowa rozmowa wymaga nowego jawnego create.

## Kolejność małych przekrojów i kryteria odbioru

1. Zamrozić v4 DTO/hash/lease/stream invariants i migracje obu ledgerów.
   Testy parserów i zgodności: v1/v2/v3 fixtures identyczne, v4 payload odrzucony
   przez stare wersje, brak implicit downgrade, cross-session/epoch/turn denial.
2. Worker persistent actor bez aktywacji HTTP: dwa kolejne prompty w jednym
   prawdziwym ACP session/process, rosnące RPC IDs, zachowany stan harnessu,
   nowe initialize/session-new tylko raz. Rzeczywisty bwrap/subreaper oraz
   ciągłe idle+active+blocked-write guardians; profile/export/native metadata
   ABA nie ożywia procesu, quota sesji trzyma slot także między turami.
3. Durable stream/outbox + controller projection: drop/reorder/duplicate frames,
   missing-seq terminal, lost ACK/reply, SQL rollback i crash po każdym cięciu.
   Transkrypt AHP dokładnie raz, żadnego ACK przed journal commit, bounded spool.
4. Adapter interaktywny: istniejący HTTP keyed create i AHP queued origin flow,
   atomowy claim/mapping, obecne sharing/read policies, bez osobnego public RPC.
   Local ordinary admission fingerprints/receipts i działające v3 pozostają zgodne.
5. Rzeczywisty Chromium + private TLS/WSS + lokalny kontrolowany ACP: drugi prompt
   wymaga pamięci pierwszego w tym samym ACP session; browser offline/reconnect
   nie restartuje procesu; queue edits/cancel/revoke oraz capability labels są
   zgodne ze stanem; brak wycieku ścieżek/sekretów do browsera lub storage.
6. SIGKILL/relaunch obydwu usług oraz guard failure w idle, bootstrap, prompt write,
   output journal i ACK. Brak orphanów, brak automatycznego prompt replay,
   Unknown nie zwalnia session reservation bez rzeczywistego stop proof,
   jawny nowy create po recovery działa. Zero podwójnego skutku poprzedniej tury.

Ustalone w kodzie: oddzielne v4 API i budżety; cancellation kończy kontekst
i pozostawia Unknown, dopóki brak dowodu wyniku; potwierdzenie stop jest odrębne.
ACP subset obsługuje bounded plaintext/reasoning i przypięty profile/model/mode.
Remote human tool approvals pozostają unsupported; UI ukrywa tę capability.
Po Interrupted nowy kontekst wymaga jawnego nowego create.
MacOS worker i provider credential provisioning wymagają własnego odbioru.
Odbiór E4.4 opiera się na powyższej macierzy i zapisanych logach, nie samej
obecności kodu. Nie zamyka E5 ani E10. Remote501 pozostaje przy wyłączonej
konfiguracji; Linux persistent runtime wymaga obu jawnych flag operatora.

## Rozszerzenie MCP: osobna wersja v5, w realizacji

Kontrakt session/broker w relay-protocol definiuje proponowane wiadomości MCP
dla dunlin.worker.v5. Kanał negocjuje Hello5 po osobnym opt-in obu stron;
Transport kontrolera ma typowaną kolejkę otwarcia MCP i wywołanie z Actor
po instalacji fabryki. Publiczny REST przekazuje prywatne selections,
a zadanie aktora instaluje broker przed wysłaniem open5. Runner ma odbiór
natywnego proxy i pełnej połączonej ścieżki z kontrolowanym HTTP OAuth/MCP.
V4, jego hashe, parsowanie i odmowa profili MCP pozostają bez zmian.
Sama obecność DTO lub raport capabilities nie odblokowuje narzędzi runnera.

Broker kontrolera ma wspólny rdzeń OAuth/katalogu/jednorazowych zgód, oddzielony
od lokalnego procesu przez hostowy Authority. Dowód nie jest deserializowany z
wiadomości. Przypina session/workspace/owner, Journal lease, urządzenie startowe,
rewizję bindingu i digest profilu. Rdzeń sam sprawdza niezmienność tych pinów,
czas życia i prywatne ownership połączeń. Adapter wykonuje świeże kontrole
checkoutu lub worker/export/transport poza SQL oraz osobny cut wewnątrz
transakcji OAuth. Nie wolno robić tam I/O ani zagnieżdżać transakcji authority.
Slot brokera należy do rzeczywistego rdzenia, także jeśli oczekujący na katalog
zostanie anulowany, dopóki zadanie nie zakończy faktycznej pracy.

Prywatne konta mają wspólny binder używany przez lokalny admission oraz format
schema1 do8KiB. Snapshot przypina owner/workspace/profile digest i najwyżej8
provider/connection/revision/configuration pins, bez URL, tokenów i referencji
vault. Nie można deserializować z niego BoundSession: restore musi odczytać
bieżące własne konto z SQL, sprawdzić connected/revision/provider oraz aktualny
digest konfiguracji operatora. Dane nie tworzą Journal lease ani dispatch permit.
Controller SQL przechowuje zapis w opcjonalnym mcp prywatnego Pins, poza
OpenBrokerSession. Osobne przygotowanie/admission wymaga świeżego konta,
opublikowanego profilu MCP, trzech flag kontrolera i opaque transportu z ofertą5.
Przy commit ponownie sprawdza konto i aktualny slot producenta/lease/kanału.
Intent v5 wiąże prywatny snapshot i wybrane limity; wire fingerprint v5 ich
prywatnej części nie zawiera. Open jest zapisywany jako odrębny wrapper v5,
a legacy pozostaje płaskim v4 JSON z tymi samymi bajtami/hashami. Nie zmieniono
schematu SQLite: użyto istniejących ograniczonych pól JSON i skip-none pinów.
Publiczny retry odczytuje dotychczasowy receipt bez żywego transportu,
nowego lease i komendy. SDK autoryzuje aktualny odczyt i wywołuje SQL-only
kontrolę hosta, która ponownie dopasowuje profil, binding, grant i prywatne
konto/config do fingerprintu. Zapisane limity służą wyłącznie porównaniu,
nie tworzą transportu ani wykonawcy. Recovery sprawdza jedynie kształt/scope prywatnego zapisu, więc
odłączenie OAuth nie blokuje zatrzymania zamiaru, który nie przekroczył send cut.
Publiczny endpoint przyjmuje MCP tylko przez aktualne prywatne selections.
Actor wybiera dodatnie wykonanie scoped tylko z dowodem zapisanym przez udaną
instalację hostowej fabryki. Przed jej zakończeniem można przyjąć kolejkę
pending/dispatched:false/receipt:none ze świeżym, tymczasowym proof oryginalnego
Actor/lease/transportu. Proof ten pozostaje lokalny w queue cut i nie jest
instalowany do dispatch. Obecność wiersza SQL nie tworzy authority.
Capture/restore są kontrolą nowego admission i wymagają connected. Aktywny
broker zachowuje odrębną walidację rewizji/config, a stan refreshing obsługuje
credential journal. Nie należy wywoływać restore w cutach trwającej rotacji:
odrzuciłoby to własne, jeszcze niezakończone odświeżenie tokena.

Rdzeń scoped kontroli kontrolera ma osobny RemoteMcpScope, bez Deserialize,
Serialize i Clone. Capture wymaga żywego transportu5 i bieżącego Journal lease;
przypina owner/workspace/device, binding revision, profil, intent fingerprint,
tożsamość open i monotoniczny deadline. Context ponownie sprawdza projekt,
urządzenie, prywatny snapshot, konto/config, lease/kanał/producenta, zamknięcie
i cancellation. Wariant context otwiera własną transakcję; context_in_transaction
wykonuje wyłącznie SQL w już istniejącym cut OAuth. Live walidacja dopuszcza
refreshing, a credential journal nadal wymaga właściwej generacji/operation ID.
Nie rekonstruuje BoundSession ani zgody z danych od workera.

Observe przyjmuje świeży typed Describe zebrany przez hosta poza Store/SQL,
wymaga identycznego export/grant/settings/fence i Execute. Sam context nie
wykonuje zdalnego I/O i nie zastępuje tego sprawdzenia. Katalog żądany przez
runner wymaga zapisanego native receipt i dokładnego SessionFence; tools/call
wymaga active sesji, accepted native TurnReceipt, pełnego TurnFence i SDK running.
Hostowy adapter Remote implementuje wspólne Authority: capture wymaga żywego
aktora i jego oryginalnego lease, a słabe referencje do Actor/Service odcinają
sesję po utracie właściciela i unikają cyklu z AppState. Adapter sprawdza
Actor/IdP/host policy, a turn pobiera wyłącznie z żywego zegara aktora. Kontrola
requestu łączy ten zegar z native Session/TurnFence i bieżącą turą SDK.
Check pobiera nowe Describe przez zwykły transport metadanych, poza Store, na
blocking workerze. Kontroluje deadline także po zakończeniu oczekiwania.
Wspólny broker zachowuje jednorazową zgodę i kontrole przed/po HTTP.

Przed każdym Store lock broker pobiera osobny SqlAuthority. Dla Remote ten cut
zajmuje bounded slot kontroli polityki, trzyma Management lock, odczytuje
ustawienia host_accounts poza SQL i zachowuje je przez całą operację SQL/OAuth.
Context w już istniejącej transakcji nie robi FS/network ani nested transaction.
Kontrola urządzenia zatwierdzającego także korzysta z tego samego cut: sprawdza
bieżącego właściciela, urządzenie i IdP w SQL oraz przechowaną politykę hosta,
zamiast ponownie czytać settings pod Store lock. Jednorazową zgodę nadal
weryfikuje SDK.
Kolejność blokad to Management→Store; cut nie jest trzymany podczas oczekiwania
na HTTP lub zgodę. Bezpośrednie wywołanie starego context na Remote odmawia,
żeby nie ominąć cut. Lokalny adapter zachowuje dotychczasową ścieżkę.
Powiadomienie brokera o zapisanej zgodzie/wyniku budzi także trwały Journal/AHP.

Hostowa fabryka łączy już ten adapter, odtworzone prywatne konta i Common Broker
z SessionBroker transportu v5. Setup jest jednorazowy dla oryginalnego Actor;
błąd/anulowanie zatrzymuje aktora, a finish usuwa jego RAII registration.
Fabryka odłącza remote_sessions OnceLock w klonie AppState przechowywanym przez
brokera. To usuwa ścieżkę Actor→producer→Broker→AppState→Service→Actor; same Weak
w adapterze nie wystarczają. Odbiór sprawdza zanik producenta/Service i powrót
runtime slotów także z oryginalnym Service wpisanym do AppState.remote_sessions.
Instalacja jest własnością zadania admission po commit, także po utracie HTTP
waitera. Jej deadline nie przekracza oryginalnego lease; błąd idzie przez wspólny
negative cleanup. Po udanej instalacji aktor korzysta z typed open5 i scoped SQL.
AHP dopuszcza toolCallConfirmed i publikuje approveTools tylko dla takiej
fabryki; pending queue i zwykłe zdalne ACP nie wystarczają. SDK weryfikuje
dokładną jednorazową decyzję i dodaje canonical nonce/human evidence.

Warstwa SQL ma jeden RemoteSessionRuntime z niepublicznym konstruktorem
wariantu MCP. Wariant ten pożycza oryginalny RemoteMcpScope i konfigurację,
a każdy odczyt/claim/dispatch/import/renew/projekcja ponownie sprawdza żywe piny,
prywatne konto i SDK lease. Zwykły wariant legacy jawnie odmawia MCP; callback
source/caller ani zapis SQL nie potrafią go odblokować. Otwarcie zwraca RemoteOpen,
a aktor wysyła oryginalny wrapper5 i waliduje receipt5. Dodatnie mutacje mają
końcową kontrolę monotonicznego deadline wewnątrz transakcji, więc błąd wycofuje
także zmiany SDK. Actor monitor, queue eligibility, import receipt i projekcja
zdarzeń wybierają tę samą ścieżkę z dowodem udanej instalacji brokera.
Wyjątek pending queue opisany wyżej nie nadaje prawa wykonania.

Korelacja source oraz negative cleanup nie zależą od konta OAuth: wymagają
właściwego node/host/workspace, opaque bieżącego kanału i dokładnego native
receipt/zdarzenia. Odłączenie konta blokuje dodatni import/renew/projekcję,
ale nie blokuje capture receipt do zamknięcia, interrupt ani potwierdzenia
rzeczywistego zatrzymania. Te operacje nie tworzą Ready, lease ani nowej zgody.

Native request może wyprzedzić zapis receipt. Osobny odczyt readiness zwraca
pending wyłącznie po sent markerze oryginalnego open albo oryginalnego submit
z SDK dispatching; nie nadaje zgody ani nie wykonuje narzędzia. Callback czeka
do2s (także pod deadline RPC), budzony zmianą aktora, i ponownie sprawdza całe
authority. Do SDK consent/tools/call przechodzi dopiero z dokładnym accepted
native TurnFence, SDK running i własnym nieodnawialnym zegarem tury.

Slot per-session callbacku przenoszony jest do rzeczywistego spawn_blocking
wykonującego Common Broker, więc cancel waitera nie zwalnia go podczas HTTP.
Broker dostaje deadline RPC oraz wynegocjowany limit odpowiedzi; strony katalogu
mieszczą się w tym limicie, a zbyt duża pojedyncza definicja odcina setup przed
startem runnera. Wynik narzędzia większy niż limit sesji daje unknown/interrupt
bez częściowego wyniku ani retry, także gdy SDK zna już wykonany wynik.
Wyłącznie lokalne błędy walidacji sprzed dispatch zwracają InvalidRequest i
pozwalają agentowi poprawić nowe żądanie. Konto, tokeny i ich vault references
pozostają na serwerze.

Żywy Actor przechowuje już oryginalny Journal lease z nowego admission i porównuje
epoch/instance, session oraz owner przy dodatnich cut i queue. Nie przyjmuje
zastępczego lease ze snapshotu SQL. Lokalny zegar tury powstaje po własnym claim,
przed SubmitTurn: przypina pełną komendę oraz minimum deadline sesji i budżetu
tury. Powtórny start nie odświeża zegara, zmieniona komenda nie przechodzi submit,
a accepted/replay receipt nie tworzy zegara. Dopiero odczyt zatwierdzonego stanu
bez aktywnej tury usuwa bieżący zegar; indeks ukończonej tury nie może być użyty
ponownie w tym aktorze. Hostowy adapter MCP korzysta z tego żywego stanu i
sprawdza przyjęty TurnFence przez RemoteMcpScope.

W proponowanym v5 runner wysyła tools/list pod pełnym SessionFence, a tools/call
pod pełnym TurnFence. Request ID i fingerprint wiążą także oba boot IDs,
channel, sesje kontrolera/workera, indeks/epoch tury, alias i dokładny JSON
argumentów. Alias jest wyłącznie hostowym mcp_ plus32 hex; argumenty są obiektem
do8KiB/1024 węzłów/depth32. Prywatne connection IDs, OAuth i client secrets nie
mają pola w kontrakcie. Kontroler rozwiązuje konto z własnego admission sesji.

Odpowiedź JSON do256KiB jest dzielona na fragmenty UTF-8 do4KiB, najwyżej128;
każda ramka nadal ma limit64KiB. Fragmenty wiążą pełną referencję requestu,
kolejność, rozmiar i SHA256 całego payloadu. Zmiana scope/header/digest, duplikat
lub reorder trwale zamykają daną assembly. Niepełna odpowiedź nie jest wynikiem.
To tymczasowa korelacja wiadomości, bez automatycznego retry, wznowienia
uprawnień czy deklaracji exactly-once wobec zewnętrznego MCP. Odbiorca musi
ponownie sprawdzić aktualną authority przed przekazaniem wyniku agentowi.

Osobne kodeki definiują Hello5 z ofertą limitów i
OpenBrokerSession z własnym fingerprintem obejmującym profil i wybrane limity.
Legacy dekodery odrzucają te wiadomości. Pasywne API worker ledger wymaga
oferty z opaque ExecutionChannel oraz własnej polityki operatora schema2 z
mcp_broker:true. Przypina limity do lease i trwałych pinów, sprawdza je przy
każdym użyciu oraz zachowuje rollback/retry/recovery bez mintowania nowego lease.
Oferta nie zastępuje własnego Read+Execute, a cofnięcie i ponowne nadanie grantu
nie przywraca starej sesji. Aktywny socket przypina ofertę do opaque transportu
kontrolera po Accepted, ale nie tworzy jeszcze brokerowego native ExecutionChannel.

Controller wymaga execution_enabled, persistent_sessions i mcp_broker oraz
żywego producenta sesji. Worker wymaga enabled, persistent_sessions, mcp_broker
i rzeczywistego Runtime wspierającego sesje; nie obniża wybranej wersji po błędzie.
Obie flagi mcp_broker domyślnie są false. V5 używa odrębnego Hello5; Hello4 na
kanale5 i Hello5 na kanale4 są odrzucane przed nonce proof/lease. Nadal wymagany
jest pełny prywatny reverse proof. Po nim wspólne control/event ramki sesji
obsługują profile bez MCP, zachowując dotychczasowe fingerprinty i piny.
Wymaganie mcp_broker w configu transportu jest oddzielne od polityki exportu
schema2/mcp_broker:true. Bez przypisanego hostowego brokera każde poprawne
żądanie MCP pod tym kanałem otrzymuje Denied, bez discovery/OAuth/HTTP; obcy
channel/boot jest błędem protokołu. Monitor hosta przyjmuje obserwację wersji5,
która nie nadaje execution_ready ani uprawnień MCP.

Pętla kontrolera WSS ma już session-owned rejestr SessionBroker. Bind wymaga
żywego opaque transportu5, prawidłowego ID sesji i unikalnego właściciela;
rejestr ma do8 sesji i przechowuje Weak callback, a owner trzyma RAII registration.
Drop usuwa wyłącznie własną generację i sygnalizuje interrupt. Drop/replacement
kanału zatrzymuje wszystkich jego producentów. Rejestr jest wyłącznie korelacją:
callback musi niezależnie sprawdzić pełne private/native/SDK authority.

Żądania są uruchamiane w osobnych futures, a socket select odbiera gotowe
fragmenty; nie czeka wprost na HTTP lub zgodę. Waiting work i gotowe odpowiedzi
współdzielą wynegocjowane max_pending sloty. Limit odpowiedzi jest sprawdzany
przed chunkowaniem, slot zostaje zajęty do ostatniego fragmentu, a każdy fragment
sprawdza bieżącego właściciela/kanał. Timeout150s, utrata producenta, malformed
lub oversized wynik kończą daną operację jako unknown i interrupt, bez częściowego
wyniku/retry. Dispatched request IDs nie mogą być powtórzone; pamięć historii
jest ograniczona do4096 na kanał. Capacity rejection nie dispatchuje callbacku.
Rzeczywisty hostowy callback zachowuje własne sloty HTTP/blocking work do
zakończenia tej pracy także po anulowaniu waitera; kolejka socketu ich nie
zastępuje. Hostowa fabryka Common Broker + Remote Authority + prywatne konta
jest zaimplementowana i odbierana wewnętrznie. Jej instalacja oraz pozytywne
cut, import receipt i negatywne cleanup sesji MCP są połączone z wewnętrznym
aktorem; instalacja przy publicznym admission/startup i proxy runnera
pozostają do podłączenia. Legacy ścieżka
nadal jawnie odmawia MCP.


Transport kontrolera przechowuje teraz w tej samej ograniczonej kolejce odrębne
warianty legacy control i OpenBrokerSessionCommand. Serializacja wybiera
open_broker_session z opakowaniem5, a walidacja wymaga rzeczywistej oferty5,
obu boot IDs, kanału, limitów i tożsamości uwierzytelnionego hosta. V4 odmawia
przed zajęciem slotu lub kolejki. Hash obejmuje oryginalny payload i limity;
komenda nie jest rzutowana na v4 przy wysyłaniu ani korelacji odpowiedzi.
SessionAccepted wymaga pełnego receipt dla v5; Rejected musi wskazywać dokładny
kanał i open identity. SessionReady oraz zdarzenia są osobnymi obserwacjami,
nie odpowiedzią na oczekujące open. Kolejka i WS owner zachowują dotychczasowe
limity, deadline i obsługę heartbeat. Sam transport nie nadaje uprawnień
wykonania; scoped SQL dispatch/import/cleanup są połączone z wewnętrznym
aktorem po instalacji brokera. Publiczny admission/startup jest podłączony;
runner odbiera typowane open5.

Runner ma osobny bounded dispatcher podłączony do właściciela socketu v5.
Kolejka i operacje wysłane dzielą maksymalnie8 slotów; anulowanie wysłanego
waitera nie zwalnia jego slotu do odpowiedzi, deadline lub zamknięcia kanału.
Każda operacja dostaje świeży UUID. Dispatcher koreluje pełny request i fragmenty,
sprawdza wynegocjowany rozmiar przed alokacją, ma osobny timer wygaśnięcia oraz
ponowną kontrolę kanału przed dostarczeniem wyniku. Oczekiwanie jest ograniczone
do150s i musi dodatkowo mieścić się w budżecie aktualnej tury. Utrata odpowiedzi
nie kolejkuje ponowienia. Dispatcher nie nadaje native authority: runtime musi
sprawdzić aktualne export/session/turn leases przed żądaniem i użyciem wyniku.
Dispatcher jest już podłączony do zweryfikowanej pętli WSS v5. SocketOwner
powstaje dopiero po Accepted i ukończonym reverse proof; używa tej samej oferty
limitów co wysłane Hello5. Własny tick100ms ma priorytet przed odbiorem ramek,
więc obcy/nieskorelowany reply, expiry lub drop kończy kanał i runtime zamiast
odtwarzać kolejkę. Heartbeat i session/outbox jobs nie czekają inline na HTTP
ani zgodę. Outbound zachowuje deadline oryginalnego RPC, także przy samym send.
Native runtime Connected otrzymuje opaque Client i sprawdza jego pełny fence,
żywy guard oraz niezakończoną kolejkę/sloty; z niego tworzy ExecutionChannel
z prawdziwą ofertą brokera. Zamknięcie SocketOwner odbiera także ten dowód,
niezależnie od flagi open samego Connected. Legacy connection pozostaje bez
oferty. Client/Connected nie są wire DTO ani zgodą narzędzia.

Odbiorca open v5 zachowuje wrapper do natywnego open_broker_session. Lokalny
Unix proxy wymaga oryginalnych NativeSession/Turn leases i świeżych FS/grant
checks przed RPC, po nim i przed dostarczeniem. ACP dostaje konfigurację tylko
z opaque HarnessTools utworzonego przez żywe proxy. Osobny token uwierzytelnia
lokalny socket, nie nadaje uprawnień do OAuth ani innej sesji. Wybrany limit
odpowiedzi jest sprawdzany przed składaniem fragmentów. Błąd dostarczenia
zaakceptowanej odpowiedzi przerywa native sesję bez retry. Cztery nowe cases
proxy i sześć dotychczasowych remote cases mają produkcyjny runtime10/10 PASS;
dowody i zakres kontrolowanego hostowego respondera zapisano w progress.md.
Sam Client nie zastępuje native lease ani zgody SDK. Publiczny zdalny profil
MCP ma odbiór runtime11/11 z kontrolowanym dostawcą HTTP OAuth/MCP, prawdziwą
zgodą SDK przesłaną przez testowego klienta AHP oraz rzeczywistym helperem.
Przy utracie TurnAccepted runner przesyła ograniczone, powtórzone potwierdzenie
oryginalnej aktywnej tury przez outbox, bez ponownego Submit; narzędzie nie musi
czekać na tekst agenta, który sam czeka na zgodę.

UI wyboru zdalnego MCP rozróżnia globalną obsługę kontrolera od lokalnego
runtime; nie uznaje flagi za authority wybranego hosta. Wymaga jawnego konta
per provider, resetuje je po zmianie targetu i zachowuje prywatne selections
w niezmiennym zamiarze retry. Chromium przechodzi formularz, utratę rzeczywistej
REST odpowiedzi, identyczny retry i zgodę na pokazane parametry; desktop/mobile
bez overflow. Zachowana jest osobna kontrola rzeczywistego stopu/ACK.

Połączone przypadki odmowy zgody, utraty odpowiedzi HTTP po przyjęciu narzędzia
oraz disconnect konta podczas HTTP mają odbiór runtime15/15 z oboma Chromium
branches. Odmowa pozwala kontynuować tę samą sesję bez HTTP tools/call.
Pozostałe dwa przypadki dają Unknown i dokładnie jedno HTTP tools/call,
bez efektu w checkout po błędzie i bez replay po ponownej dostawie zgody lub
zamiaru REST. Potwierdzono końcowy stop/ACK2, reservation0 i zatrzymanie potomka
z otwartym FD. Po disconnect stary zamiar REST jest odrzucany przez świeżą
kontrolę konta; po utracie odpowiedzi zwracany jest pierwotny receipt.
Dodatkowy wymuszony test kończy źródłową turę po przygotowaniu RenewTurn,
ale przed obsłużeniem tego RPC. Runner nadal odmawia StaleAttempt. Kontroler
odczytuje ograniczony, zweryfikowany stream i wymaga canonical terminal tej
tury w świeżym Describe/authority/SQL cut; dopiero wtedy zachowuje sesję.
Nie wysyła ponownie RenewTurn, Submit ani tools/call i nie przedłuża budżetu.
Brak terminala lub bieżących uprawnień nadal kończy kontekst. Wariant sprzed
poprawki deterministycznie przerywał sesję jako Conflict; po poprawce przechodzą
dwie tury z tym samym procesem ACP i końcowym ACK5/reservation0.
Form MRTR ma publiczny runtime/UI i odbiór19 scenariuszy opisany w mcp-client.md:
2 rundy, exact schema validation/correction, dedupe, decline/cancel i account
revocation podczas oczekiwania. Client/Pending/requestState zostają na hoście;
runner otrzymuje dopiero zweryfikowany końcowy wynik. Pozostają URL elicitation,
trwałe provider revoke/GC, ogólne scoped secrets i odbiór dostawców/platform.
Po utracie authority/channel lub niepewnym dispatch wymagane jest zatrzymanie
i unknown, bez odtwarzania narzędzia po reconnect. DTO nie zastępuje tych bramek.
Bieżący backend810, runtime19 z wszystkimi Chromium branches, frontend147,
TypeScript i Clippy all-targets PASS. Backend/runtime/Clippy wiąże ten sam
sourceab27a153; frontend/typy mają własne mniejsze zbiory wejść. Publiczny UI
oraz żywa kontynuacja form MRTR są podłączone i opisane w mcp-client.md.
Dowody i dokładne granice odbioru zapisano w progress.md.
