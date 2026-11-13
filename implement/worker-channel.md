# Kanał controller–worker

Ten dokument opisuje warstwę kanału v1/v2: uwierzytelnione WSS, reverse proof,
heartbeat, trwałe fencing i metadata exports. Lease kanału sam nie jest grantem
projektowym, sandboxem lub pozwoleniem wykonania. Zdalne wykonanie v3 i trwałe
sesje ACP v4 mają osobne kontrakty i jawne flagi operatora:
[wykonanie](worker-execution-plan.md), [sesje](remote-session-plan.md).
Eksperymentalny transport v5 ma dodatkowy `mcp_broker = true` po obu stronach:
`[worker_controller]` na kontrolerze i `[execution]` w configu workera. Domyślnie
jest false. Wymaga również obu dotychczasowych zgód wykonania/trwałych sesji,
rzeczywistego runtime i żywego producenta sesji. Nie ma automatycznego fallbacku
do v4. Oddzielny Hello5 i oferta limitów przechodzą ten sam reverse proof;
po Accepted oferta należy do konkretnego opaque transportu i obu boots.
Profile bez MCP mogą używać wspólnych ramek sesji przez v5. Sam kanał5 nie
uruchamia MCP: bez przypisanego hostowego brokera request dostaje Denied i nie
wykonuje discovery/OAuth/HTTP. Nowy open MCP, prywatne konta, dispatcher i proxy
wymagają dalszego podłączenia opisanego w planie sesji. Polityka exportu
schema2/mcp_broker:true jest osobną zgodą, której flaga transportu nie zastępuje.
E4.4 i panel kondycji E4.5 są odebrane w zakresie zapisanych fixtures/testów;
realne platformy i tailnet pozostają oddzielnymi bramkami. Statyczny rejestr
hostów nadal zwraca execution_ready:false; nie zastępuje weryfikacji zlecenia.

## Transport i tożsamość

`dunlin.worker.v1` i `dunlin.worker.v2` to oddzielny protokół infrastruktury, bez browser cookies,
user/native bearer lub workflow service principals. Kontroler przed upgrade
wymaga dokładnie jednego Bearer dedykowanego credential workera, zatwierdzonego
hosta, braku Origin/Cookie i dokładnego subprotocol. Hello musi odpowiadać
uwierzytelnionemu host/workspace/enrollment epoch/credential revision.

Worker łączy się wyłącznie przez jawnie skonfigurowany Unix LocalAPI Tailscale:

1. Sprawdza własny Self ID/DNS, aktywny daemon oraz prywatny Serve root proxy
   do dokładnego loopback listenera. Funnel, nakładki i nieobsługiwane tryby
   odrzuca tak samo jak [lokalna inspekcja](hosts.md).
2. Odczytuje peer z netmap po stabilnym ID i dokładnym DNS, wymaga online,
   InNetworkMap, braku Expired i wyłącznie przypisanych adresów Tailscale.
   Nie używa systemowego DNS, LAN, adresów subnet routes ani env proxy.
3. Wysyła LocalAPI POST `/localapi/v0/dial` dla numerycznego adresu peera,
   port443/tcp i wymaga101 z `Upgrade: ts-dial`. HTTP200 `Dial-Self` jest
   celowo odrzucane: upstream może sugerować wtedy zwykły systemowy dial.
   Nie ma takiego fallbacku w Dunlin.
4. Na uzyskanym strumieniu wymaga TLS z publicznym root store i dokładnym
   server name, a potem ponownie sprawdza mapowanie i lokalne Serve.
   Nie ma produkcyjnej flagi wyłączającej weryfikację certyfikatu.

