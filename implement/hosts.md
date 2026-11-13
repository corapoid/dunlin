# Rejestr hostów i granica zaufania workera

Obecny przekrój obejmuje rejestr, jednorazowy enrollment, oddzielne poświadczenie
workera, metadane capabilities, odwołanie i panel operatorski. Sam rejestr nie
nadaje prawa wykonania. Każdy wpis zwraca `network_verification: "unverified"` oraz
`execution_ready: false`, także po zatwierdzeniu przez administratora.

Zatwierdzenie oznacza akceptację wpisu i zgłoszenia w aplikacji. Nie jest dowodem
prywatności Serve, tożsamości węzła sieci, jakości sandboxa ani gotowości procesu.
Kanał WSS i worker CLI opisuje [worker-channel.md](worker-channel.md).
Zdalne ProjectBinding i opt-in execution leases/guardian/inbox/outbox są osobnymi
warstwami E4.2/E4.4: [remote-bindings.md](remote-bindings.md) i
[remote-session-plan.md](remote-session-plan.md). Raport REST albo badge
połączenia nie zastępuje ich świeżego Describe, grantów i przypiętego profilu.

## Własność i definicja

Rejestr należy do workspace. Wszystkie odczyty i zapisy browser API wymagają
aktualnego `ManageHosts`: roli owner/admin w tym workspace. Rola administratora
natywnego relay nie zastępuje członkostwa. Adapter browser ponownie sprawdza
urządzenie, OIDC i politykę kont hosta pod tą samą blokadą Store co operację.

Definicja zawiera nazwę, oczekiwany stabilny `node_id`, kanoniczny `serve_origin`
i politykę capabilities. Origin to wyłącznie małe litery
`https://host.tailnet.ts.net`, z implicit443; bez jawnego portu, slash, ścieżki,
userinfo, query, fragmentu lub IP. Walidacja składni nie wykonuje DNS ani nie
uwierzytelnia sieci. Tożsamość i origin istniejącego wpisu są niezmienne.

Polityka obejmuje dozwolone stałe identyfikatory harnessów i limit1–32 zadań.
Raport workera obejmuje protocol_version1, wersję, OS/architekturę, harnessy,
opis sandboxa i limit równoległości. Wszystkie pola są ograniczone, a nieznane
pola zaufania/komend są odrzucane. Raport może opisywać więcej niż lokalna polityka;
jej przecięcie ogranicza dane konfiguracyjne, ale nadal nie nadaje authority.
Maksymalnie64 hosty na workspace, licząc odwołane tombstones.

## Enrollment i odwołanie

1. Administrator tworzy oczekiwany wpis. Serwer zwraca jeden losowy bilet `dhe_`
   ważny600 sekund; w SQLite zapisuje jego SHA-256, nie wartość.
2. Dedykowany klient workera wymienia bilet raz na oddzielne poświadczenie `dhw_`.
   Bilet jest konsumowany atomowo (`IMMEDIATE` transaction). Status zmienia się
   z pending na claimed. To potwierdza posiadanie biletu, nie tożsamość Tailscale.
3. Administrator może jawnie zatwierdzić claimed. Metadane wpisu pozostają
   unverified; wykonanie wymaga osobnego bindingu, profilu, aktualnych grantów
   i odpowiedniego żywego producenta/transportu.
4. Odwołanie ustawia revoked/drain, usuwa oba hashe i podnosi enrollment epoch
   oraz credential revision. Nowy enrollment wymaga osobnej jawnej operacji CAS;
   stary bilet i credential nigdy nie odzyskują ważności.

Sekrety mają256 losowych bitów i różne prefiksy. Workertoken nie jest tokenem
użytkownika, browser cookie, native bearer ani service principal workflow.
Endpointy workera odrzucają Cookie i Origin; enrollment odrzuca także Authorization,
a report wymaga dokładnie jednego dedykowanego Bearer. Nie używamy credential
workera do nadawania członkostw/grantów, dostępu do plików ani modeli.

Wszystkie zmiany administratora mają `expected_revision`. Raport aktualizuje
oddzielny `capability_revision` i serwerowy last_seen; nie zmienia administracyjnej
rewizji CAS. Drain i status nie są dowodem zatrzymania procesu: ten przekrój nie
ma jeszcze zdalnego executora ani jego guardian.

