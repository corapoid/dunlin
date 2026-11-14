# Wersjonowane profile agentów

Definicja agenta należy do projektu zespołu. Szkic jest edytowany przez CAS
(`expected_revision`), a osobna publikacja tworzy niezmienną wersję i digest
SHA-256 kanonicznego JSON zapisanego przez serwer. Ponowienie publikacji tej samej
rewizji zwraca tę samą wersję. Edycja ani nowa publikacja nie zmieniają istniejących
sesji. Zdarzenia audytu wskazują agenta przez `resource_id`; `target_id` nadal
wskazuje użytkownika zgodnie z wcześniejszym FK.

Definicje i opublikowane wersje odczytuje użytkownik z `ViewProject`. Każdy zapis
oraz publikacja wymagają aktualnego `ManageProject`. Serwer ponownie sprawdza
urządzenie, użytkownika, politykę hosta i OIDC w tej samej sekcji Store, w której
wykonuje operację. Limity wynoszą 128 agentów na projekt i 128 wersji na agenta.

## Kontrakt i wykonanie

`schema_version: 1` obejmuje nazwę, opis, identyfikator harnessu, dokładne wire IDs
modelu i trybu ACP, instrukcje, osadzone skills oraz limity czasu i wyjścia.
Harness to jeden z `opencode`, `codex`, `claude`; profil może uruchomić wyłącznie
harness aktualnie wybrany przez administratora serwera. Własna komenda hosta,
która nie odpowiada dokładnie katalogowi, nie jest zgadywana jako któryś z tych
identyfikatorów. Istniejące API v1 nadal obsługuje jej wcześniejsze zachowanie.

Model i tryb są jawną konfiguracją administratora. Katalog nie sugeruje, że zostały
już sprawdzone u dostawcy. Runtime negocjuje ACP, ustawia obie opcje i sprawdza
potwierdzony wynik przed oznaczeniem harnessu jako gotowego. Nieobsługiwana opcja
przerywa start bez wysłania promptu. Testy używają lokalnego fixture, bez modeli,
poświadczeń lub dostępu do dostawców.

Skill zawiera `id`, jawny `version` i literalne `instructions`. Cała lista jest
kopiowana do niezmiennej wersji agenta; aktualizacja wersji skill wymaga nowego
szkicu i publikacji. Nie ma rozwiązywania ścieżek, pobierania URL ani wykonywania
pakietów. Maksymalnie 8 skills, po 8 KiB instrukcji; instrukcje profilu i skills
łącznie mają najwyżej 16 KiB. Definicja ma limit 64 KiB zakodowanego JSON;
endpointy zapisu mają dodatkowy limit transportu 72 KiB (opakowanie CAS).
Czas wynosi 1–1800 sekund, budżet wyjścia 1 KiB–4 MiB.

Instrukcje profilu oraz skills z nagłówkami `[Skill id@version]` poprzedzają tekst
każdego promptu ACP, oddzielony nagłówkiem `## User request`. Jest to kontekst
tekstowy, nie obietnica obsługi roli systemowej przez harness. Transkrypt i kolejka
przechowują oryginalną treść użytkownika. Nie dodajemy niejawnej tury inicjującej.
Schema2 dodaje opcjonalną allowlistę mcp: do32 wpisów dla najwyżej8 providerów,
każdy z provider_id, dokładną nazwą narzędzia i definition_digest. Nie przyjmuje
wildcards, powtórzeń provider/name, endpointów, connection IDs ani sekretów.
Schema1 nadal jest obsługiwany, ale nie może zawierać niepustej polityki MCP.
Pusta/nieobecna lista jest serializowana przez Rust jako brak pola, zachowując
wcześniejsze kanoniczne bajty i digest opublikowanych profili v1. Aktualizacja
polityki tworzy nową wersję agenta; stare publikacje pozostają niezmienne.

Agent Builder pozwala zapisać/opublikować taką listę. Przypięcia można skopiować
z podglądu katalogu prywatnego połączenia MCP. Wspólny profil nie wybiera konta
użytkownika; prywatne connection/revision mają należeć do admission sesji.
allows_mcp jest dokładnym porównaniem provider/name/definition digest, bez
wnioskowania uprawnień ze skills, instrukcji lub adnotacji dostawcy.