Kontrakty odczytano2026-11-11 w oficjalnym
[LocalClient](https://github.com/tailscale/tailscale/blob/main/client/local/local.go)
i [handlerze LocalAPI](https://github.com/tailscale/tailscale/blob/main/ipn/localapi/localapi.go).
DialTCP/UserDial jest oznaczone jako stabilne API; nieobsługiwane odpowiedzi
powodują odmowę. Sprawdzono także nagłówek101 w bieżącym źródle handlera.
Kontrola własnej konfiguracji nie jest atomową blokadą na przyszłość ani
kryptograficznym poświadczeniem dowolnego zdalnego hosta. Lokalny OS user/root
oraz wybrany daemon są granicą zaufania.

## Reverse proof przed lease

Kontroler wysyła losowy256-bit nonce, channel ID i swój boot ID. Worker zapisuje
je wyłącznie dla bieżącego uwierzytelnionego WSS, z własnym boot ID, hostem
oraz epochami. Zapis ma10s ważności. Publiczny request JSON nie może utworzyć
rekordu proof.

Po Ready kontroler otwiera własne zweryfikowane połączenie TLS przez LocalAPI
na przypięty Serve origin workera i wysyła POST `/v1/worker/channel-proof`.
Worker odpowiada tylko dla zgodnego aktualnego pending nonce. Kontroler wymaga
zgodności nonce, channel ID, obu boot IDs, hosta i obu epochów. Dopiero potem,
przy nadal aktualnym credentialu, transakcyjnie otwiera lease i wysyła Accepted.
Samo posiadanie credentiala i wysłanie Ready nie wystarcza do otwarcia kanału.
Po zaakceptowaniu/wygaśnięciu/zerwaniu połączenia worker usuwa pending proof.

Proof endpoint odrzuca Cookie/Origin/Authorization, ma limit64KiB/5s,
8 równoczesnych żądań przed ekstrakcją body i64 połączenia TCP, również przed
odczytem kompletnych nagłówków. Odmowa lub utrata callbacku nie przyjmuje pracy.
Nonce nie trafia do URL, logów ani browser storage.

## Heartbeat i fencing

Trwały `team_worker_channels` ma jeden rekord na host, osobny rosnący epoch,
channel/boot IDs, expiry, sequence i phase. Nie jest usuwany przy reenrollment.
`IMMEDIATE` transaction szereguje wyścigi. Każde open/renew/validate ponownie
sprawdza dokładny hash credentiala, host enrollment/credential epochs i Approved.
Drain dopuszcza diagnostyczny kanał; nie nadaje nowych uprawnień wykonania.

Nowy kanał zastępuje poprzedni z nowym fencing epoch. Heartbeat ma kolejną
sekwencję; dokładny duplicate zwraca wcześniejsze expiry bez przedłużania go.
Starsza/skacząca sekwencja, expired lease lub inny boot/channel są odrzucane.
Klient sprawdza deadline przed i po odczycie, także przy ramkach już gotowych
w buforze po zatrzymaniu runtime; spóźniony ACK nie przywraca wygasłego lease.
Seria ramek kontrolnych podczas oczekiwania jest ograniczona.
Expired kanał nie może zostać odnowiony przez spóźniony heartbeat.
Maksymalnie64 aktywne kanały, także przy wyścigach niezależnych połączeń SQLite.

Kontroler niezależnie sprawdza credential/lease i shutdown co100ms. Ten guard
obejmuje również zablokowane Accepted/ACK i operacje proof. Oddzielny guard
co5s kontroluje lokalną konfigurację i peer, z limitem3s. Worker ma analogiczny
niezależny guard sieci oraz monotoniczny deadline liczony konserwatywnie od
wysłania Ready/heartbeat, nie od opóźnionego nadejścia ACK. Interwały nie są
obietnicą czasu rzeczywistego przy zatrzymanym kernelu/runtime.

Dokładny captured lease można zamknąć offline także po utracie credentiala;
cleanup nie może zmienić nowszego kanału. Restart kontrolera ma nowy boot ID,
a restart workera nowy worker boot. Stare stampy nie odzyskują ważności.
GET `/v1/web/workspaces/{workspace}/hosts/{host}/channel` udostępnia raw diagnostic
record, bieżący controller boot ID i `reported_state_only:true`.
Diagnostic read wymaga aktualnego ManageHosts; zapisany active może być stary
lub expired, więc samo odczytanie rekordu nie dowodzi zdrowia lub authority.

V2 udostępnia path-free DescribeExport dla aktualnego principal grantu.
Kontroler ogranicza kolejkę i pending map wspólnymi8 permits, request ma5s.
Odpowiedź musi mieć dokładny request/channel/lease, scope, export i principal.
Zamiana kanału odcina stare requesty; opóźniona rejestracja nie może nadpisać
nowszego epochu. V1 zachowuje handshake/heartbeat, bez obsługi metadata.

Worker ma maksymalnie8 metadata requests i2 jobs FS. Permit FS pozostaje
w blocking closure do rzeczywistego końca, także po timeout/drop odbiorcy.
SQL candidate→FS→fresh SQL nie trzyma mutexa/transakcji podczas FS. Pętla WSS
obsługuje heartbeat równolegle, bez czekania na metadata job.

Protokół ma64KiB limit przed JSON, ścisłe DTO i znaną listę kodów błędów.
Kontroler ogranicza także częstość ramek. V1/v2 nie mają komunikatów wykonania
ani resume zadania. V3/v4 rozszerzają kanał o osobne typowane zlecenia i lokalny
guardian; v4 ma trwały strumień zdarzeń i jego replay. Replay zdarzeń nie jest
ponowieniem pracy ani odtworzeniem procesu ACP po restarcie; szczegóły i dowody
zawiera [plan sesji](remote-session-plan.md).

## Uruchomienie

Lokalny katalog zatwierdzonych checkoutów i grantów operatora jest opisany
w [worker-exports.md](worker-exports.md). Kanał v2 odczytuje wskazany export, lecz nie wykonuje pracy na jego podstawie.

Operator najpierw rejestruje i zatwierdza host zgodnie z [hosts.md](hosts.md).
Bilet i credential są oddzielne. Nie logujemy credentiala ani nie przekazujemy go
w args/env/URL. Obecna komenda przyjmuje pojedynczy credential przez stdin;
integracja trwałego keyringu workera pozostaje osobnym odbiorem.

Kontroler: dodatkowa sekcja jego istniejącego TOML:

```toml
[worker_controller]
socket = "/var/run/tailscale/tailscaled.sock"
node_id = "nCONTROLLER"
serve_origin = "https://controller.tailnet.ts.net"
```

Własny Serve musi wskazywać dokładny listen adres relay. Bez tej sekcji kanał
jest wyłączony. Worker używa odrębnego pliku:

```toml
host_id = "hst-ID-Z-ENROLLMENTU"
workspace_id = "WORKSPACE-ID"
enrollment_epoch = 1
credential_revision = 1
# Opcjonalnie: istniejący katalog zatwierdzony przez operatora CLI worker export.
# exports_state_dir = "/operator/private-worker-state"

[network]
socket = "/var/run/tailscale/tailscaled.sock"
node_id = "nWORKER"
serve_origin = "https://worker.tailnet.ts.net"
listen = "127.0.0.1:8788"

[controller]
node_id = "nCONTROLLER"
serve_origin = "https://controller.tailnet.ts.net"

[capabilities]
protocol_version = 1
version = "OPERATOR-CONFIGURED-VERSION"
os = "linux"
architecture = "aarch64"
harnesses = ["opencode"]
sandbox = "linux_bubblewrap"
max_parallel = 1
```

```sh
dunlin-relay worker run --config /operator/worker.toml --credential-stdin
```

Raport capabilities jest deklaracją, nie discovery zainstalowanych narzędzi
ani dowodem sandboxa. Wartości host/workspace/epochs muszą pochodzić z własnego
enrollmentu. Config musi być zwykłym plikiem do64KiB, bez symlinków/FIFO;
Unix open jest nonblocking przed sprawdzeniem typu. Lokalny system plików
operatora pozostaje zaufaną granicą; nie obiecujemy anulowania dowolnego kernel
I/O. Stdin ma30s/128B i jest zeroizowany. Nie ma auto restartu pracy po błędzie.

Tryb wymaga dwóch różnych węzłów w netmap i Unix LocalAPI. Lokalny standalone
korzysta z dotychczasowych local bindings. Mac GUI LocalAPI port/password,
bezpośredni TCP i SSH nie są zastępczymi transportami tego adaptera.

## Weryfikacja i bramki

Fixture ma dwa syntetyczne Unix LocalAPI,101 dial, rzeczywiste TLS proxy,
produkcyjny controller/router i pełny worker client/proof listener. Jedyna
zmiana trust w fixture to testowa CA przez cfg(test); weryfikacja nazwy i łańcucha
pozostaje aktywna. Testowe certyfikaty/klucz nie reprezentują realnego hosta.

Dokładne wykonane wyniki są w [progress.md](progress.md). Realny tailscaled,
policy/tagi, brak Funnel i dwie rzeczywiste maszyny wymagają odrębnej zgody oraz
odbioru E4.7/E10. Nie wykonywano ich w tej sesji. Dokumentacja nie zastępuje
tego odbioru ani wykonania całego planu.