Odwołanie pozostaje możliwe przy pełnym zwykłym audycie. W tej samej transakcji
z unieważnieniem powstaje obowiązkowy `team_host_revocations`: jeden ostatni
terminalny receipt na host. Zwykły audyt zachowuje wcześniejsze zdarzenia, jeśli
ma miejsce; pełny audyt blokuje dodatnie operacje, więc nie umożliwia pętli
reenrollmentu nadpisującej nieograniczoną historię. Niepowodzenie zapisu receipt
cofa transakcję. Odczyt potwierdzenia wymaga ManageHosts.
Dodatnie operacje rezerwują ostatnią wartość licznika na revoke; odwołanie
unieważnia credential również dla zastanego rekordu z licznikiem MAX.

## Endpointy

Podstawa browser: `/v1/web/workspaces/{workspace}/hosts`.

| Endpoint | Kontrakt |
| --- | --- |
| `GET /` i `GET /{host}` | metadane bez sekretów i hashy |
| `POST /` | HostDefinition → `{host,enrollment:{ticket,expires_at}}` |
| `POST /{host}/enrollment` | `{expected_revision}` → nowy jednorazowy bilet |
| `POST /{host}/approve` | `{expected_revision}` → zatwierdzony wpis |
| `POST /{host}/drain` | `{expected_revision,drain}` |
| `POST /{host}/revoke` | `{expected_revision}` → revoked |
| `GET /{host}/revocation` | `{revocation:null|ostatni terminalny receipt}` |

`POST /v1/worker/enroll` przyjmuje `{ticket,capabilities}` i zwraca
`{host_id,workspace_id,enrollment_epoch,credential_revision,credential}`.
`POST /v1/worker/report` przyjmuje dedykowany Bearer oraz
`{host_id,enrollment_epoch,credential_revision,capabilities}`. Jest to bootstrap
metadanych; nie implementacja live heartbeat, leases ani wykonywania pracy.
Nie istnieje produkcyjna pętla HTTP zastępująca docelowy kanał WSS.

## Panel

Zakładka Hosty pokazuje wyłącznie aktualnie uprawniony rejestr. Bilet istnieje
jedynie w pamięci odpowiedzi na mutację; nie trafia do URL ani browser storage.
Kopiowanie wymaga jawnego kliknięcia. GET nie odzyskuje biletu. Ręczne odświeżenie
usuwa go; push metadanych może zachować już posiadany bilet tylko dla tego samego
pending host/epoch i po świeżej kontroli dostępu. Utrata ManageHosts usuwa listę
oraz bilet także bez ręcznego odświeżania. Spóźnione odpowiedzi GET i mutacji
podlegają kontroli generacji. Niepewna mutacja nie jest automatycznie ponawiana.

Migracja29→30 dodaje rejestr i ograniczony terminalny receipt. Nie tworzy nowych
hostów ani grantów z istniejących lokalnych bindingów. Zachowuje opublikowane
profile agentów, piny sesji i wcześniejsze admission receipts.

## Odczytowa inspekcja lokalnego Serve

CLI jest osobnym narzędziem diagnostycznym, bez zmian bazy lub konfiguracji sieci:

```sh
dunlin-relay host inspect-local --socket /var/run/tailscale/tailscaled.sock \
  --node-id nEXPECTED --origin https://worker.tailnet.ts.net --listen 127.0.0.1:8787
```

Wymaga jawnie wybranego absolutnego Unix socketu lokalnego demona. Nie zgaduje
macOS GUI port/password, nie przechodzi na TCP i nie korzysta z proxy środowiska.
Peer socketu musi należeć do root lub bieżącego OS usera; lokalny użytkownik i
wybrany daemon stanowią granicę zaufania. Nie jest to kryptograficzne poświadczenie
zdalnego hosta.

Inspekcja ma wspólny deadline3s, limit1MiB odpowiedzi,16KiB bufora nagłówków
oraz64 nagłówki. Odczytuje tylko GET status bez peerów i GET serve-config;
sprawdza Self ID/DNS/Online/Expired przed i po konfiguracji. Wymaga background
HTTPS443 z pojedynczym root proxy do dokładnego numerycznego loopback backendu.
Funnel, foreground overlays, Services, overrides ścieżek, redirect/raw TCP,
nieznane tryby oraz odmienne cele są odrzucane. Nie przepisuje cudzych tras.
Wynik zawiera czas obserwacji i `application_authority:false`; nie aktualizuje
network_verification hosta i nie może być przyjęty z browser JSON jako dowód.
Odczyty status/config nie są atomową blokadą konfiguracji na przyszłość.