Lokalny runtime Unix ma broker MCP i prywatne powiązania połączeń sesji.
Żądanie profilu może dodać mcp_connections jako listę
provider_id/connection_id/revision. Wymagany jest dokładnie jeden własny, aktywny
connection dla każdego providera profilu; nie ma automatycznego wyboru konta.
Powiązania i konfiguracja dostawców wchodzą w fingerprint utworzenia, poza
wspólną definicją agenta. Puste powiązania zachowują wcześniejsze bajty żądania.
Wspólny binder przyjmuje cały zwalidowany profil. Przygotowany prywatny zapis
schema1 (wersja zapisu, nie profilu agenta) ma limit8KiB i zawiera owner,
workspace, digest profilu oraz posortowane provider/connection/revision/configuration
pins. Nie zawiera endpointów, tokenów ani referencji vault. Odczyt ponownie
sprawdza scope, profil, konfigurację operatora i bieżący stan kont w SQL; samo
JSON nie tworzy BoundSession ani zgody na wykonanie. Ponowna autoryzacja po
odłączeniu nie przywraca starej rewizji. Lokalny admission korzysta ze wspólnego
bindera. Controller SQL ma osobny admission zapisujący ten format w prywatnych
pinach i odrębne open v5, bez danych kont w komendzie workera. Publiczny endpoint
i Actor nadal odmawiają MCP do czasu podłączenia scoped authority/producenta.
To uzupełnienie kontroli projektu/device/lease, nie ich zamiennik.
Nowy scoped guard ma osobną live walidację zapisu: odświeżanie tokena może
pozostać w refreshing wewnątrz cutu OAuth. Nowy admission nadal wymaga connected;
dostęp do konkretnej generacji tokena rozstrzyga credential journal.
Hostowa fabryka producenta v5 odtwarza te prywatne powiązania tylko dla
oryginalnego żywego aktora i łączy je ze wspólnym brokerem oraz scoped authority.
Broker wymaga zapisanego native receipt i aktywnej tury SDK przed zgodą/wywołaniem;
krótkie oczekiwanie na import receipt nie daje uprawnień. Callback zachowuje
poświadczenia na serwerze i respektuje limity odpowiedzi/deadline sesji.
Fabryka nie jest jeszcze wywoływana przez publiczne remote admission/Actor.
Katalog API zgłasza mcp_sessions_supported wyłącznie na Unix przy włączonym
experimental_interactive_runtime i skonfigurowanych providerach. Formularz pobiera
prywatną listę dopiero dla wybranego profilu MCP, wymaga jawnego wyboru i przypina
rewizje także przy ponowieniu po utracie odpowiedzi. Wykonanie wymaga zgodnego
katalogu oraz jednorazowej zgody na każde wywołanie; [kontrakt brokera](mcp-client.md#lokalny-broker-wykonania-mcp).

Zdalny controller, DTO v3/v4 i bezpośredni worker session engine nadal odmawiają
profili MCP. Zdalne proxy, A2A, ogólne scoped secrets i capability discovery
pozostają osobnymi punktami E5. Dowody testów i granice odbioru są w progress.md.

## API przeglądarkowe

Podstawa ścieżek: `/v1/web/workspaces/{workspace}/projects/{project}`.

| Metoda i ścieżka | Wynik lub żądanie |
| --- | --- |
| `GET /agents` | `{drafts, can_edit}` |
| `POST /agents` | definicja → nowy szkic |
| `GET /agents/{agent}` | bieżący szkic |
| `PUT /agents/{agent}` | `{expected_revision, definition}` → nowa rewizja szkicu |
| `POST /agents/{agent}/publish` | `{expected_revision}` → niezmienna wersja |
| `GET /agents/{agent}/versions/{version}` | konkretna opublikowana wersja |
| `GET /agent-profiles` | `{profiles, can_edit, configured_harness}`; ostatnia publikacja każdego agenta |
| `POST /interactive-sessions` | `{profile:{agent_id, version, digest}}` → historyczne przyjęcie |

Tworzenie profilu sesji zachowuje kontrakt `Idempotency-Key` i oczekiwanej tożsamości
przeglądarki. Serwer rozwiązuje dokładną wersję pod aktualnymi uprawnieniami,
sprawdza digest i ujmuje profil w fingerprint żądania. W jednej transakcji zapisuje
sesję, lease, historyczny receipt i `interactive_session_profiles`. Pin istnieje
również przy błędzie startu przed `harness_ready`; późniejszy event ready zawiera
potwierdzony profil. GET diagnostyki sesji udostępnia pin pod aktualnym ACL.
Retry ani recovery nie uruchamiają nowego procesu.

Starszy jawny payload `{model, mode, max_seconds, max_output_bytes}` jest zachowany.
Jego fingerprint nie otrzymał nowych pól, a prompt bez profilu pozostaje bez
prefiksu. Nie tworzymy domyślnego agenta ani nie przypisujemy profilu historycznym
sesjom. Migracja 28→29 dodaje nowe tabele i opcjonalny identyfikator zasobu audytu,
zachowując wcześniejsze przyjęcia i dane użytkowników.

## Interfejs i granice odbioru

Zakładka Agenci pozwala wybrać zespół/projekt, edytować szkic i osobno go opublikować.
Formularz w Sesjach wybiera opublikowany profil i przypina wersję z digestem.
Niepewny wynik tworzenia jest zachowany między widokami; jawne ponowienie wysyła
dokładnie ten sam klucz, body i oczekiwaną tożsamość. Historyczne potwierdzenie nie
odblokowuje promptu; interfejs korzysta z aktualnego stanu AHP.

Realny harness, odbiór operatora, Safari/Firefox, test definicji z raportem
capabilities oraz integracje MCP/A2A nadal wymagają odpowiednich prac i odbiorów.
Konkretne wyniki wykonanych weryfikacji są zapisywane w [progress.md](progress.md).

## Scoped secrets — provisioning przed integracją profilu

Schema48 przechowuje dziennik publikacji scoped credentials; polityka i wartość
pozostają w jednym niezmiennym rekordzie osobnego keyringu. Konfiguracja wymaga
ManageProject i ManageHosts. BrowserJournal przypina rewizje członkostwa,
grantu projektu i hosta; rezerwacja i aktywacja sprawdzają bieżącą tożsamość
w tej samej transakcji SQL co CAS. Potwierdzenie fizycznego zapisu po odwołaniu
jest zachowywane, ale nie stanowi nowej zgody na aktywację lub użycie.

API `/v1/web/workspaces/{id}/projects/{project}/hosts/{host}/scoped-secrets`
pozwala listować statusy. POST `/{credential}` przyjmuje expected_revision,
policy i value; POST `/{credential}/disable` przyjmuje expected_revision.
Listowanie i receipts nie zawierają wartości ani referencji keyringu. Odpowiedzi
mają no-store. Niepewne zapisy zachowują referencje; limity256 na credential
oraz4096 na workspace blokują nowe zapisy, zamiast usuwać niepewne rekordy.

Publikacja wartości nie przyznaje prawa użycia. Schema2 agenta zawiera
allowlistę MCP, a użycie wymaga osobnego grantu profilu i jednorazowej zgody.
Status execution_supported opisuje skonfigurowany interfejs zdalnych sesji
human: Unix, uruchomiony service oraz execution_enabled/persistent_sessions/
mcp_broker. Nie potwierdza bieżącego dostępu ani żywotności hosta.
Testy oraz granice odbioru są w [progress.md](progress.md).

Odwołanie ma obowiązkowy, ograniczony `last_disable` receipt w wierszu
poświadczenia. Wyłączenie może więc zapisać wynik przy pełnym ogólnym audycie;
nie zależy od odzyskania miejsca na nowy event. Stare referencje i write ACK
pozostają zachowane. Taki receipt nie jest zgodą na ponowną aktywację.

## Sloty i granty scoped HTTP

Sloty `dunlin-secret-*` używają istniejącej allowlisty MCP profilu schema2:
`provider_id`, nazwa `http` i SHA pełnej, stałej definicji narzędzia. Prefiks
jest zarezerwowany; nie może identyfikować zewnętrznego dostawcy OAuth.
Katalog `scoped-secret-tool` pod zakresem projektu/hosta zwraca schemat oraz
digest. Wywołanie podaje method/url albo operation/params. Pole secret nie
należy do schematu; wybór poświadczenia jest prywatnym powiązaniem hosta.

Schema49 zapisuje osobny grant dla principal kind/id, konkretnego
AgentProfileRef i slotu oraz konkretnej wersji poświadczenia. Manager może
listować/przyznawać grant przez `scoped-secrets/{credential}/grants`, a odwoływać
przez `scoped-secrets/{credential}/grants/{grant}/revoke`. Read-only jest jawne.
Zmiany mają CAS i zachowują tombstone; pierwotny wybór nie przejmuje nowszej
rewizji grantu. Wygaśnięcie/revocation lub ABA źródłowych uprawnień wymaga
ponownego, jawnego przypisania sekretu. Granty human wymagają ViewProject i
CreateSession; service — ViewProject i RunWorkflow oraz żywego principal.

Użytkownik z prawem tworzenia sesji może odczytać własne bieżące powiązania przez
`secret-bindings?agent_id=…&version=…&digest=…`, bez prawa ManageHosts. Zarządzanie
grantami pozostaje uprawnieniem ManageProject+ManageHosts. Katalog nie ujawnia
wartości ani keyring reference. Ten katalog opisuje konfigurację i sam nie
upoważnia do wykonania. Marker execution_supported opisuje dostępność interfejsu
zdalnych sesji human, a nie uprawnienie do dispatchu. Status i granice są w [progress.md](progress.md).

W przyroście runtime z 2026-11-14 prywatny wybór slotu scoped HTTP używa
istniejącego `mcp_connections`: `provider_id` wskazuje slot `dunlin-secret-*`,
`connection_id` jest ID grantu z `secret-bindings`, a `revision` — jego rewizją.
To identyfikator konfiguracji, bez wartości i referencji keyringu. Admission
porównuje go z rzeczywistym projektem, hostem i pełnym AgentProfileRef.
Broker wystawia statyczny schemat i rejestruje osobną, jednorazową zgodę SDK
na całe wywołanie. Przed keyringiem i po I/O ponownie sprawdza turę, lease,
źródłowe uprawnienia, grant, wersję poświadczenia i urządzenie zatwierdzające.
HTTP wykonuje kontroler; harness/runner dostaje zredagowany wynik. Dziennik
zgody zapisuje status zakończenia bez body odpowiedzi.

Odbiór tej ścieżki jest opisany w progress.md. Osobne4 scenariusze przechodzą
przez produkcyjny bridge/runner, REST/AHP, proxy stdio i sandbox bubblewrap:
poprawne wykonanie, odmowę, odwołanie grantu podczas HTTP i utratę odpowiedzi.
Endpoint wykonuje rzeczywiste TLS HTTP; testowy dialer przypina fixture CA i
loopback, a magazyn sekretów jest pamięciowy. Nie jest to odbiór rzeczywistego
keyringu ani publicznego DNS/IP, tailnetu i kont. Lokalny cel `local`
nie jest utożsamiany z zatwierdzonym hostem zdalnym; wykonanie workflow przez
service principal pozostaje odrębną pracą.

Zakładka Poświadczenia pozwala zapisać/zastąpić wartość i politykę, wyłączyć
credential i przypisać/odwołać grant human dla opublikowanej wersji profilu.
Wartość znika z inputu przed oczekiwaniem na odpowiedź; po błędzie wymagany jest
jawny odczyt statusu, bez retry zapisu. Builder pobiera pin HTTP z katalogu.
Przy tworzeniu sesji użytkownik wybiera grant dla konkretnego hosta/profilu,
a osobno konta OAuth pozostałych dostawców. Wybór nie przechodzi między celami.
Nie ma zapisów credential value do localStorage/sessionStorage.

Schema50 pozwala wskazać cel `local` przez istniejące powiązanie projektu
z lokalnym checkoutem. Nie tworzy wpisu workera. Grant przypina rewizję tego
powiązania i wymaga aktualnego native grant; rebind wymaga jawnego ponownego
nadania dostępu. Kontrolowany odbiór lokalnego procesu, proxy stdio i Chromium
ma3 scenariusze PASS: wykonanie po zgodzie, native revoke po keyring read oraz
binding revoke podczas HTTP. UI odwołuje i ponownie nadaje grant, a jego nowa
rewizja trafia do rzeczywistej sesji. Keyring i HttpExecutor są zastępnikami
testowymi; realne usługi pozostają osobnym odbiorem. Marker execution_supported wymaga
Unix, experimental_interactive_runtime i rozpoznanego skonfigurowanego harnessu;
nie zastępuje kontroli filesystemu, sesji, tury, lease ani jednorazowej zgody.

## Retencja lokalnych kopert — migracja51

Administrator z ManageProject i ManageHosts może odczytać liczniki przez
`GET .../scoped-secrets/{credential}/retention` i jawnie zlecić partię przez
`POST .../scoped-secrets/{credential}/cleanup` z expected_revision.
Odpowiedzi zawierają wyłącznie liczniki, bez wartości, polityki i referencji.
UI pokazuje pojemność, niepotwierdzone zapisy, rozpoczęte sprzątanie i historię.
Usunięcie lokalnej koperty nie potwierdza odwołania klucza u dostawcy.

Partia obejmuje do16 referencji. Warunkiem jest write_ack dokładnego,
jednorazowego writera: synchroniczny set zakończył się przed ACK i nie jest
ponawiany. Sam activated, wiek lub restart nie wystarczają. Aktywna i staged
referencja pozostają chronione; disabled może atomowo odłączyć swoje wskaźniki.
Niepotwierdzone write pozostają w limicie, także po wyłączeniu credential.

Trwały zamiar cleanup przypina pełny scope, wersję, SHA i nonce. Przed każdym
delete ponownie sprawdzana jest tożsamość i bieżące uprawnienia zarządzającego.
Faktyczny delete ACK jest zachowany także po logout; wynik dla przeglądarki
wymaga świeżej kontroli. Utrata delete lub ACK nie zwalnia pojemności. Kolejne
jawne sprzątanie może ponowić delete tego samego wycofanego, niezmiennego slotu.
Nie powtarza zapisu sekretu ani jego publikacji.

Kursor partii jest trwały i cykliczny. Limit256 na credential i4096 na workspace
obejmuje nieusunięte referencje; po delete ACK limit jest odzyskiwany. Osobna
historia zawiera maksymalnie512 bezpiecznie usuniętych wpisów na workspace.
Przycinanie dotyczy tylko potwierdzonych delete po zakończeniu writera. REST
nigdy nie przyjmuje referencji do zapisu; SDK zawsze losuje nowy slot. Stare
claims po usunięciu metadanych nie mogą aktywować referencji lub wznowić zapisu.
Status odbioru i jego granice są w [progress.md](progress.md).

## Przerywanie transportu po utracie authority

Chroniony HTTP vaulta używa osobnego poola klienta dla każdego wywołania.
Watchdog sprawdza live authority co25ms i zamyka sockety po odmowie lub końcu
budżetu, także podczas TLS, oczekiwania na nagłówki i odczytu body. Nie zamyka
połączeń innych sesji. Czas żądania jest ograniczony pozostałym budżetem operacji.
EOF po shutdown nie jest dowodem pełnej odpowiedzi: końcowy fence odrzuca
pozorny sukces. Nie ma automatycznego ponawiania operacji.

Własny HttpExecutor musi jawnie zaimplementować execute_guarded i faktyczne
anulowanie blokującego I/O. Domyślna implementacja odmawia chronionego wykonania.
Legacy guarded vault stosuje ten sam kontrakt także do refresh przed HTTP.
Scoped runtime nadal wymaga pełnego grantu i jednorazowego permitu SDK.

Guarded MCP modern/legacy utrzymuje watchdog przez życie klienta i odczyt
odpowiedzi. Zamknięcie właściciela modern zamyka również sockety niezależnych
subskrypcji SSE. Odmowa DispatchGuard pozostaje terminalna po regrant. Jego
callback może być wywoływany współbieżnie; powinien sprawdzać bieżące authority
bez trzymania blokady przez HTTP i bez wykonywania własnego sieciowego I/O.

Pozytywna wymiana kodu OAuth i refresh mają osobny `with_token_guard`.
Guard obejmuje wysłanie formularza, nagłówki, body i walidację tokenów na
izolowanym poolu. Callback sprawdza oryginalną sesję przeglądarki i trwały
stan authorizing; katalog sprawdza członkostwo i rewizję połączenia, a broker
sesji także deadline, uprawnienia sesji i dodatkowy warunek danej operacji.
Odmowa jest terminalna. Niepewny refresh pozostaje zablokowany w journalu;
regrant lub ponowne inspect nie ponawia rotującego requestu. Token guard nie
obejmuje jawnej negatywnej revocation: ta ma własne uprawnienia i potwierdzenia.
Discovery ma osobny `Discovery::with_guard`: probe MCP, metadane zasobu i AS
używają izolowanego poola z watch przez odczyt i walidację odpowiedzi. Odmowa
blokuje też kolejne adresy fallback oraz utworzenie flow z metadanych. Limity
10s/request i 30s/discovery nadal obowiązują. Pozytywne ścieżki serwera tworzą
discovery przez `flow_authorized`; katalog/runtime jawnie zachowują ten guard
również dla wymiany tokenów. Start autoryzacji nie zachowuje discovery guard
w Pending: callback dostaje świeżą kontrolę dla przygotowanej rewizji. Dzięki
temu Pending nie utrzymuje przez guard własnego serwisu. Wyniki odbioru są
zapisywane w progress.md.

Trwający systemowy DNS lub TCP connect nie jest przerywany przez shutdown.
Obowiązują timeout i budżet; socket zakończony po shutdown nie może wysłać
żądania. Zamknięcie transportu nie cofa wcześniejszego efektu u dostawcy.
Jawne negatywne przepływy AS mają własne uprawnienia i factual ACK; nie są
przerywane guardem dodatniego użycia. Recipes/service workflow i pełny odbiór
kill switcha pozostają kryteriami E5.3/E10; status testów jest w progress.md.


## Receptury scoped — rdzeń i kontrakt hosta

Biblioteka vault udostępnia `permit_recipe`/`recipe`, osobny `RecipePermit` oraz
statyczną definicję MCP `recipe`. Argumentami są nazwa i zadeklarowane parametry;
credential, program, env i katalog roboczy wybiera host. Lokalny runtime Linux z włączonym sandboxem obsługuje również pin `recipe`.
Katalog/API udostępnia go przez `scoped-secret-tool?kind=recipe`. Builder pobiera
przypiętą definicję HTTP lub recipe dla wskazanego celu i blokuje dodanie
narzędzia przy execution_supported=false. Native macOS, adapter zdalny
i workflow wymagają dalszego podłączenia.

`RecipeTarget` ma oddzielne warianty local i remote. Local opisuje binding
workspace/project/host, canonical cwd i digest checkout stamp; remote nie ma
lokalnej ścieżki. Konstruktor sprawdza format; dowód pochodzi z wymaganego
`RecipeAuthority::check_target`, wykonywanego przy admission, przed keyringiem,
po I/O i przez live guard. HTTP authority samodzielnie nie wystarcza.
Lokalny digest zachowuje dotychczasowe bajty/domenę `dunlin.scoped-recipe.v1`.
Remote ma osobną domenę `dunlin.scoped-recipe.remote.v1` i wiąże pełny input,
binding, channel/turn stamps, export ID/revision i checkout stamp. RecipePermit nie ma Deserialize ani Clone i nie jest
zamienny z permit HTTP. Zmiana inputu, checkoutu, hosta, wersji credential lub
read-only odcina wykonanie. Zużytego request ID nie wolno przywrócić po regrant.

Wykonanie wybiera dokładnie jedną recepturę z przypiętej koperty. Wymaga pinu
SHA-256 programu, odrzuca brak pinu i niejednoznaczną nazwę. Parametry przechodzą
walidację istniejącego template; agent nie wybiera shell ani dodatkowych argv.
Timeout jest ograniczony pozostałym budżetem permitu i polityką receptury.
Niepewne wykonanie lub utrata completion ACK zwracają Unknown, bez retry.
Redakcja obejmuje stdout/stderr i wspólne kodowania wartości. Oba strumienie mają
limit także po redakcji; zbyt duża odpowiedź adaptera jest odrzucana. Debug planu
maskuje sekret i wartości argv; bufory przejściowe wyjścia są zeroizowane.

CommandExecutor kończy własną grupę również po naturalnym wyjściu rodzica,
żeby potomkowie nie trzymali odziedziczonych pipe. WNOWAIT zachowuje unreaped
leader do zakończenia sygnalizacji i watchdoga, ograniczając ryzyko ponownego
użycia PID/PGID. RAII sprząta posiadany proces przy błędzie/unwind. Panic live
callbacku w watchdogu terminalnie odwołuje dostęp i zabija grupę. Ten prymityw
nie stanowi sandboxa nieufnego programu; adapter targetu musi zapewnić wymaganą
izolację, własność drzewa procesu oraz odseparowany HOME/config/credential store.
Adapter utrzymuje zweryfikowaną tożsamość i niezmienność programu przez spawn.
Test pinu CommandExecutor nie zastępuje dowodu tej własności w adapterze hosta.
Dziedziczenie HOME w legacy CommandExecutor nie jest dowodem takiej izolacji.
Guarded wykonanie na platformie bez mechanizmu grupy procesu jest odmawiane. Odbiór native platform pozostaje w E10.

Docelowe podłączenie runtime wymaga następujących warstw:

1. Adapter local buduje target z bieżącego, zaufanego checkoutu i potwierdza
   go razem z grantem, profilem, turą i lease. Serwer wykonuje CLI dla celu local.
2. Adapter remote wykonuje CLI na przypiętym runnerze. Wymaga jawnej negocjacji
   capabilities/protokołu i polityki operatora dla nazwanych, przypiętych
   programów; istniejące `mcp_broker` nie upoważnia do dostarczenia sekretu CLI.
   Prywatna komenda wiąże nonce operacji z sesją/turą, fence kanału, hostem,
   exportem/revision i checkout stamp. Runner weryfikuje swój grant i template,
   korzysta z własnego cwd i izoluje proces od harnessu. Wartość trafia wyłącznie
   do prywatnego wykonawcy; odpowiedź i ledger nie zawierają sekretu. Powtórzenie
   receipt nie uruchamia programu. Brak capability pozostaje unsupported.
3. Broker i Builder publikują pin receptury dopiero po podłączeniu adaptera;
   jednorazowa zgoda SDK obejmuje pełną operację oraz przypięty cel sesji.
4. Workflow używa żywego service principal, RunWorkflow, dokładnej opublikowanej
   wersji workflow/step i grantu credential/target. Durable admission kroku
   stanowi jego własny dowód użycia; nie podszywa się pod urządzenie człowieka.
   Recovery zachowuje Unknown i nie ponawia potencjalnego efektu CLI.

Te integracje pozostają wymaganiami E5.3/E7. Biblioteczny odbiór nie zastępuje
odbioru runtime, protokołu runnera, UI ani rzeczywistego keyringu i kont.


## Lokalny adapter receptur Linux

Local authority konstruuje RecipeTarget z rzeczywistego NativeCheckoutStamp
(binding, granty, canonical root i tożsamość checkoutu). Wymaga Write oraz
włączonej izolacji; wszystkie dotychczasowe warunki sesji/tury/lease/urządzenia
zatwierdzającego pozostają aktywne. Otwarty deskryptor katalogu jest porównany
z zapisanymi dev/inode. Kernelowy bind deskryptora wybiera dokładnie ten obiekt
nawet przy późniejszej podmianie ścieżki. Zmieniony checkout odcina kolejne guardy.

LinuxRecipeExecutor przechwytuje maksymalnie128MiB obrazu programu, sprawdza
SHA-256 i blokuje zapis/zmianę długości memfd przez kernel seals. Bubblewrap
kopiuje ten obraz z deskryptora do własnej, read-only ścieżki wykonywalnej.
Zmiana pliku źródłowego po przechwyceniu nie zmienia obrazu. Deskryptory mają
CLOEXEC w procesie hosta; tylko konkretny fork udostępnia je launcherowi.

Launcher `/usr/bin/bwrap` i jego systemowe katalogi muszą należeć do root oraz
być niezapisywalne dla group/other. Namespace PID i `die-with-parent` wiążą drzewo
procesu z wykonaniem, w tym potomków używających setsid. Checkout jest widoczny
pod `/workspace`; HOME/config/cache/tmp są prywatne. Poza obrazem i checkoutem
montowane są read-only systemowe runtime i wybrane publiczne pliki DNS/CA.
Sekret trafia do środowiska prywatnego procesu, poza argv i środowiskiem harnessu.
Zmienne sterujące loaderem, runtime i ambient credential/config są odmawiane.
Wynik przechodzi redakcję i limity rdzenia przed wysłaniem do MCP.

Grant może przypinać HTTP, recipe albo oba narzędzia z ich dokładnymi digestami;
recipe wymaga read_only=false. Katalog recipe wymaga skonfigurowanego lokalnego
adaptera, a dispatch ponownie odczytuje target i wykonawcę po zgodzie SDK.
Błąd przygotowania po zużyciu zgody kończy request jako Unknown i przerywa
runtime. Zwykły exit/error CLI ma jawne exit_code/signal/timed_out i isError.
Powtórzone historyczne potwierdzenie SDK nie wykonuje programu ponownie.

Bieżący odbiór jest Linux-only z produkcyjnym proxy i kontrolowanym keyringiem.
To nie jest odbiór rzeczywistych kont, native macOS, restartu hosta ani polityki
zasobów na docelowej maszynie. Programy korzystają z udostępnionego systemowego
runtime; dodatkowe assets i linked Git metadata wymagają jawnego rozwiązania
przez adapter. Remote authority nadal odmawia receptur do czasu negocjacji
prywatnej capabilities/protokołu i polityki operatora; nie wykonuje ich na
kontrolerze dla zdalnego celu. Service workflow pozostaje otwarty.


## Konfiguracja receptury w interfejsie

W Builderze wybierz „Dodaj narzędzie z poświadczeniem”, lokalny cel, rodzaj
„Receptura CLI” i nazwę dostępu. Definicja narzędzia i digest pochodzą z katalogu
serwera dla aktualnego zakresu; odpowiedź poprzedniego zakresu nie może dodać
pinu. Brak obsługi na celu blokuje dodanie narzędzia. Opublikuj profil osobną
akcją, następnie zapisz poświadczenie i jego pełną politykę w zaawansowanym
formularzu JSON. Polityka receptury wskazuje program, SHA-256, nazwę zmiennej,
argumenty, parametry i timeout; model nie wybiera tych ustawień przy wywołaniu.

W poświadczeniach wybierz opublikowany profil i użytkownika. HTTP i recipe
współdzielą jeden grant providera. Każdy taki dostęp obejmujący recipe wymaga
prawa zapisu; opcja „Tylko odczyt HTTP” jest wyłączona. Nie znosi to osobnej,
jednorazowej zgody właściciela sesji na pełne wywołanie ani kontroli backendu.
W sesji wybierz ten grant i zatwierdź rzeczywiste argumenty receptury w AHP.

Kontrolowany odbiór Chromium przechodzi przez utworzenie i publikację nowego
profilu, zapis polityki, grant, utworzenie sesji, zgodę i wykonanie CLI. Backend
sprawdza liczbę efektów, odczytów keyringu, brak HTTP fallback i brak sekretu
w transkrypcie. Oddzielne scenariusze sprawdzają deny, zmieniony program,
revoke działającego potomka i HTTP. Keyring jest pamięciowy; realne konta,
native macOS, prywatny remote i odbiór E10 pozostają poza tym sprawdzeniem.


## Wspólny pin i polityka runnera — przygotowanie prywatnych receptur

RecipeTemplate opisuje wyłącznie konfigurację bez wartości poświadczenia.
Digest ma domenę `dunlin.worker.recipe-template.v1`; obejmuje obraz programu,
argv template, parametry, zmienną i timeout. Różne lokalne ścieżki tego samego
przypiętego programu mogą mieć ten sam pin. ID credential, jego wersję i pełny
cel musi osobno wiązać jednorazowa operacja; zgodność template nie jest authority.

Operator może zapisać politykę schema3 z jawnymi recipes. Domyślna schema1 ani
MCP-only schema2 nie upoważniają do tego rozszerzenia. Nowa lista jest przypięta
raw settings digest do eksportu, więc rebind odcina wcześniejsze granty i lease.
[Format i granice polityki](worker-exports.md#nazwane-receptury-operatora--schema-3).
Nie dodano jeszcze prywatnej komendy, negocjacji transportu, przekazania wartości,
ledgeru jednorazowej operacji ani hostowego adaptera remote. Authority nadal
odmawia recipe dla zdalnego celu. Kolejny krok to negocjowany prywatny kanał
z dowodem faktycznego runnera/checkoutu, zamiast wykonania na kontrolerze.

Pozostałe podłączenie prywatnego wykonania ma zachować następujące granice:

- Negocjacja musi być osobna od MCP-only v5 i od samego zapisu polityki3.
  Serwer i runner wymagają własnego opt-in oraz faktycznej implementacji,
  z ofertą ograniczeń i bez fallbacku do local ani starszego protokołu.
- RecipeTarget rozdziela już local cwd i RemoteRecipeTarget. Kontroler nie zna
  canonical root runnera i nie może zastąpić go własną ścieżką ani fikcyjnym
  katalogiem. Hostowy adapter musi jeszcze budować channel/turn/checkout stamps
  z rzeczywistego worker boot, channel fence, sesji/tury i export/revision;
  runner wybiera swój captured FD i sandbox-visible cwd z zaufanego adaptera.
- Wartość trafia dopiero po jednorazowym admission/zgodzie do prywatnego
  wykonawcy daemonu runnera. Nie trafia do ACP, konfiguracji MCP harnessu,
  publicznego DTO, pliku projektu ani trwałego ledgeru. Dane operacji wymagają
  maskowanego Debug, ograniczeń i sprzątania buforów. Obcy lub nieobsługiwany
  kanał nie może wyzwolić odczytu keyringu ani otrzymać wartości.
- Runner niezależnie porównuje semantyczny template z własną polityką oraz
  bieżące granty i checkout. Przed spawn przechwytuje obraz z weryfikacją SHA,
  używa sandboxa i live guarda, a utrata lease/kanału/grantu kończy jego drzewo.
  Wynik należy zredagować przed odpowiedzią i utrwalić wyłącznie bezpieczny
  fakt zakończenia. Sam ograniczony stdout nie dowodzi braku credential.
- Trwały nonce/fingerprint należy zużyć przed efektem. Powtórzony packet lub
  receipt nie uruchamia programu; utracony ACK pozostaje Unknown. Odczyt stanu
  operacji po reconnect nie może wymagać ponownego przesłania sekretu ani
  implicit retry. Recovery nie odtwarza żywego pozwolenia z wire/SQL.

To wymagania następnego przyrostu, a nie opis już działającego transportu.


## Oddzielny plan zdalnego wykonania — rdzeń biblioteczny

RecipeTemplate i reguły parametrów znajdują się w domain, a moduł protokołu
pozostawia ich wspólny eksport. Digest/domena/JSON pinów operatora nie zmieniają
się po tym przeniesieniu; stały wektor SHA-256 sprawdza format. Serde JSON oraz
istniejące w workspace sha2/hex są zależnościami normalnymi domain, ponieważ
pin jest semantyką współdzieloną przez vault i operatora, niezależną od transportu.
Nie wprowadzono nowej wersji zewnętrznego pakietu ani zależności vault od protokołu.

RemoteRecipeTarget nie ma Deserialize i nie zawiera cwd. Jego konstruktor
sprawdza tylko kształt; nie wytwarza authority ze stampów ani metadanych SQL/wire.
RecipeAuthority nadal musi potwierdzać cały target w każdym świeżym sprawdzeniu.
Plan lokalny obsługuje wyłącznie RecipeExecutor, a zdalny — osobny interfejs
RemoteRecipeExecutor. Niewłaściwy rodzaj jest odrzucany przed keyring read,
bez fallbacku. Jednorazowość, odwołanie dostępu oraz Unknown po możliwym efekcie
lub utracie completion pozostają wspólne.

`Credentials::recipe_remote` buduje RemoteRecipePlan wyłącznie z przypiętej
koperty i targetu. Plan zawiera template, zatwierdzone params, timeout oraz
wartość w buforze Zeroizing; nie zawiera ścieżki programu ani cwd kontrolera.
Debug pomija wartość i treść parametrów. Zaufany adapter otrzymuje LiveAccess
bez opcjonalnego/unguarded wariantu. Musi zweryfikować rzeczywisty cel, wykonać
program na runnerze i skorzystać z redakcji obu strumieni przed prywatną
odpowiedzią. Udostępniona redakcja uwzględnia także Basic username:value,
ogranicza rozmiar i zeroizuje przejściowe bufory. Kontroler redaguje ponownie
przed publikacją rezultatu do MCP.

Odbiór biblioteczny z kontrolowanym adaptorem nie jest odbiorem transportu ani
prawdziwego runnera. Serwerowy Remote authority nadal odmawia recipe. Brakuje
negocjacji, prywatnego payloadu/RPC, wiązania rzeczywistych fence i ledgeru
zdalnej operacji oraz integracji wykonawcy daemonu.

Podłączenie katalogu wymaga osobnego sprawdzenia gotowości na poziomie sesji:
lista tools/list może powstać przed pierwszą turą. Nie wolno tworzyć fikcyjnego
turn stamp w celu pokazania recipe. Zdalny target z rzeczywistym turn stamp
powstaje dopiero dla tools/call i musi odpowiadać zweryfikowanemu BrokerScope
tej operacji. Sama obecność pinu w katalogu ani historical session receipt
nie upoważniają do przesłania sekretu. Authority ma osobny recipe_catalog;
local zachowuje dotychczasowe sprawdzenie executora, a domyślna implementacja
odmawia. Hostowy remote preflight pozostaje do podłączenia po negocjacji WSS.


## Prywatny kontrakt v6 — kodeki metadanych, przed podłączeniem WSS

`worker::recipe::private` ma osobne Hello6, OpenRecipeSessionCommand oraz
PrepareRecipe. Dotychczasowe kodeki v1–v5 odrzucają te koperty. Nie dodano flag
aktywacji ani obsługi v6 do obecnego endpointu lub klienta, więc sam kodek nie
negocjuje kanału i nie pozwala przesłać wartości. Do rzeczywistego włączenia
potrzebny jest działający ledger i daemonowy executor oraz opt-in obu stron.

RecipeLimits wymagają jawnego limitu pending1–4, secret bytes1–16384,
serialized reply bytes1024–262144 oraz timeout1–120000ms. Wybrany limit nie może
przekroczyć oferty żadnej strony ani właściwego budżetu brokera. To górne
limity; nie są obietnicą przedłużenia aktualnej tury/lease ani wolnego slotu.
Open fingerprint ma domenę v6 i przypina całe open oraz oba zestawy limitów;
odczyt jego historycznego receipt nie tworzy żywego pozwolenia.

RecipeOperation zawiera tylko korelację i wejście operacji: nonce, TurnFence,
ExportDescriptor, dokładny AgentMcpTool, nazwę/digest template, credential stamp,
zatwierdzone params, timeout/reply budget i fingerprint. Nie zawiera wartości,
ścieżki programu ani cwd. Debug pomija params i credential stamp. Metadane
mają limit64KiB, params8KiB. Wartość wymaga osobnego ograniczonego efemerycznego
payloadu; dodanie pól value/secret do PrepareRecipe jest odrzucane.

Fingerprint operacji ma osobną domenę i wiąże wszystkie wymienione pola.
validate_for_turn porównuje faktyczny turn/descriptor, sprawdza dokładną
allowlistę przypiętej definicji profilu i wybrane limity. To weryfikacja danych,
nie authority. Runtime musi jeszcze potwierdzić channel guard, oryginalne
monotonic leases, własny grant i checkout oraz dopasować operator template.
Dopiero trwałe zużycie nonce/fingerprint i własny opaque handle mogą dopuścić
payload i efekt. Ledger powinien utrwalać bezpieczne piny/status/fingerprint,
nie surowe params, wartość ani pełny blob metadanych. Powtórzenie po utracie ACK
nie może odtworzyć pozwolenia. Redakcja poprzedza ograniczanie odpowiedzi;
niepewne/truncated bufory i prywatny result codec wymagają osobnego odbioru.
