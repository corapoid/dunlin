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

Trwający systemowy DNS lub TCP connect nie jest przerywany przez shutdown.
Obowiązują timeout i budżet; socket zakończony po shutdown nie może wysłać
żądania. Zamknięcie transportu nie cofa wcześniejszego efektu u dostawcy.
Pozostałe przepływy AS, recipes/service workflow i pełny odbiór kill switcha
pozostają odrębnymi kryteriami E5.3/E10; status testów jest w progress.md.
