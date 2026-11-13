# Odtwarzalne sprawdzanie rzeczywistego runtime

Suite `runtime` uruchamia zbudowany `dunlin-relay` z `sandbox = true` i rzeczywistym
`/usr/bin/bwrap`. Ignorowany domyślnie test
`production_browser_acceptance_runtime_restart` używa lokalnego fixture ACP,
trwałej bazy i rzeczywistego procesu relay. Sprawdza restart i niepewny wynik
przerwanej pracy bez automatycznego ponowienia. Nie używa adaptera zastępującego
produkcyjny sandbox ani realnych kont dostawców. Nazwa testu ma historyczny prefiks
`browser`; ten przekrój steruje protokołem bez Chromium. Dziesięć zwykłych scenariuszy
browser pozostaje w oddzielnym suite `browser`, z domyślnymi zabezpieczeniami Dockera.

## Przygotowanie na dedykowanym hoście Linux

Wymagane są lokalny Docker, Python 3, kernel z AppArmor i obsługą nieuprzywilejowanych
user namespaces oraz `apparmor_parser`. Na Ubuntu 24.04 pakiet parsera to `apparmor`.
Te polecenia pobierają obrazy, narzędzia i zależności podczas jawnego przygotowania.
Nie przekazuj do nich poświadczeń, socketów innych usług ani katalogów z sekretami.

```sh
test "$(uname -s)" = Linux
test "$(cat /sys/module/apparmor/parameters/enabled)" = Y
docker info --format '{{json .SecurityOptions}}' | grep -q apparmor
sudo apt-get update
sudo apt-get install -y apparmor
docker build --tag dunlin-web-checks:rust-1.99 - < scripts/web-checks.Dockerfile
docker build --tag dunlin-web-checks:browser - < scripts/web-browser-checks.Dockerfile
docker build --tag dunlin-web-checks:browser-bwrap - < scripts/runtime-checks.Dockerfile
mkdir -p target/web-checks/cargo
docker run --rm --mount "type=bind,src=$PWD,dst=/repo,readonly" --mount "type=bind,src=$PWD/target/web-checks/cargo,dst=/cache" --workdir /repo --env CARGO_HOME=/cache dunlin-web-checks:rust-1.99 cargo fetch --locked
python3 scripts/prepare_runtime_sandbox.py --fetch --output target/web-checks/runtime-policies
sudo apparmor_parser -r target/web-checks/runtime-policies/runtime.apparmor
sudo grep -q '^dunlin-runtime-checks (enforce)$' /sys/kernel/security/apparmor/profiles
python3 scripts/test_web_sandbox.py --suite runtime --registry target/web-checks/cargo/registry --wall-seconds 900
```

Parser ładuje tylko nowy, nazwany profil `dunlin-runtime-checks`. Nie wyłączamy
AppArmor, nie zmieniamy profili usług hosta ani globalnych sysctl. Przy Dockerze
wewnątrz dedykowanej VM profil musi zostać załadowany w kernelu tej VM, gdzie działa
daemon. Opcja launchera `--docker-host unix:///…` wybiera lokalny socket bez zmiany
globalnego kontekstu. Samo wygenerowanie profili nie dowodzi, że kernel je przyjął.
`cargo clean` usuwa również pomocniczy katalog `target/web-checks`. Przed kolejnym
przebiegiem odtwórz zależności i polityki. Jeśli VM montowała usunięty katalog,
odśwież mount przez restart wyłącznie dedykowanej VM testowej; nie restartuj
innych profili ani usług użytkownika.

## Pochodzenie i zakres poluzowania izolacji testowej