Kontrakty sprawdzono2026-11-11 w oficjalnej dokumentacji i upstream kodzie.
LocalAPI ServeConfig jest oznaczone jako niestabilne; nowe tryby wymagają
świadomej aktualizacji adaptera. W tej sesji użyto wyłącznie fixture Unix HTTP;
nie odczytywano ani nie zmieniano rzeczywistego tailscaled/tailnetu.

Źródła: [Serve](https://tailscale.com/docs/features/tailscale-serve),
[Funnel](https://tailscale.com/docs/features/tailscale-funnel),
[LocalAPI GET ServeConfig](https://github.com/tailscale/tailscale/blob/main/client/local/serve.go),
[status](https://github.com/tailscale/tailscale/blob/main/client/local/local.go),
[ServeConfig](https://github.com/tailscale/tailscale/blob/main/ipn/serve.go),
[Self/PeerStatus](https://github.com/tailscale/tailscale/blob/main/ipn/ipnstate/ipnstate.go).
Nagłówki tożsamości Serve nie są źródłem stable node identity urządzeń z tagami;
nie używamy ich do uwierzytelniania workera.

Dokładne wyniki testów i otwarte bramki są w [progress.md](progress.md).


## Bieżąca obserwacja i podgląd live — E4.5

Nowy `GET /v1/web/workspaces/{id}/hosts/{host}/health` odczytuje osobny registry
zaakceptowanych WSS lifetimes. Sam trwały rekord active nie tworzy obserwacji.
Scope/identity/epoch/current boot/expiry są weryfikowane w tej samej transakcji
co ManageHosts i zagregowane counts. Zwracane dane są observational; nie mają
konwersji do execution lease ani uprawnień do projektu. `/channel` zachowuje
dotychczasowy kontrakt historycznych diagnostyk.

`/health/live` wymaga cookie, zgodnych Origin/Host i jawnego subprotocol
`dunlin.host-health.v1`. Wysyła pełne snapshoty z własnym stream_id i rosnącym
sequence; reconnect zaczyna nowy stream. Wiadomości snapshot/heartbeat/unavailable
nie przyjmują komend wykonania. Źródłowy heartbeat odnawia zegar obserwacji tylko
przy nowej zatwierdzonej sekwencji, według receive-time i monotonicznego czasu;
późny SQL commit albo duplicate nie udaje świeżego połączenia.

Podgląd korzysta ze wspólnego browser connection budget64, input/frame1KiB,
output16KiB, send deadline5s i kontroli scope co500ms także przy niezmienionym
payloadzie. Serwerowy keepalive nie odnawia worker lease w browserze. UI czyści
obserwację przy wygaśnięciu, zerwaniu strumienia, zmianie scope albo odmowie
dostępu. Nie odpytuje REST cyklicznie i nie zapisuje danych w browser storage.

Wersja raportu capabilities1 i rzeczywiście negocjowany transport1/2/3/4 są
różnymi polami. Counts obejmują rezerwacje v3/v4, aktywne sesje/tury, niepewne
rezerwacje oraz oczekujące ACK. Nie zawierają owner/session IDs, tekstu promptów
ani transkryptów; są diagnostyką workspace operatora posiadającego ManageHosts.
Drain i stan transportu pozostają odrębne od source OS stop proof.

Frontend128, osobny typecheck, Vite, pełny runtime z16 kernel checks oraz
rzeczywisty Chromium v4 heartbeat mają PASS; Clippy snapshotu health także PASS.
Końcowa regresja registry Chromium PASS7,89s obejmuje enrollment, prawdziwy
HTTP claim/report workera, CAS/drain/revoke, utratę ManageHosts, viewer403 oraz
brak persistence tickets. E4.5 odebrano w zakresie panelu z izolowanymi fixtures.
Dokładne snapshoty i wyniki: [progress.md](progress.md). Nie stanowi to odbioru
rzeczywistego tailnetu, systemu macOS ani kont dostawców; te bramki są w E10.7.