Generator przypina źródła do
[Moby profiles 6049e40058759e8a1673ac1090ec6a7e36b9c9bd](https://github.com/moby/profiles/tree/6049e40058759e8a1673ac1090ec6a7e36b9c9bd).
Weryfikuje SHA-256 obu upstream profili i licencji Apache-2.0 przed zapisem.
Katalog wyjściowy zachowuje `seccomp.upstream`, `apparmor.upstream`, `license.upstream`
i `provenance.json`. Bez `--fetch` generator wymaga już obecnych, zgodnych źródeł.
Launcher ponownie sprawdza dokładne hashe wygenerowanych polityk; ich domyślna
lokalizacja to `target/web-checks/runtime-policies`.

[Docker opisuje seccomp jako allowlistę z domyślną odmową](https://docs.docker.com/engine/security/seccomp/).
Na potrzeby zagnieżdżonego bubblewrap dodajemy do przypiętej allowlisty wyłącznie
`clone`, `unshare`, `setns`, `mount`, `umount2`, `pivot_root`. Nazwany profil AppArmor
pochodzi z tego samego upstreamu i zastępuje odmowę `mount` zezwoleniami `mount`,
`pivot_root`, `userns`; zachowuje pozostałe ograniczenia proc/sys i sieci upstreamu.
`systempaths=unconfined` usuwa zewnętrzne maskowanie ścieżek potrzebnych wewnętrznemu
`/proc`. To realne osłabienie zewnętrznej warstwy testowej, konieczne do sprawdzenia
wewnętrznego sandboxa. Profil AppArmor i seccomp pozostają aktywne; kontener nie jest
`--privileged` i nie otrzymuje `CAP_SYS_ADMIN`.

Launcher nadal wymusza UID/GID 65534, `--cap-drop=ALL`, `no-new-privileges`,
`--network=none`, readonly root i snapshot źródeł, readonly cache zależności,
czyste środowisko przez `env -i`, brak montowania poświadczeń i socketu Dockera oraz
ograniczone tmpfs. Obowiązują limity CPU, RAM, PID, rozmiaru plików i czasu.
Suite runtime/server/browser mają limit3584MiB RAM. Runtime buduje artefakty
produkcyjnego relay i binarnego testu; rozbudowany zestaw testów server/browser
z klientem HTTP LocalAPI również przekroczył dawny limit3072MiB w kompilacji.
OOM w cgroup potwierdzono w logu kernela dedykowanej VM przed zmianą limitu. W VM
uwzględnij także pamięć kernela i demona Docker; limit kontenera nie jest jej
całkowitym budżetem. Ta różnica nie zmienia profili izolacji ani zestawu testów.
Runtime/server/browser/macos-profile mają scratch tmpfs1536MiB. Runtime buduje
najpierw test binary, a potem zwykły executable w tym samym target. Dawniejszy
przebieg budujący najpierw executable po rozbudowie worker metadata dwukrotnie
zakończył link testu SIGBUS przy scratch1280MiB,
bez OOM; kontrolowany przebieg z1536MiB przeszedł przy niezmienionym RAM3328MiB.
Launcher wypisuje df/du/memory.events po build, po worker tests i przy błędzie
ich kompilacji. To limit RAM tmpfs, nie zapis hostowego Rust target.

Kompilacje testowe używają8 codegen units (browser/macos-profile:64,
server/runtime:256, z strip=symbols) przy jednym Cargo build job, aby
ograniczyć szczyt LLVM; nie włączają incremental ani debug info. W przekroju WSS
po potwierdzonymOOM przy jednym dużym codegen unit zmiana obniżyła peak dookoło
2,5–2,7GiB przy zachowanym limicie3328MiB. Rozbudowa testów MCP/CIMD ponownie
osiągnęła OOM przy64 i128 CGU ustawionych globalnie, a także przy64 dla relay
i8 dla zależności. Ostatnia próba wykazała1,2GiB artefaktów na tmpfs.
Limit server zwiększono o256MiB do3584MiB; browser i runtime kompilujące ten sam
duży test relay używają tego samego budżetu. Pierwszy test brokera MCP osiągnął
3546923008B bez OOM, powyżej dawnego limitu runtime3328MiB. Scratch i granty pozostały
bez zmian. Server po każdym przebiegu raportuje także df/du katalogu kompilacji.
Po dodaniu writer lease testowe binarium runtime dwukrotnie osiągnęło OOM przy
CGU64 i3584MiB. Runtime zmieniono na128 CGU bez zwiększenia RAM/scratch/CPU,
bez zmiany zakresu testów i izolacji; header zapisuje codegen_units. Bieżący
wynik i peak wymagają osobnego odbioru w progress.md, a tooling contract
nie zastępuje rzeczywistej kompilacji ani pełnego runtime regression.
Przed testem restartu suite buduje
produkcyjny relay i wykonuje pełny zestaw relay/identity/runtime/terminal/usage/workspace,
w tym testy worker_, lokalnych zgód i typed_target_http. Ten ostatni sprawdza
odmowę Remote przed FS capacity/receipt i dokładny Local revision pin; nie
wykonuje zdalnej pracy. Runtime filter `production_browser_acceptance_runtime_`
obejmuje actual binary restart, ACP metadata kernel smoke i linked interactive
permission/revoke/regrant: prawdziwy bwrap, linked checkout, RO/RW metadata roots,
odmowę source workspace i guardian revoke. Ostatni test dołączono do bieżącego
przekroju; jego wynik należy sprawdzić w konkretnym logu, nie w tej instrukcji.
Zwykły browser suite pomija cały ten prefix; nie podszywa się pod kernel acceptance.
Mac privacy profile ma testy tekstowe, lecz ten Linux przebieg nie dowodzi jego
wykonania przez kernel macOS. Sam test binarnego CLI
odrzuca FIFO oraz błędny sekret stdin bez jego wypisywania. Nie używa realnego
LocalAPI, dostawców ani credentials użytkownika.

Najpierw musi przejść rzeczywista próba namespace/mount `bwrap … /bin/true`, potem
budowa binarnego relay i test restartu. Odmowa kernela lub brak polityki kończy suite
błędem, bez przełączenia na atrapę albo pominięcia testu.

## CI i interpretacja wyniku

Job `runtime-sandbox` w `.github/workflows/ci.yml` wykonuje przygotowanie na
Ubuntu 24.04 i uruchamia suite offline. Jest osobną wymaganą zależnością zbiorczego
jobu `test`. Job `app-web` jawnie pomija cały prefix
`production_browser_acceptance_runtime_` i zachowuje zwykłe scenariusze browser. Nie zmieniamy ich LSM/seccomp.

Każde wykonanie launchera wypisuje identyfikator zawartości obrazu, hash snapshotu
źródeł, limity oraz wynik testów. Tagi bazowych obrazów i pakiety apt nie są
przypięte digestem, więc odtworzenie przygotowania może dać inne narzędzia; źródła
polityk i ich wynikowe hashe są przypięte. Zachowaj log konkretnego wykonania jako
dowód. Ta instrukcja i obecność jobu nie oznaczają PASS na CI ani odbioru produkcji.
Test lokalnego fixture nie zastępuje zgody i odbioru realnego harnessu, IdP,
keyringu, tailnetu ani zdalnych hostów.

Runtime ma per-file RLIMIT_FSIZE2GiB, ponieważ workspace import preflight test
używa dwóch plików sparse512MiB+1 oraz pojedynczego sparse1GiB+1 i odmawia
transferu przed kopiowaniem. Poprzednie
128MiB powodowało SIGXFSZ25 zamiast sprawdzić odmowę importu. Limit długości
pliku nie zwiększa RAM ani pojemności tmpfs: scratch1536MiB, /tmp16MiB i RAM3328MiB
pozostają takie same. Suite workspace używa tego samego limitu logicznej długości dla tych samych
sparse fixtures. Browser/server/macos-profile wymagają także per-file2GiB, ponieważ duży test
ELF relay przekroczył128MiB (zmierzony SIGXFSZ25 podczas linkowania); nie zmienia
to RAM, tmpfs, PID ani LSM. Pozostałe suites zachowują per-file128MiB. Fixture nie wymaga
alokacji ani zapisu1GiB na dysku hosta.

## Pion trwałej sesji Remote

Nowy test `production_browser_acceptance_runtime_remote_session_journal` sprawdza
produktowy REST z przypiętym profilem/powiązaniem i Idempotency-Key, rzeczywisty
AHP WebSocket, prywatny fixture TLS/WSS oraz jeden proces ACP w bubblewrap dla
kolejnych tur. Sprawdza zachowanie kontekstu, idle renewal, projekcję Journalu,
identyczny retry admission oraz stop/ACK. Jego wynik należy zapisać oddzielnie;
kompilacja lub wcześniejszy test samego transportu nie dowodzi tego pionu.

```sh
python3 -I scripts/test_web_sandbox.py --suite runtime --rust-test production_browser_acceptance_runtime_remote_session_journal --docker-host unix:///Users/corapoid/.colima/dunlin-checks/docker.sock --tmp-root target/web-checks --wall-seconds 900
```

Focused runtime buduje rzeczywisty relay i binarny test, lecz pomija zwykłą
pełną regresję. Po jego odbiorze uruchom także suite runtime bez filtra oraz
oddzielne frontend/typecheck/build/browser checks dla zmienionego formularza.
Testy używają kontrolowanych agentów i lokalnych daemon/proxy fixtures. Nie są
odbiorem prawdziwego harnessu, IdP, tailnetu ani dwóch fizycznych hostów.

Opcjonalny odbiór tego samego pionu z produkcyjnym formularzem i Chromium:

```sh
python3 -I scripts/test_web_sandbox.py --suite runtime --runtime-browser --rust-test production_browser_acceptance_runtime_remote_session_journal --node-modules target/web-checks/node-deps/node_modules --browser-node-modules target/web-checks/browser-node-deps/node_modules --docker-host unix:///Users/corapoid/.colima/dunlin-checks/docker.sock --tmp-root target/web-checks --wall-seconds 900
```

Flaga `--runtime-browser` jest jawna i dozwolona wyłącznie dla runtime: pełnej
regresji, dokładnego filtra journal, dokładnego filtra product MCP albo prefixu
`production_browser_acceptance_runtime_remote_`, który uruchamia oba browser branches. Buduje web z istniejących readonly caches, używa
Chromium z obrazu runtime i dodatkowego scenariusza `remote-session.mjs`.
Sprawdza formularz Remote, utratę prawdziwego HTTP202, identyczny retry, dwie tury
po przeładowaniu strony, trzeci aktywny prompt, dwie pozycje kolejki, edycję
i reorder oraz AHP cancel całej sesji. Wymaga confirmed remote stop przy Unknown
wyniku przerwanej tury, końcowego worker ACK i braku wykonania queued promptów. Zwykły runtime nie wykonuje tej
opcjonalnej gałęzi. Header logu podaje `chromium=remote-session-production-ui`;
sukces samej gałęzi jest dodatkowo potwierdzany po sprawdzeniu worker SQL i
zatrzymania procesu potomnego, nie samym kodem wyjścia przeglądarki.

Chromium potrzebuje osobnego limitu PID256 zamiast domyślnego runtime96. CPU1,
RAM3584MiB, scratch1536MiB, AppArmor/seccomp, brak capabilities, readonly host
mounts i `network=none` pozostają bez zmian. Nie montujemy socketów hosta ani
credentials. Launcher ma test walidacji flag/filtra i generowania tej konfiguracji.
Wynik tego nowszego scenariusza wymaga odrębnego zapisu w progress.md.

Zdalny MCP ma dodatkowy rendered case `remote-mcp-session.mjs`: prywatny wybór
konta, reset targetu, utrata prawdziwej REST odpowiedzi i ten sam retry, zgoda
AHP na parametry narzędzia, desktop/mobile i sprawdzenie stopu/ACK w obu bazach.
Używa kontrolowanego HTTP OAuth/MCP, nie realnego konta dostawcy. Wspólny odbiór
obu pionów z pozostałą zdalną regresją:

```sh
python3 -I scripts/test_web_sandbox.py --suite runtime --runtime-browser --rust-test production_browser_acceptance_runtime_remote_ --node-modules target/web-checks/node-deps/node_modules --browser-node-modules target/web-checks/browser-node-deps/node_modules --docker-host unix:///Users/corapoid/.colima/dunlin-checks/docker.sock --tmp-root target/web-checks --wall-seconds 900
```

Wymagany CI runtime-sandbox uruchamia pełny runtime z tą jawną flagą i readonly
`apps/web/node_modules` oraz `apps/web/e2e/node_modules`, przygotowanymi przez
locked npm ci --ignore-scripts. Nie zwiększa to limitów CPU/RAM/scratch/wall
ani nie zmienia polityki kernelowej. Lokalny runtime11 z oboma branches jest
odebrany w progress.md; samo skonfigurowanie jobu nie zalicza GitHub Actions.


Test `production_browser_acceptance_runtime_remote_session_process_restart`
przygotowuje osobne procesy kontrolera i workera na trwałych bazach.
Wykonuje dwa warianty: Journal commit przed ACK wire oraz worker SQL ACK commit
przed doręczeniem potwierdzenia. Rodzic czeka na test-only marker i sprawdza
obie bazy przed SIGKILL; nie wybiera losowego momentu awarii. Są to role
w prywatnym binarnym runnerze testowym, wywołujące wspólny produkcyjny startup
remote service oraz rzeczywisty worker client/runtime. Statyczne CA, daemon
LocalAPI/proxy i ACP peer pozostają fixture; nie dodano produkcyjnego bypassu
TLS ani flagi crash injection. Bounded bootstrap i fixture credential przechodzą
prywatnym stdin, bez sekretów w argumentach. Test zabija SIGKILL oba procesy
podczas aktywnej tury, otwiera te same bazy i wymaga nowych boots, historycznego
retry bez wykonania, Unknown/Queued, source stop/ACK oraz dopiero jawnej nowej
sesji. Rozszerzona gałąź zatrzymuje cały kontroler SIGSTOP przez ponad oba
dziesięciosekundowe lease, wymaga samodzielnego zakończenia workera i po SIGCONT
sprawdza brak ożywienia starych tur. Sygnalizuje wyłącznie własny proces przez
pidfd, bez ryzyka ponownego użycia numeru PID. Nie jest dowodem wydania ani
wykonania na dwóch fizycznych hostach.
Obecność scenariusza nie oznacza PASS; konkretny wynik należy sprawdzić w progress.md.

```sh
python3 -I scripts/test_web_sandbox.py --suite runtime --rust-test production_browser_acceptance_runtime_remote_session_process_restart --docker-host unix:///Users/corapoid/.colima/dunlin-checks/docker.sock --tmp-root target/web-checks --wall-seconds 900
```


Połączone scenariusze zdalnego produktu MCP można uruchomić bez Chromium:

```sh
python3 -I scripts/test_web_sandbox.py --suite runtime --rust-test production_browser_acceptance_runtime_remote_mcp_product_ --docker-host unix:///Users/corapoid/.colima/dunlin-checks/docker.sock --tmp-root target/web-checks --wall-seconds 900
```

Filtr obejmuje poprawny przepływ, odmowę zgody, utratę odpowiedzi HTTP po
przyjęciu tools/call, disconnect konta przez REST podczas trwającego HTTP oraz
wymuszony terminal-vs-renew. Ten ostatni uwalnia prawdziwą odpowiedź HTTP
po przygotowaniu RenewTurn i czeka na rzeczywisty source terminal przed
wysłaniem RPC; zachowuje wcześniejsze packet loss/replay/ACK faults.
Dwa ostatnie przypadki sprawdzają Unknown, jedno rzeczywiste wywołanie,
stop/ACK2 i reservation0, brak efektu w checkout oraz brak replay zgody/Submit.
Odmowa jest odzyskiwalnym błędem narzędzia: ta sama sesja kończy dwie tury,
a tools/call nie wychodzi do dostawcy. Pierwszy odbiór wykrył błędne oczekiwanie
testowego helpera dla canonical odmowy; po poprawce pełny remote_ z oboma
Chromium branches przeszedł14/14. Dalsza regresja wykryła sporadyczny Conflict
na granicy terminal/renew. Wymuszony przypadek odtworzył StaleAttempt i przerwanie
w4,77s przed poprawką; po uzgodnieniu oryginalnego streamu i świeżej kontroli
terminal/authority pełny remote_ z oboma Chromium branches przeszedł15/15.
Dowody i zakres fixtures w progress.md.

Form MRTR dodaje cztery przypadki remote_mcp_product_forms*: accept w dwóch
rundach, decline, cancel i disconnect konta podczas oczekiwania. Pełny remote_
z runtime-browser ma19 scenariuszy i dodatkowe gałęzie Chromium dla wszystkich
trzech odpowiedzi formularza. Sprawdza wszystkie płaskie typy,0/false/empty enum,
brak automatycznych defaults, property __proto__, korektę integer/URI przez
nowy clientSeq, canonical rejection bez utraty WS, brak requestState/bearer w DOM
i brak żądań browsera do URI z formularza. Rust potwierdza potem liczbę
HTTP initial/continuation, tury/efekty, stop/ACK i zatrzymanie potomka z FD.
Przy zapisie PNG z logu należy przypisać linie REMOTE_MCP_IMAGE_DESKTOP/MOBILE
do bieżącego testu forms/forms_decline/forms_cancel/pipeline, zamiast nadpisywać
wszystkie tym samym plikiem. W evidence są osobne accept/decline/cancel/tool PNG.
Źródła i granice odbioru w progress.md. Nadal nie jest to realny provider/tailnet.

## Izolowane testy klienta MCP

Suite `mcp` wykonuje `cargo test -p dunlin-mcp-client -p dunlin-http -p dunlin-vault --offline --locked`,
a po jego sukcesie all-targets Clippy `-D warnings` dla trzech pakietów.
Obejmuje adapter prywatnego SecretStore; testy używają fikcyjnych sekretów,
bez odczytu/zapisu rzeczywistego keyringu.
Używa standardowych polityk kontenera: readonly snapshot/cache, UID65534,
network none, cap-drop ALL, PID96, RAM1536MiB, scratch768MiB i file128MiB.
Opcjonalny `--rust-test` przyjmuje wyłącznie ograniczony identyfikator Rust.
Nie otrzymuje tokenów dostawców ani dostępu do zewnętrznych serwerów MCP.
Obejmuje również kompilację/walidację JSON Schema, ograniczenie rozwinięć
referencji i typowane odpowiedzi form MRTR. Testy sprawdzają brak HTTP dla
błędnych argumentów/odpowiedzi i brak pobierania zewnętrznych schematów.
Obejmuje publiczny profil CIMD: ścisłe adresy dokumentu, wymagane wsparcie AS,
PKCE/code exchange, refresh bez client secret oraz odrębny durable binding.
Suite server sprawdza eksport publicznego dokumentu bez keyringu/sieci i pełną
ścieżkę konfiguracja → start/callback → prywatny katalog MCP z lokalnym AS.
Publiczny hosting i rzeczywiste konto dostawcy pozostają osobnym odbiorem.
Suite sessions obejmuje hostowe zgody MCP: niezmienny pin wywołania, odmowę
raportów ACP w zarezerwowanej przestrzeni, jednorazowe consume, równoległe
połączenia SQLite, atomowy zapis wyniku oraz unknown po crash/recovery.
Pełny server suite uruchamia ten sam zestaw dziennika razem z regresją relay.
Nowe zależności muszą być wcześniej pobrane i rozpakowane w registry cache;
cache pozostaje readonly podczas testu. Profil schematów opisano w
[kontrakcie](mcp-client.md#walidacja-schematów-narzędzi).

```sh
python3 -I scripts/test_web_sandbox.py --suite mcp --docker-host unix:///Users/corapoid/.colima/dunlin-checks/docker.sock --tmp-root target/web-checks --wall-seconds 300
```

Panel OAuth ma osobny scenariusz produkcyjnego UI w Chromium:

```sh
python3 -I scripts/test_web_sandbox.py --suite browser --browser-test mcp_oauth --node-modules target/web-checks/node-deps/node_modules --browser-node-modules target/web-checks/browser-node-deps/node_modules --docker-host unix:///Users/corapoid/.colima/dunlin-checks/docker.sock --tmp-root target/web-checks
```

Relay i lokalna strona zgody używają różnych sites (127.0.0.1 i127.0.0.2).
Test sprawdza code/PKCE, brak Strict session cookie i obecność Lax flow cookie
na callbacku, powrót do nazwanego połączenia, utratę odpowiedzi start/disconnect,
brak automatycznego retry oraz spóźnioną odpowiedź po zamknięciu panelu.
Serwer dodatkowo sprawdza liczbę operacji i prywatnych zapisów oraz końcowy stan SQL.
Scenariusz używa też przycisku Sprawdź narzędzia: wymaga potwierdzonej wersji MCP,
jednego odczytu katalogu przez broker i pokazania nazwy narzędzia. Fixture odrzuca
każde tools/call; sam katalog nie jest dowodem wykonania narzędzi w runtime.
Nie korzysta z realnego providera ani keyringu. Limity browser RAM3584MiB,
scratch1536MiB i PID256 pozostają bez zmian. Log zawiera obrazy PNG zakodowane
w liniach MCP_UI_IMAGE_DESKTOP/MOBILE; przy zapisie dowodu można wydzielić je
do osobnych plików, zachowując tekstowy wynik testu. Obecność scenariusza nie
oznacza PASS; wynik i snapshot należy sprawdzić w progress.md.

## MCP OAuth revoke — produkcyjny helper i Chromium

Dedykowany odbiór UI odwołania wymaga wariantu runtime, ponieważ używa rzeczywistego
binarium relay jako facade MCP oraz Linux sandboxa. Sam zwykły wariant browser
korzysta z bezpośredniego kontrolowanego adaptera broker IPC. Jawny filtr:

```sh
python3 -I scripts/test_web_sandbox.py --suite runtime --rust-test production_browser_acceptance_runtime_mcp_oauth_revocation --runtime-browser --node-modules target/web-checks/node-deps/node_modules --docker-host unix:///Users/corapoid/.colima/dunlin-checks/docker.sock --tmp-root target/web-checks --wall-seconds 900
```

Scenariusz obejmuje logowanie u kontrolowanego AS, prywatne konta w Builderze,
rzeczywisty tools/call po zgodzie AHP oraz odwołanie obu zachowanych generacji.
Driver gubi odpowiedź REST dopiero po wykonaniu /revoke, wymaga odświeżenia
statusu przez GET i sprawdza, że UI nie wysłał drugiego żądania. Rust wymaga
czterech AS revoke POST (refresh/access dla dwóch kopert), disabled connection,
trwałego receipt confirmed_known_tokens i zachowania prywatnego SecretStore.
Nie oznacza to odwołania całego grantu AS ani lokalnego usunięcia sekretów.

Aktualny przyrost writer lease ma odbiór wybranego Chromium case przy CGU128
bez OOM i bez zwiększenia RAM/scratch/CPU. Dwa wcześniejsze CGU64 OOM oraz błąd
nowej obserwacji testowej są zachowane oddzielnie od zaliczonego przebiegu.
[Manifest odbioru](evidence/2026-11-13-mcp-writer-lease/verified-source.json)
opisuje dokładny zakres i różnicę ignorowanego fixture; nie jest to pełny runtime
regression ani rzeczywisty GitHub Actions run.

Po dodaniu write receipt także server CGU64 osiągnął potwierdzony OOM przed
uruchomieniem testów. Próbę serwera z CGU128 wykonano przy tym samym RAM3584MiB,
scratch1536MiB i jednym Cargo job; także zakończyła się OOM przed testami.
Kontrakt launchera sam nie potwierdza działania testów.

Rozbudowa write receipt przekroczyła limit serwera także przy CGU128. Aktualny
server/runtime używa CGU256 i strip=symbols, przy niezmienionych3584MiB RAM,
1536MiB scratch, CPU1 i jednym Cargo job. Zmiana nie wyłącza debug assertions,
nie podnosi poziomu optymalizacji i nie filtruje testów; usuwa tablice symboli
z wynikowych binariów. Jej odbiór i rzeczywiste memory.events są w progress.md.

Aktualny [odbiór write receipt](evidence/2026-11-13-mcp-write-receipt/verified-source.json)
ma pełny backend846 i wybrany Chromium/helper case przy CGU256+strip, bez OOM
oraz bez memory.events.max. Scratch kończy na62%/64%. Źródło jest identyczne
z MCP153/Clippy; normalny frontend pozostał bez zmian. To nie odbiór wszystkich
browser/macOS-profile/runtime branches ani rzeczywistego GitHub Actions run.


Migracja47 retencji zachowuje powyższe budżety CGU256/strip,3584MiB RAM,
1536MiB scratch, jeden Cargo job i CPU1. Bieżący backend851 oraz wybrany
Chromium/helper PASS na source895b6ab3; memory.events.max i OOM0. Test sprawdza
odzyskaną pojemność256 i historię2 rekordów po cleanup. Test768 zapisów z
restartami i ograniczeniem historii jest osobnym testem Store w pełnym backendzie.
Dowody są w [progress.md](progress.md); realny AS/keyring/tailnet oraz native
macOS pozostają osobnymi odbiorami.


## MCP URL elicitation — zdalny runtime i Chromium

```sh
python3 scripts/test_web_sandbox.py --suite runtime --rust-test production_browser_acceptance_runtime_remote_ --runtime-browser --node-modules target/web-checks/node-deps/node_modules --tmp-root target/web-checks --docker-host unix:///Users/corapoid/.colima/dunlin-checks/docker.sock --wall-seconds 900
```

Bieżący [odbiór URL](evidence/2026-11-13-mcp-url-input/verified-source.json)
obejmuje23 zdalne acceptance,738 pozostałych testów odfiltrowano. Cztery nowe
cases to URL accept z2 ręcznymi rundami, decline, cancel i disconnect konta
podczas oczekiwania na URL input. W tym samym przebiegu przeszły wcześniejsze
form i ich negatywne przypadki. Produkcyjny UI i helper używają kontrolowanego
OAuth/MCP/ACP, a zewnętrzna strona HTTPS jest przechwytywaną fixture Playwright.
Nie jest to prawdziwa strona dostawcy, tailnet ani pełna regresja runtime/E10.

Chromium sprawdza brak prefetch, tokenów/cookies/referer na docelowym żądaniu,
brak opener, osobne gesty otwarcia i akceptacji oraz czytelne desktop/mobile
bez poziomego overflow. Dane strony nie wracają jako answers/content. Po
odłączeniu nie ma kontynuacji i input jest anulowany. Runtime peak3669221376B,
max0/OOM0, scratch64%; budżety CGU256/strip/RAM/scratch/CPU pozostają bez zmian.
Launcher dopuszcza również wąski filtr runtime_remote_mcp_product_urls z browser;
kontrakt tego wyboru ma testy, ale powyższy wynik pochodzi z szerszego remote_.


## Operatorowy legacy OAuth — produkcyjny CLI

```sh
python3 scripts/test_web_sandbox.py --suite runtime --rust-test production_browser_acceptance_runtime_mcp_oauth_legacy_recovery --tmp-root target/web-checks --docker-host unix:///Users/corapoid/.colima/dunlin-checks/docker.sock --wall-seconds 900
```

To odbiór CLI w gotowym produkcyjnym executable, bez runtime-browser i bez
Chromium. Na syntetycznej bieżącej bazie uruchamia show/recover z pustym env,
sprawdza konflikt writer lease, revision3/unknown, zachowanie referencji i
odrzucenie starego przeglądu. Nie wykonuje sieci/secret-store I/O ani migracji
prawdziwej bazy. Nie jest dowodem zatrzymania starego niechronionego workera:
warunek shutdown deklaruje operator po sprawdzeniu własnych usług.

[Odbiór](evidence/2026-11-13-mcp-legacy-recovery/verified-source.json):CLI1 PASS,
768 pozostałych testów odfiltrowano,0,10s po build; peak3563261952B/max0/OOM0,
scratch64%,570060KiB wolnego. Pełny backend860/MCP153/Clippy ma ten sam źródłowy
snapshot;7 nowych cases Store/parsera sprawdza również stale digest/ABA,
audit rollback/full, nieaktywnego ownera i odrzucenie niebezpiecznych plików.
