# Klient MCP i granice wykonania

E5.2 obejmuje transport2026, adapter2025, discovery/metadata, MRTR,
subscriptions, OAuth i ograniczoną walidację schematów. Zakres tego punktu
został odebrany według macierzy wymagań na końcu dokumentu. E5.3–E5.5,
pozostałe punkty aplikacji i zewnętrzne odbiory E10 pozostają otwarte.

## Transport i wybór wersji

`dunlin_mcp_client::Client` jest dotychczasowym adapterem2025 z initialize,
negocjowaną wersją i opcjonalnym Mcp-Session-Id/DELETE. Nowe
`negotiated::Client` najpierw wysyła read-only server/discover jako2026-07-28.
Broker katalogu używa nowego interfejsu; dotychczasowy adapter Linear pozostaje2025.

Modern transport używa osobnych POST JSON/SSE, per-request metadata i nagłówków
MCP-Protocol-Version/Mcp-Method/Mcp-Name. Nie wykonuje initialize ani DELETE
i nie zapamiętuje session ID, nawet jeśli serwer taki nagłówek zwróci.
Samo odkrycie serwera nie nadaje żadnego grantu projektowego lub narzędziowego.

Nierozpoznane HTTP400 z probe wybiera adapter2025. Rozpoznane błędy modern
(-32022/-32021/-32020), błędy uwierzytelnienia, inne statusy i utrata transportu
nie powodują takiego downgrade. Nie ma automatycznego retry wywołań narzędzi.
Oba etapy dzielą90s budżetu; pojedynczy HTTP request ma najwyżej20s i2MiB body.
Endpoint musi być HTTPS albo loopback HTTP, bez credentials/query/fragment;
env proxy i redirects są wyłączone.

## Katalog i nagłówki

Modern tools/list publikuje cały katalog atomowo, maksymalnie512 narzędzi,
16 stron i4KiB cursor. Duplikaty nazw i zapętlone cursory są błędem.
Nieudany refresh usuwa wcześniejszy katalog. Narzędzie z błędną adnotacją
x-mcp-header jest wykluczane; lokalny budżet pracy nad schematem pozostaje
osobnym limitem. Wskaźniki schematu nie uruchamiają zewnętrznych odczytów.

Projekcja nagłówków korzysta z dokładnych ścieżek properties. Sprawdza tchar,
unikalność nazw bez względu na wielkość liter i primitive types. Wartości
integer muszą mieścić się w safe integer range. Unicode, control characters,
padding i sentinel-shaped literals są kodowane Base64. Brak/null pomija header.
Nagłówki narzędzia mają prefiks Mcp-Param-, nie zastępują Authorization.

Wywołanie musi odpowiadać pobranemu name/inputSchema/outputSchema. Obie wersje
transportu wymagają pobranego katalogu i walidują argumenty przed HTTP. Sam pin
i poprawność danych nie są zgodą na narzędzie; allowlisty należą do E5.3.

## Walidacja schematów narzędzi

Klient kompiluje schematy przez jsonschema0.46.0, bez domyślnych funkcji odczytu
plików i HTTP, dodatkowo z retrieverem zawsze odmawiającym zewnętrznych zasobów.
Domyślny draft to2020-12. Jawne standardowe URI draft4/6/7,2019-09 i2020-12
są obsługiwane; niestandardowe metaschematy są odrzucane.

Profil lokalny dopuszcza referencje fragmentowe: JSON Pointer i kotwice wewnątrz
jednego dokumentu. Zagnieżdżone resource IDs i referencje do innych dokumentów
nie są obsługiwane. Analiza grafu odrzuca cykle bez postępu w instancji oraz
nadmierne rozwinięcia współdzielonych definicji. Rekursja przez właściwości lub
elementy instancji pozostaje możliwa pod limitami. Nie interpretuje referencji
w enum/default/examples jako dyrektyw schematu.

Znane formaty są sprawdzane. Regex używa liniowego silnika z ograniczeniami
kompilacji/cache; wzorce wymagające lookaround/backreferences są odrzucane.
Nie ma fallbacku pomijającego nieobsługiwane ograniczenie. Schemat niespełniający
profilu kompilacji wyklucza narzędzie; błędna ramka katalogu lub przekroczenie
budżetu odrzuca odczyt katalogu i blokuje użycie poprzedniego pinu.

Walidacja nie konwertuje typów i nie dopisuje default. Wariant Arguments nie
zawiera treści instancji. Po udanym tools/call, także po MRTR, outputSchema
wymaga pasującego obiektu structuredContent. Błędny wynik daje Protocol bez retry;
isError nadal daje ToolFailed. Zmiana samego outputSchema również unieważnia pin.

| Zasób | Limit |
| --- | --- |
| Jeden schemat |64KiB JSON,1024 węzły, głębokość64 |
| Analiza referencji | głębokość64,65536 rozwinięć dla korzenia,1M kroków analizy |
| Instancja argumentu/wyniku |256KiB,8192 węzły, głębokość64 |
| Praca walidacji | koszt schematu × (węzły instancji + szacowane bajty wartości/64 zaokrąglone w górę) ≤1M |
| Katalog |65536 jednostek kosztu input/output,128 wzorców |
| Regex |16 wzorców/schemat,32KiB kompilacji i32KiB DFA/wzorzec |

Po przetworzeniu katalogu, odpowiedzi MRTR lub wyniku narzędzia interfejs ponawia
DispatchGuard przed zwróceniem danych integratorowi. Obejmuje to czas kompilacji
i walidacji schematów po ostatnim sprawdzeniu HTTP. Walidator nie utrzymuje Store lock.

Budżet walidacji jest strukturalny; nie jest osobnym przerywalnym procesem CPU.
Dokładne cases i wyniki są w progress.md. Podstawy:
[schematy narzędzi MCP](https://modelcontextprotocol.io/specification/2026-07-28/server/tools),
[biblioteka i konfiguracja](https://docs.rs/jsonschema/0.46.0/jsonschema/struct.ValidationOptions.html).

## Wieloetapowe operacje MRTR

Nowy interfejs rozróżnia Outcome::Complete i Outcome::InputRequired(Pending).
Obsługuje tools/call przez begin_call, prompts/get przez get_prompt oraz
resources/read przez read_resource. Kontynuacja jest osobnym, jawnym wywołaniem
continue_request z decyzjami dla dokładnie tych kluczy, o które poprosił serwer.
Wygodne call zachowuje dotychczasowy Result<Value> i zwraca Unsupported przy
potrzebie kontynuacji; nie kontynuuje w tle.

Pending nie ma Clone/Serialize/Debug. Jest związany z konkretną instancją
uwierzytelnionego klienta i oryginalną operacją. Przeniesienie do innego klienta,
nawet tego samego endpointu, kończy się odmową. Każde użycie zużywa uchwyt,
także przy błędzie walidacji lub transportu. Po błędzie sieci wynik może być
niepewny; nie wolno traktować błędu jako instrukcji ponowienia mutacji.

requestState jest opcjonalnym opaque string: klient sprawdza jedynie typ
i rozmiar, zachowuje dokładną treść i nie interpretuje jej. Każda kontynuacja
ma nowe JSON-RPC ID, oryginalną metodę/parametry i wyłącznie stan/odpowiedzi
bieżącej rundy. Poprzedni stan nie trafia do kolejnych niezależnych operacji.

Budżety:8 równocześnie zajętych miejsc,8 rund,60s na operację,16 input requests,
16KiB requestState. Rezerwacja miejsca następuje przed wysłaniem operacji;
kontynuacja zachowuje swoje miejsce, a zakończenie/drop/błąd je zwalnia.
Deadline obejmuje także kolejne HTTP requests i pozostaje ograniczony90s
całego klienta. Kontynuacja narzędzia ponownie sprawdza pin katalogu.

Domyślne clientCapabilities są puste. Opcjonalne InputSupport::elicitation_url
deklaruje wyłącznie elicitation.url. Integracja musi zapewnić przegląd żądania,
wskazanie serwera i domeny docelowej oraz jawne accept/decline/cancel.
Transport sprawdza HTTPS i brak credentials w URL; nie otwiera linku ani nie
wysyła dodatkowego HTTP requestu. Odpowiedź zawiera tylko action, bez sekretów.
Bearer MCP nie jest zmieniany przez zewnętrzną interakcję.

Opcjonalne InputSupport::elicitation_form deklaruje wyłącznie obsługę form;
brak mode oznacza form. Pending udostępnia osobno form_requests i dotychczasowe
URL input_requests. continue_with_responses wymaga dokładnego zestawu kluczy
i właściwego typu ElicitationResponse dla każdego z nich. Accept formularza
wymaga zwalidowanego content; decline/cancel nie przyjmują content. Nieprawidłowa
odpowiedź zużywa Pending bez HTTP, a odebrana authority blokuje kontynuację.
Dotychczasowe continue_request pozostaje skrótem dla decyzji URL.

FormSchema obsługuje płaski profil MCP: string/number/integer/boolean, single-select
enum/oneOf oraz multi-select enum/anyOf. Sprawdza min/max, wymagane pola i formaty
email/uri/date/date-time; odrzuca niezamówione pola i powtórzone wybory. Zagnieżdżone
obiekty i arbitralne schematy formularzy są odrzucane. Limity to16KiB/1024 węzły
schematu,32 pola,64 wybory/pole; odpowiedź do64KiB/1024 węzłów,8KiB pojedynczego
tekstu i2KiB wartości wyboru. Integer mieści się w safe integer range. Default
jest wyłącznie metadanymi do przeglądu; transport go nie zatwierdza ani nie dopisuje.

Domyślnie oba tryby elicitation w bibliotece i niezależnej inspekcji katalogu
są wyłączone. Wspólny broker wykonania jawnie włącza form i URL dla
zatwierdzonych tools/call. URI w polu formularza jest zwykłym tekstem, bez nawigacji lub fetch.

Migracja41 przechowuje hostowe ID, przypiętą prezentację/schemat oraz decyzję,
powiązane kluczem obcym z oryginalną consumed zgodą MCP. Standardowe AHP
chat/inputRequested i chat/inputCompleted korzystają z origin ledger/dedupe.
Publiczny dispatch używa dokładnego FormSchema::validate w transakcji decyzji.
Błędna odpowiedź ma canonical rejection bez zużycia input lub zamknięcia WS;
poprawa wymaga nowego clientSeq, a retry starego zamiaru pozostaje historyczne.
Zwykły dispatcher SDK nadal odmawia accept bez jawnego walidatora.

Mapper zachowuje tekst, number/integer, boolean, single-select i multi-select,
formaty/zakresy/wymagalność oraz wskazanie provider/origin/tool. Wartości
proponowane są tylko metadanymi do przeglądu, bez automatycznego wypełniania.
UI zachowuje0/false/puste enum/required empty array i bezpieczne property names,
w tym __proto__. Przeglądarka ma jawne accept/decline/cancel i immutable retry.
Prezentacja enum może powielić value jako label: SDK display ma jawny limit
48KiB/4096 nodes, a schemat nadal16KiB/1024 nodes; transport input nadal64KiB.

Dziennik nie przechowuje requestState ani Pending i nie nadaje prawa wykonania
kontynuacji po restarcie. Parent invalidation/recovery zamyka oczekujące input.
Broker utrzymuje oryginalny uwierzytelniony Client/Pending i niezmienne arguments,
permit, tool pin i deadline przez wszystkie rundy. Kontroluje bieżące urządzenia
osób odpowiadających przed dalszym HTTP, po odpowiedzi i przed utrwaleniem wyniku,
obok pierwotnej zgody/konta/OAuth/authority. Decline/cancel nie wysyłają content.

Zdalna ścieżka ma kontrolowany odbiór runtime19 i Chromium desktop/mobile:
dwie rundy accept, korekta ułamkowego integer w API oraz URI w UI bez dodatkowej
kontynuacji, duplikaty odpowiedzi bez replay, odmowa/cancel bez efektów w checkout,
i disconnect konta podczas formularza (Unknown, canceled input,0 continuations,
stop/ACK2, reservation0). Opaque state i bearer nie trafiają do DOM ani runnera.
Wspólny rdzeń jest używany także lokalnie; ten nowy pełny odbiór dotyczy zdalnego
Linux fixture z kontrolowanym OAuth/MCP i ACP, nie prawdziwych dostawców/platform.
To historyczny odbiór form. Późniejsze URL, trwałe revoke i bounded cleanup/
retencja mają osobne odbiory opisane niżej. Ogólne scoped secrets należą do
otwartego E5.3; prawdziwi dostawcy i platformy do E10. Wyniki są w progress.md.

Kontrakt:
[MCP elicitation](https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation).

Roots i sampling pozostają unsupported i nie są reklamowane. Oficjalna wersja
2026 oznacza je jako deprecated i odradza ich adopcję w nowych implementacjach:
[roots](https://modelcontextprotocol.io/specification/2026-07-28/client/roots),
[sampling](https://modelcontextprotocol.io/specification/2026-07-28/client/sampling).

## Subskrypcje zmian

Modern listen wysyła subscriptions/listen przez POST z jawnie wybranym filtrem:
toolsListChanged, promptsListChanged, resourcesListChanged i konkretne resource
URIs. Pierwszym komunikatem musi być ACK z identyfikatorem tego requestu.
Uzgodniony filtr może być podzbiorem żądanego; nie może rozszerzać dostępu
do powiadomień. Każda kolejna wiadomość ma ten sam subscriptionId. URI update
musi dokładnie odpowiadać jednej z uzgodnionych subskrypcji.

Subscription posiada body HTTP niezależnie od klienta i nie przechowuje bearer.
Integracja aktywnie odbiera zdarzenia przez next_event; może przenieść uchwyt
do własnego reader thread i równolegle używać klienta do wywołań. Nie ma
automatycznego thread/reconnect ani ponawiania mutacji. Legacy adapter zwraca
Unsupported zamiast próbować GET lub resources/subscribe.

Odbiór tools/list_changed unieważnia pin katalogu wyłącznie swojego klienta.
Otwarcie subskrypcji unieważnia dotychczasowy pin, aby pokryć lukę przed ACK;
po otwarciu trzeba pobrać katalog. Zmiana podczas pobierania kolejnych stron
blokuje publikację tego wyniku. Drop, błędna ramka, abrupt EOF oraz graceful
completion również unieważniają objęte filtrem obserwacje. Refresh nie jest
wykonywany automatycznie. Epochy prompts/resources są przygotowane dla cache;
cache odczytów wykorzystuje te same epochy do natychmiastowego unieważniania.

Ograniczenia:4 uchwyty na klienta,64KiB na ramkę SSE,2MiB na cały stream,
1024 komunikaty oraz64 resource URIs do4KiB każdy. Limit ramki obejmuje też ACK.
Każdy HTTP stream podlega dotychczasowemu deadline do20s i pozostałemu budżetowi
90s klienta; dalsze nasłuchiwanie wymaga jawnego ponownego listen. Nie używamy
Last-Event-ID ani wznowienia strumienia. Komentarze SSE są ignorowane, ale
liczą się do budżetu bajtów.

Drop Subscription zamyka reader i zwalnia miejsce. close/drop klienta oznacza
jego odbiorniki jako anulowane; sprawdzają to przy odbiorze. Odczyt już
zablokowany w HTTP pozostaje ograniczony swoim deadline, bez obietnicy
natychmiastowego przerwania blokującego wywołania.

Kontrakt odczytano z oficjalnego
[subscriptions.mdx](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/patterns/subscriptions.mdx),
blob SHA0349e28be419632e1498a3924a8ed0ba2b85d363.

## Cache odczytów i TTL

Cache jest memory-only, prywatny dla konkretnego endpointu, bearer i zestawu
input capabilities. Nawet wyniki oznaczone public nie są współdzielone między
instancjami klienta. Klucz obejmuje metodę i dokładne parametry, w tym cursor
lub URI. Nie zapisuje tokenów ani decyzji autoryzacyjnych.

Lista dopuszczonych metod obejmuje server/discover, tools/list, prompts/list,
resources/list, resources/templates/list i resources/read. Obecne publiczne
odczyty discover/tools/read_resource korzystają z tej warstwy. Wyniki
input_required, tools/call, prompts/get oraz wszystkie żądania z requestState
lub inputResponses nie są cacheowane, także gdy odpowiedź deklaruje dodatni TTL.

Brak/zero/ujemny ttlMs oznacza brak cache. Dodatni integer jest ograniczony
lokalnie do300000ms. Wpis wymaga rozpoznanego cacheScope public/private;
brak scope wyłącza przechowywanie, błędna wartość jest błędem protokołu.
Świeżość sprawdza monotoniczny zegar przy dostępie; nie ma background pollingu.
Zegar jest pobierany przed dekodowaniem odpowiedzi, co konserwatywnie skraca
świeżość, gdy odczyt body lub dekodowanie trwają długo.

Strony katalogu mają niezależne TTL i klucze cursorów. Sprzeczne public/private
cache scopes na stronach są odrzucane. Błąd odświeżenia usuwa wszystkie strony
tej metody i poprzedni pin katalogu. refresh_tools omija cache i zaczyna od
pierwszej strony. Lokalnie atomowa publikacja katalogu nie jest obietnicą
transakcyjnego snapshotu zewnętrznego serwera; ten protokół takiej gwarancji
między stronami nie daje.

Powiadomienia/drop subskrypcji unieważniają wpisy przez epoch. Resource update
konserwatywnie unieważnia całą grupę resources w swoim kliencie. Cache lookup
ponownie sprawdza epoch po skopiowaniu JSON, przed udostępnieniem wyniku;
zmiana w tej chwili powoduje odrzucenie hitu. Deadline i stan klienta są
sprawdzane także przy cache hit, bez przedłużania90s budżetu.

Zaobserwowane401/403 na zwykłym odczycie lub podczas otwierania subskrypcji
usuwa cache/katalog, anuluje strumienie i blokuje dalsze użycie tego klienta.
Nowe uwierzytelnienie wymaga nowej instancji. HeaderMismatch/MethodNotFound/
InvalidParams z tools/call usuwa katalog i jego cache; nie powtarza narzędzia
ani nie pobiera definicji automatycznie.

Budżety:64 wpisy,8MiB łącznego zakodowanego payloadu i kluczy oraz16KiB na
klucz. Przekroczenie limitu usuwa najstarsze wpisy lub pomija zbyt duży wpis.
To pamięć danych odpowiedzi, a nie grant dostępu do narzędzia. Polityki E5.3
pozostają osobną warstwą.

Podstawa: [oficjalny kontrakt cache](https://modelcontextprotocol.io/specification/2026-07-28/server/utilities/caching).

## OAuth: odkrywanie metadanych

Moduł oauth::Discovery wykonuje read-only odkrywanie Protected Resource
Metadata i Authorization Server Metadata. Nie ma dostępu do bearer, refresh
tokenów lub sekretów klienta; nie rozpoczyna sesji użytkownika ani nie otwiera
przeglądarki. AuthorizationFlow opisany poniżej dodaje osobny etap PKCE/code
exchange i callback. Refresh/revoke oraz trwałe przechowywanie opisano niżej.

protected_resource_from_server wysyła nieuwierzytelniony POST server/discover
i przy401 odczytuje wszystkie WWW-Authenticate. Wynik zachowuje zarówno
metadane zasobu, jak i oryginalne Bearer challenge/scopes. Parser obsługuje
wiele nagłówków, inne schemes, quoted commas, escapes i whitespace przy equals;
odrzuca duplikaty lub niejednoznaczne Bearer challenges. Limit8 nagłówków/16KiB.

URL resource_metadata z challenge ma pierwszeństwo. Przy jego braku klient
sprawdza well-known przy ścieżce MCP, następnie root. Dla issuer ze ścieżką
sprawdza kolejno OAuth path insertion, OIDC path insertion i OIDC path appending;
dla root issuer sprawdza dwa warianty. Tylko404/405 prowadzi do następnego URL.
Nieprawidłowy dokument200 lub błąd auth/transportu nie są ukrywane fallbackiem.

Resource identifier musi odpowiadać originowi i granicy ścieżki skonfigurowanego
MCP endpointu. Issuer z metadanych jest porównywany do wybranego issuer dokładnie
jako string, bez normalizacji slash/host/scheme/port. Authorization/token
endpoints podlegają walidacji URL i origin policy. Wymagane są deklarowane code
response type i S256 PKCE. Parser zachowuje też deklaracje RFC9207 iss i CIMD.
Prerejestracja, publiczny profil CIMD i callback są opisane poniżej.

Operator konfiguruje originy dopuszczone do discovery; dokument nie może tej
listy rozszerzać. Origin MCP jest dopuszczony, maksymalnie8 dodatkowych originów
wymaga jawnej konfiguracji. HTTPS lub loopback HTTP, brak embedded credentials
i fragmentów, wyłączone redirects/env proxy. Dokumenty do64KiB, request do10s,
całe discovery do30s; maksymalnie8 issuerów i64 scopes do256B każdy.

Scopes z challenge są zachowywane jako wskazanie wymagań operacji nawet jeśli
nie występują w scopes_supported. Bez challenge stosowana jest lista z Resource
Metadata. To kandydaci do późniejszej zgody użytkownika i decyzji polityki,
a nie automatycznie nadane uprawnienia. Wybór jednego z wielu issuerów jest
jawnym parametrem, bez przenoszenia credentials między nimi.

Podstawa: [MCP authorization discovery](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/authorization-server-discovery),
[registration i issuer binding](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration).

## OAuth: PKCE, callback i wymiana kodu

Discovery.authorization_flow tworzy serwis dla konkretnego MCP endpointu,
resource, issuer i pre-registered klienta. Registration jest związana z dokładnym
issuer. Obsługiwane są klient publiczny none oraz confidential client_secret_basic
i client_secret_post, zgodnie z metodami zadeklarowanymi przez AS. Przy braku
token_endpoint_auth_methods_supported stosowany jest standardowy default Basic.
Zarezerwowane parametry OAuth w query endpointów są odrzucane, aby uniknąć
duplikatów redirect_uri/state/resource/credentials.

begin otrzymuje AuthContext z user/workspace/connection/connection_revision oraz jawnie wybrane scopes.
Context musi pochodzić z aktualnie uwierzytelnionego wywołania aplikacji; sam DTO
nie jest dowodem tożsamości. Serwis przechowuje maksymalnie32 rekordy przez300s,
pod hashem256-bit state. Verifier PKCE ma256 bitów losowości,43 znaki base64url
i challenge S256. URL do zatwierdzenia przez UI zawiera state/challenge, lecz
nie zawiera verifiera ani sekretu klienta. Resource jest przypięty w authorize
i późniejszym token request.

finish weryfikuje origin/path callbacku, limit16KiB i duplikaty parametrów.
Użytkownik, workspace i connection muszą odpowiadać rekordowi. Obcy context
nie może zużyć cudzego state. Właściwy callback atomowo usuwa rekord przed siecią;
spóźniony lub ponowiony callback jest odrzucany. Błąd HTTP nie przywraca rekordu
i nie powoduje automatycznej ponownej wymiany kodu.

Issuer jest sprawdzany przed obsługą code lub error. Gdy AS deklaruje wymóg iss,
brak pola kończy się odmową; każdy obecny iss musi dokładnie odpowiadać issuer,
także przy error. Nie normalizujemy host/scheme/slash/port podczas porównania.
Opis błędu i error_uri nie są wyświetlane ani otwierane przez bibliotekę.

Token request używa POST form-urlencoded do przypiętego endpointu, z resource,
redirect_uri i verifierem. Basic najpierw form-encode client ID/secret, potem
koduje ich połączenie Base64. Request ma10s, odpowiedź64KiB, bez redirects/proxy
lub retry. Bufory sekretów posiadane przez warstwę są Zeroizing; Registration,
pending records i TokenSet nie mają Debug/Serialize.

TokenSet zachowuje context, issuer, endpoint i scopes. Odczyt access tokenu
wymaga zgodnego context/endpoint i niewygasłego lokalnego deadline. Akceptowany
jest Bearer; scope escalation z odpowiedzi jest odrzucane, downscope zachowywane.
Lokalna ważność jest liczona konserwatywnie od początku token requestu, najwyżej
3600s; bez expires_in najwyżej60s. Refresh token jest przechowywany prywatnie;
jego cykl życia i oddzielną warstwę persistence opisano niżej. Ten moduł nie tworzy sesji
logowania Dunlin, nie zapisuje keyringu i nie przyznaje uprawnień do narzędzi.

Callback HTTP, powiązanie z browser cookie, zapis poświadczeń i UI connect są
jeszcze do podłączenia w aplikacji. Testy biblioteki nie zastępują tego odbioru.

## OAuth: refresh, revoke i status poświadczeń

TokenSet jest związany także z client ID i resource. Refresh/revoke ponownie
sprawdzają user/workspace/connection/revision, issuer, endpoint, resource i client
ID przed zmianą lokalnego stanu lub siecią. Connection revision jest dodatnią
safe integer; aktualną rewizję musi dostarczyć uwierzytelniona warstwa aplikacji.
Obcy lub stary context nie może odświeżyć ani wyłączyć cudzego poświadczenia.

refresh przyjmuje mutable TokenSet. Stan Active oznacza możliwość obsługi
poświadczenia, a termin access tokenu jest sprawdzany oddzielnie; wygasły access
może być odświeżony. Brak refresh tokenu zwraca Unsupported bez próby sieci
i bez zmiany Active. Przed POST stan przechodzi do RefreshUncertain. Taki stan
blokuje odczyt access tokenu i kolejne dodatnie odświeżenie.

POST ma grant_type=refresh_token, aktualny opaque refresh token, przypięty
resource i dotychczasowe scopes. Używa zamrożonej metody auth klienta. Po
zatwierdzonej odpowiedzi wymienia access token i ewentualnie refresh token;
brak nowego refresh tokenu zachowuje dotychczasowy, rotacja usuwa starą wartość.
Downscope jest zachowywane; rozszerzenie scopes, błąd HTTP, malformed response
lub utrata odpowiedzi pozostawiają RefreshUncertain. Nie ma automatycznego
retry starego refresh tokenu. Dalszą opcją jest nowa autoryzacja albo jawne revoke.

Revocation endpoint przechodzi ten sam URL/origin policy i kontrolę query
params. revocation_endpoint_auth_methods_supported ma niezależny default Basic
według RFC8414. Niedostępny endpoint/metoda nie blokuje utworzenia flow, ale
uniemożliwia potwierdzenie zdalnego revoke.

revoke ustawia RevocationUnconfirmed lokalnie przed siecią. Brak endpointu,
nieobsługiwana metoda lub błąd dostawcy pozostawia poświadczenie wyłączone
i nie jest raportowany jako sukces zdalny. Jeśli możliwe, wysyła osobne żądania
dla znanego refresh tokenu i access tokenu, z token_type_hint i odpowiednim
uwierzytelnieniem klienta. HTTP200, także dla tokenu już nieważnego, potwierdza
to sprzątanie; body jest ignorowane zgodnie z RFC7009.

Po obu potwierdzeniach stan jest Revoked, wartości sekretów są usuwane,
a ponowne revoke nie wysyła HTTP. Po niepewnym wyniku użytkownik może jawnie
powtórzyć wyłącznie ujemne sprzątanie; brak auto retry. Revoked dotyczy znanych
wartości tokenów, nie obiecuje odwołania całego grantu lub nieznanych tokenów
wydanych przy utraconej odpowiedzi. Kaskadowe odwołanie zależy od polityki AS.

Podstawowe refresh/revoke działają na TokenSet w pamięci. Dla trwałych
poświadczeń aplikacja powinna używać poniższej ścieżki z dziennikiem, zamiast
odtwarzać kopię Active i wywoływać bezpośrednio refresh.

Podstawa: [refresh RFC6749](https://www.rfc-editor.org/rfc/rfc6749.html#section-6),
[revoke RFC7009](https://www.rfc-editor.org/rfc/rfc7009.html#section-2),
[metadata RFC8414](https://www.rfc-editor.org/rfc/rfc8414.html#section-2).

## Trwały dziennik OAuth i generacje sekretów

CredentialJournal ma implementację w głównym Store serwera (migracja37→38).
Przechowuje właściciela, workspace, rewizję połączenia, fingerprint konfiguracji,
generację, stan, zamiar operacji oraz referencje i SHA256 prywatnych rekordów.
Tokeny dostawcy nie trafiają do SQLite. Każda operacja sprawdza aktualne aktywne
członkostwo i przypisanie połączenia do konkretnego użytkownika; rola admin nie
udostępnia cudzego osobistego połączenia.

install_durable zużywa TokenSet z callbacku. Rezerwuje w SQLite unikalną
referencję, utrwala tożsamość koperty przez prepare_write, a następnie przekazuje
prywatny rekord do funkcji zapisu magazynu sekretów i dopiero potem aktywuje
generację przez CAS. refresh_durable odczytuje aktywną
generację, weryfikuje prywatny rekord i zatwierdza stan refreshing **przed HTTP**.
Po odpowiedzi dostawcy przygotowuje metadane nowej koperty, zapisuje nowy rekord
pod nową referencją i wykonuje CAS. Nie nadpisuje poprzedniego sekretu.

Migracja41→42 zachowuje przy każdej nowej referencji oryginalną rewizję,
fingerprint konfiguracji, generację oraz hash koperty. prepare_write sprawdza
aktualny zamiar i uprawnienia przed zapisem do vaulta; ponowienie z innym hashem
jest odrzucane. commit wymaga dokładnie tych przygotowanych metadanych. Błąd
vaulta, niepewny wynik set lub późniejsze disable nie usuwają tego zapisu.
Historyczna migracja uzupełnia hash tylko aktywnej/zablokowanej refreshing koperty,
której tożsamość wynika z istniejącego wiersza. Dla staged zachowuje znaną
rewizję/binding/generację bez wymyślania hasha. Starsze wycofane lub odłączone
referencje pozostają nieznane: obecna rewizja połączenia nie dowodzi rewizji
ich koperty. Ten zapis przygotowuje trwałe revoke i GC, ale ich nie wykonuje.

Nie ma transakcji rozproszonej SQLite/keyring;
bezpieczeństwo wynika z kolejności zapisów, unikalnych referencji i blokady.

Awaria po rezerwacji, utrata odpowiedzi, błąd magazynu sekretów lub przegrany CAS
nie przywracają starej generacji. Niezatwierdzony zamiar pozostaje nieaktywny również
po ponownym otwarciu bazy. Jeśli przerwanie nastąpi już po zatwierdzeniu SQL, dostępna
jest wyłącznie nowa zapisana generacja. Nie ma automatycznego powrotu do starego Active
ani ponowienia starego refresh tokenu.
Konkurencyjna rezerwacja przegrywa. disable zwiększa rewizję połączenia i blokuje
zatwierdzenie spóźnionego zapisu; ponowna instalacja wymaga nowego context.
Nie zastępuje to zdalnego RFC7009 revoke.

Prywatny rekord JSON jest ograniczony do64KiB, tworzony/odczytywany w Zeroizing
i związany z user/workspace/connection/revision, fingerprintem i generacją.
Fingerprint obejmuje przypięte endpointy, resource, issuer, redirect, client ID,
metodę uwierzytelnienia oraz sekret klienta. Odtworzenie sprawdza digest i binding,
ważność do3600s oraz cofnięcie zegara względem zapisu. access_token_durable
ponownie sprawdza bieżący dziennik po pobraniu sekretu; aplikacja musi wywoływać
tę ścieżkę przy dispatchu zamiast cache'ować bearer między operacjami.

Callbacks get/put są zaufanym adapterem prywatnego magazynu sekretów i nie mogą
zapisywać rekordów w logach, SQLite ani plikach jawnych. OAuthCredentials w
crates/vault podłącza te operacje do istniejącego SecretStore. Konstruktor relay
wybiera odseparowaną przestrzeń poświadczeń serwera i sam nie wykonuje operacji
keyringu. Adapter akceptuje wyłącznie referencje mcp-oauth:<UUID>, ogranicza rekordy
do64KiB, nie udostępnia błędów backendu i nie tworzy polityk/narzędzi vault MCP.
Metody install/refresh/access_token/disable są synchroniczne; aplikacja wykonuje
operacje z keyringiem/HTTP na ograniczonej puli blokującej.

CredentialJournal ma adapter Mutex<J>: blokada jest brana tylko na pojedynczą
metodę dziennika, nigdy na HTTP lub callback magazynu sekretów. Poisoned Mutex
zwraca Storage; nie ma automatycznego odzyskiwania dostępu. Dla AppState
Arc<Mutex<Store>> przekazuje się do OAuthCredentials bez wcześniejszego lock().
Test z SQLite, lokalnym OAuth i interfejsem SecretStore sprawdza try_lock podczas
get/set i odbioru HTTP, odłączenie podczas HTTP, zapis keyringu z niepewnym wynikiem,
późny CAS i odtworzenie po zamknięciu wszystkich starych połączeń Store.
Rzeczywisty keyring i konto dostawcy nie zostały użyte.

Serwer udostępnia GET /v1/web/workspaces/{id}/mcp-connections/{connection}
oraz POST pod tym samym adresem z /disconnect. To osobiste połączenia: wymagają
aktualnego cookie/device, polityki IdP/hosta, aktywnego członkostwa i zgodnego
właściciela. GET zwraca id/revision/status oraz opcjonalne provider_id, bez bindingu, digestu lub
referencji sekretów. Statusy: connected, authorization_incomplete,
reauthorization_required, disabled. Pending refresh jest przedstawiany jako
wymagający ponownej autoryzacji, nie jako gotowy do retry starego tokenu.

Disconnect wymaga same-origin i expected_revision, zwiększa rewizję i atomowo
zapisuje zdarzenie audytu mcp_oauth_disconnected; powiadomienie następuje po commit.
Powtórzenie z bieżącą rewizją już odłączonego połączenia nie tworzy nowego zdarzenia;
stara rewizja powoduje409. Odpowiedź jawnie wskazuje provider_revocation=not_confirmed.
Ten endpoint nie czyta keyringu ani nie wysyła HTTP do dostawcy. Start/callback opisano
poniżej. Broker katalogu odświeża lokalnie wygasłe poświadczenie przed dispatch;
wykonanie zatwierdzonych narzędzi jest podłączone do brokera runtime opisanego
w dalszej części dokumentu.

Referencje zarezerwowanych generacji są zachowywane także po utracie CAS, aby
późny zapis nie zniknął z ewidencji. Limit wynosi64 osobiste połączenia na
user/workspace i256 referencji obciążających pojemność na połączenie; wyczerpanie
blokuje nowe operacje bez zmiany istniejącego stanu. Trwały remote-revoke opisano
poniżej. Migracja47 oddziela tę pojemność od ograniczonej historii cleanup;
niepewny późny zapis nie uzyskuje prawa do automatycznego wygaszenia.

## Rozpoczęcie i callback OAuth w serwerze

Operator konfiguruje do16 dostawców w tablicy mcp_oauth w relay.toml. Provider ma
id/name, endpoint MCP, przypięty issuer/client_id/redirect_uri, allowed_origins,
scopes oraz authentication public/basic/post/client_metadata_document. Dla confidential klienta podaje
wyłącznie client_secret_ref z przestrzeni mcp-oauth-client:, bez sekretu w TOML.
Przeglądarka wybiera identyfikator; nie przesyła endpointu, issuer, origin allowlist,
redirectu ani danych rejestracji. Zmiana dostawcy na istniejącym connection ID jest
odrzucana. Prerejestracja i CIMD są wybierane jawnie przez operatora;
brak wsparcia CIMD nie uruchamia automatycznej prerejestracji ani DCR.

```toml
[[mcp_oauth]]
id = "work"
name = "Work MCP"
endpoint = "https://mcp.example.test/mcp"
issuer = "https://auth.example.test"
client_id = "registered-client-id"
redirect_uri = "https://build.tail123.ts.net/v1/web/mcp/oauth/work/callback"
allowed_origins = ["https://auth.example.test"]
scopes = ["files:read"]
```

Adresy i client_id w przykładzie należy zastąpić własną konfiguracją; sam hostname
callbacku nie potwierdza prywatności Serve/Funnel. Nie podłączono realnego konta.

### Publiczny klient CIMD

Operator może zastąpić `client_id` adresem własnego publicznego dokumentu,
np. `https://client.example/oauth/client.json`, i ustawić
`authentication = "client_metadata_document"`. Dokument musi być dostępny dla
serwera autoryzacji pod tym dokładnym adresem HTTPS. Hosting dokumentu jest
osobnym wdrożeniem operatora; prywatny relay nie otwiera publicznego listenera,
Funnel ani endpointu bez autoryzacji. Sam publiczny dokument nie wymaga dostępu
do prywatnego callbacku z sieci AS: callback obsługuje przeglądarka użytkownika.
Provider nadal przypina issuer i dokładny callback; `client_secret_ref` jest
w tym trybie zabroniony.

Publiczny JSON można wyeksportować bez uruchamiania serwera, dostępu do bazy,
keyringu lub sieci:

```sh
dunlin-relay mcp-client-metadata --config relay.toml --provider work > client.json
```

Eksport zawiera tylko client_id, client_name, dokładny redirect_uris,
grant_types authorization_code/refresh_token, response_types code i
token_endpoint_auth_method none. Biblioteka udostępnia ten sam walidowany model
ClientMetadataDocument. Nie zawiera kont, połączeń użytkowników ani tokenów.
Operator publikuje wyeksportowane JSON bez zmian pod skonfigurowanym URL,
z Content-Type application/json; realne publikowanie wymaga osobnej zgody.

Profil Dunlin ogranicza client_id do256 bajtów, kanonicznego HTTPS z niepustą
ścieżką inną niż root, bez query, fragmentu, userinfo, normalizacji i segmentów
dot. Dokument ma1–16 unikatowych callbacków bez wildcard/query, nazwę do120
bajtów i stałe tryby publicznego klienta. Brak private_key_jwt i automatycznego
DCR. Konstrukcja rejestracji nie pobiera URL dokumentu: jego odczyt i porównanie
client_id/redirect_uris należą do AS.

Discovery wymaga jawnego client_id_metadata_document_supported=true, S256,
response_type code i dozwolonego token auth none przed utworzeniem oczekującej
autoryzacji. PKCE, resource binding i jednorazowy callback pozostają wymagane.
Client ID jest zachowany dokładnie w authorize, token i refresh. Durable binding
rozróżnia CIMD od prerejestrowanego public klienta nawet przy tym samym ID;
zmiana trybu nie przejmuje istniejącej generacji sekretu. Wybór CIMD nie zmienia
polityki dostępu do narzędzi ani nie przekazuje OAuth do runnera.

Podstawa: [MCP2026 Client Registration](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration)
i [CIMD draft00, sekcja3](https://www.ietf.org/archive/id/draft-ietf-oauth-client-id-metadata-document-00.html#section-3),
odczytane2026-11-13. Obsługiwany jest opisany publiczny profil klienta;
nie jest to implementacja serwera autoryzacji ani dowód dostępności hostingu.

GET /v1/web/workspaces/{id}/mcp-providers zwraca tylko id/name zatwierdzonych
providerów dla aktywnego członka. POST pod /mcp-providers/{provider}/authorize wymaga
same-origin i aktualnego browser cookie. Body jest puste lub zawiera display_name
dla nowego połączenia albo connection_id i expected_revision dla jawnego ponownego logowania.
Opcjonalna nazwa jest przycinana, ma1–120 znaków bez znaków kontrolnych i należy
do prywatnych metadanych właściciela. Migracja39→40 dodaje display_name bez zmiany
poświadczeń i referencji. Ponowne logowanie zachowuje wcześniejszą nazwę.
Przekazane ID musi już należeć do bieżącego użytkownika w tym workspace; brak lub
cudze ID daje404. Nowe ID jest generowane wyłącznie przez serwer.
Dla aktywnego/niepewnego poświadczenia najpierw wymagane jest lokalne disconnect.
Odpowiedź zawiera authorization_url, connection_id i revision, bez sekretów.

Bootstrap wykonuje wyłącznie nieuwierzytelniony server/discover/resource metadata
oraz discovery przypiętego issuera; nie wywołuje narzędzi MCP. Po metadanych SQL
utrwala stan authorizing i fingerprint przed wydaniem URL. Migracja38→39 zachowuje
stare poświadczenia, referencje i niepewne refreshing bez ich reaktywacji; dodaje
opcjonalne provider_id. Ponowna autoryzacja niedokończonego authorizing zwiększa
revision, blokując stary callback. GET /mcp-connections pozwala właścicielowi
odnaleźć niedokończone połączenie także po utracie odpowiedzi start.

Główne cookie sesji pozostaje SameSite=Strict. Start ustawia osobne host-only
HttpOnly cookie dunlin_mcp_oauth_flow z SameSite=Lax, ścieżką /v1/web/mcp/oauth,
TTL300s i Secure dla HTTPS, również na loopback. Pending przechowuje wyłącznie jego
hash, hash pierwotnej sesji, device/user/context, hash state, digest konfiguracji
oraz flow z PKCE. Limit wynosi32 pending/starting globalnie,4 na użytkownika,
jedna oczekująca autoryzacja na sesję przeglądarki i2 równoległe operacje discovery,
keyring/HTTP. Odczyt/zapis keyringu i sieć działają na spawn_blocking, bez Store lock.

Callback GET /v1/web/mcp/oauth/{provider}/callback wymaga właściwego Host,
provider/config digest, flow cookie i state. Nie wymaga przesłania głównego Strict
cookie przy nawigacji od AS. Weryfikuje pierwotną sesję przez jej zachowany hash,
aktualne urządzenie, użytkownika, IdP/host policy, członkostwo i rewizję. Te sprawdzenia
są ponawiane wewnątrz tej samej transakcji SQLite co zapis generacji. Obce/wadliwe
state lub cookie nie zużywają prawidłowego flow i nie usuwają jego cookie.

Po przyjęciu callback jest jednorazowy. Biblioteka sprawdza iss/PKCE i wymienia
code, po czym OAuthCredentials zapisuje prywatny rekord przez zatwierdzony journal.
Wylogowanie lub disconnect w trakcie HTTP/keyring blokuje aktywację poświadczenia.
Sukces usuwa flow cookie i przekierowuje wyłącznie do /. Błąd po konsumpcji pokazuje
stały komunikat, bez opisu od AS, code lub tokenu; nie powoduje retry. Odpowiedzi
mają no-store/no-referrer, również przy wczesnym odrzuceniu cookie/state. Po restarcie pending/verifier nie są odtwarzane: stary
callback jest odrzucany, a jawny nowy start wymaga świeżej rewizji i nowego state.

Podstawa: [OAuth Security BCP](https://www.rfc-editor.org/rfc/rfc9700.html#section-4.7)
i [atrybuty cookie SameSite](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie#samesitesamesite-value).
REST i lokalne fixture nie zastępują realnego dostawcy/keyringu ani końcowego
odbioru E5.2/E10. Panel ma osobny scenariusz Chromium opisany poniżej;
konkretny wynik jest zapisany w progress.md.

## Panel połączeń MCP

Widok Połączenia MCP wybiera zespół i udostępnionego przez operatora dostawcę.
Użytkownik nadaje prywatną nazwę konta, rozpoczyna logowanie oraz widzi własne
połączenia i ich statusy. Niedokończone logowanie można anulować; ponowne logowanie
istniejącego połączenia wymaga wcześniejszego lokalnego odłączenia. Widok wyraźnie
oddziela odłączenie w Dunlin od cofnięcia zgody u dostawcy.

Żądania panelu mają X-Dunlin-Expected-User. Serwer porównuje go z bieżącą tożsamością
cookie; nie jest to poświadczenie ani źródło uprawnień. Mismatch lub duplikat daje401,
aby stara karta nie działała jako inna osoba po zmianie zalogowanego użytkownika.
Zmiana widoku przerywa oczekiwanie; spóźniona odpowiedź nie przekierowuje do AS.
Zmiana członkostwa ponownie sprawdza dostępne zespoły.

Frontend sprawdza ograniczone odpowiedzi, ID/revision oraz URL OAuth, w tym
redirect do dokładnego własnego callbacku, state i S256 challenge. Zapis nie jest
automatycznie ponawiany. Przy niepotwierdzonym wyniku start/disconnect/inspect kolejne
mutacje są wstrzymane do jawnego odświeżenia listy z serwera.

Po sukcesie callback nadal kieruje do /. Jednorazowa preferencja sessionStorage
zawiera wyłącznie user ID i workspace ID, aby otworzyć właściwy panel po powrocie;
obca tożsamość ją odrzuca. URL autoryzacji, state, verifier, code i tokeny nie trafiają
do tego wpisu ani localStorage. Brak dostępu do storage nie blokuje samego OAuth.

## Broker katalogu prywatnego połączenia

Jawne POST /v1/web/workspaces/{workspace}/mcp-connections/{connection}/inspect
z expected_revision sprawdza katalog przez produkcyjnego negotiated Client.
Wymaga same-origin i świeżego browser cookie; oczekiwana tożsamość przesłana
przez panel musi odpowiadać bieżącej sesji.
Provider jest rozwiązywany wyłącznie z własnego połączenia i katalogu operatora.
Zwraca connection_id/revision, potwierdzoną wersję protokołu i ograniczoną listę
nazw narzędzi. Nie wykonuje tools/call ani nie nadaje grantów do narzędzi.
Panel udostępnia przycisk Sprawdź narzędzia i oznacza wynik jako katalog dostawcy.

Broker przypina rewizję członkostwa i połączenia oraz generację poświadczenia.
Każda operacja dziennika ponawia browser/device/IdP/host policy i sprawdzenie
członkostwa w tej samej transakcji SQL co odczyt/zapis generacji. Odczyt sekretu
i HTTP odbywają się poza Store lock. Odebranie i ponowne nadanie członkostwa
nie przywraca uprawnień trwającego sprawdzenia katalogu.

Prawidłowy lokalnie wygasły envelope może spowodować jedną rotację przed MCP
dispatch. Rezerwacja CAS dotyczy dokładnie odczytanej wygasłej generacji;
równoległy zapis nowej generacji uniemożliwia rotację na podstawie starego odczytu.
Zamiar refresh jest trwały przed token HTTP. Utrata odpowiedzi blokuje stare
poświadczenie; ponowne inspect nie ponawia starego refresh tokenu. Provider401
nie powoduje automatycznej rotacji ani ponowienia requestu MCP.

DispatchGuard przekazuje transportowi funkcję sprawdzającą aktualne uprawnienia
i dokładną generację. Jest wywoływana przed/po HTTP i przy odczycie cache;
odmowa jest terminalna dla klienta, również po ponownym nadaniu uprawnień.
Obejmuje modern i jawny adapter2025, jego initialize/notification/DELETE oraz
odczyt subskrypcji. Wynik końcowy brokera ma dodatkowe sprawdzenie dziennika.
Nie da się cofnąć żądania już przyjętego przez providera; późny wynik jest odrzucany.

Operacja zajmuje jeden z dwóch wspólnych slotów OAuth/connection service i nie
ma automatycznego retry w UI. POST jest wymagany, ponieważ inspect może zapisać
nową generację tokenów. To integracja katalogu z prywatnym OAuth, nie odbiór
runtime tools, allowlist, scoped secrets, kill switch lub human approvals.

## Weryfikacja i otwarte elementy

Testy wykonuje izolowany suite mcp, z lokalnymi fixture HTTP bez kont dostawców:
[instrukcja](runtime-checks.md#izolowane-testy-klienta-mcp).
Dokładne snapshoty, wyniki i nieudane przebiegi są w [progress.md](progress.md).

## Przypięcia definicji narzędzi i allowlista profilu

Tool.definition_digest waliduje dopuszczoną definicję (name, inputSchema,
opcjonalne outputSchema), porządkuje rekursywnie klucze obiektów JSON i liczy
SHA256 z prefiksu dunlin.mcp.tool.v1, jednego bajtu NUL i kompaktowego JSON UTF-8.
Kolejność tablic i reprezentacja liczb pozostają częścią definicji. To własny
wersjonowany codec, nie deklaracja RFC8785 ani odcisk dowolnych nieobsługiwanych
adnotacji serwera. Nie pobiera zewnętrznych schematów. Zmiana nazwy albo schematu
we/wy zmienia pin; kolejność kluczy nie zmienia go także przy preserve_order.

Prywatne inspect zachowuje tools jako listę nazw i dodaje tool_pins z
provider_id/name/definition_digest. Lista pinów może być podzbiorem nazw: nazwy
nieobsługiwane przez politykę (np. wildcard) nie dostają pinu. API ogranicza
cały zakodowany katalog do256KiB, bez zwracania surowych schematów lub sekretów.
Ostateczne sprawdzenie authority/generacji następuje po obliczeniu pinów i
sprawdzeniu rozmiaru. UI weryfikuje unikatowe nazwy, zgodność z obserwowanym
katalogiem i providerem połączenia; starsze odpowiedzi bez pinów pozostają
odczytywalnym katalogiem nazw. Podgląd pozwala skopiować pin do Agent Buildera.

AgentDefinition schema2 zapisuje do32 dokładnych grantów dla8 providerów.
allows_mcp sprawdza provider/name/digest oraz poprawność całej polityki.
Nie łączy się z dostawcą i nie zastępuje decyzji użytkownika ani aktualnej
authority. Publikacja wymaga ManageProject i tworzy niezmienną wersję; prywatne
connection IDs, URL/tokeny nie są częścią profilu. Lokalny broker opisany poniżej
wymaga jawnych prywatnych powiązań sesji i nie pomija opublikowanej polityki.

## Lokalny broker wykonania MCP

Actor lokalnej sesji Unix tworzy prywatny socket i stdio konfigurację ACP
dunlin-session-tools. Proces dunlin-relay --mcp-tools obsługuje lokalny adapter
MCP2025 JSON lines; broker używa negocjowanego klienta HTTP2026/2025.
Facade startuje przed inicjalizacją aplikacji: nie otwiera bazy ani keyringu.
Harness otrzymuje tylko ścieżkę socketu i losowy dowód ograniczony do sesji;
OAuth access/refresh tokens i client secrets pozostają w brokerze.
Sandbox otrzymuje dokładny socket i executable, bez katalogu bazy lub vault.

Przed startem procesu broker pobiera katalog i sprawdza wszystkie piny profilu.
Udostępnia wyłącznie dozwolone narzędzia pod stabilnymi aliasami. Przed zgodą
sprawdza argumenty i świeży katalog; po zgodzie ponownie rozwiązuje OAuth,
sprawdza katalog i wykonuje tools/call z hostowym permitem. Klient HTTP nie jest
utrzymywany podczas oczekiwania na człowieka. Zmiana definicji wymaga nowego
profilu; nie akceptuje po cichu nowego schematu.

Widok zgody pokazuje dostawcę/narzędzie i domyślnie rozwinięte, czytelnie zapisane
arguments. Techniczne connection/revision/digest pozostają w pełnym niezmiennym
wywołaniu dziennika AHP; formatowanie UI nie zmienia zatwierdzanego payloadu.

Guard sprawdza aktualne urządzenie/OIDC, projekt, lokalny binding/grant/checkout,
lease, prywatną rewizję połączenia i generację tokena. Przy dispatch dodatkowo
sprawdza zatwierdzające urządzenie, dokładne argumenty, aktywną turę i TTL zgody.
Odłączenie konta podnosi rewizję i odcina starą sesję. Sprawdzenie przed zwrotem
danych odbywa się także po serializacji odpowiedzi. Cofnięcie dostępu nie cofa
operacji już wykonanej u zewnętrznego dostawcy.

Każda nowa operacja stdio dostaje osobny UUID, niezależny od JSON-RPC id, który
może być ponownie użyty. Facade nie ponawia requestu po utracie odpowiedzi.
Broker rozpoznaje powtórzenie tego samego identyfikatora transportowego i
argumentów bez ponownego HTTP; inny payload dla tego ID jest konfliktem.
Niepewny wynik albo błąd zapisu dziennika przerywa actor i zachowuje unknown.
Wynik definite jest utrwalany przez hostowy SDK przed odpowiedzią do harnessu.

Limity obejmują8 brokerów sesyjnych,2 równoległe operacje OAuth/MCP,
256 receipts na sesję, katalog4MiB, strony katalogu192KiB i ramki256KiB.
Argumenty/wyniki mają dodatkowe limity SDK poniżej. Zgoda runtime trwa najwyżej
120s i nie przekracza czasu tury/sesji. Socket jest w prywatnym katalogu0700,
ma uprawnienia0600; zakończenie actora unieważnia broker. Zdalny proxy i form
elicitation mają kontrolowany odbiór opisany wyżej i w remote-session-plan.md.
Trwałe provider revoke, bounded cleanup/retencja i URL elicitation mają
późniejsze odbiory opisane niżej. Ogólne scoped secrets pozostają w E5.3;
wyniki poszczególnych odbiorów są w progress.md.

Rdzeń brokera korzysta z hostowego Authority, oddzielonego od procesu ACP.
Utrwala własną kopię pinów session/workspace/owner/lease/device/binding/profile
i odmawia ich zmiany także wewnątrz transakcji OAuth. Sam egzekwuje deadline;
adapter dostarcza świeże kontrole lokalne lub przyszły dowód worker/export.
Powiadomienia i interrupt muszą być nieblokujące i nie mogą ponownie otwierać
Store pod jego mutexem. Slot należy do rzeczywistego zadania, nie do waitera:
anulowanie oczekiwania na katalog nie zwalnia pojemności pracującego brokera.
Proponowany [kontrakt v5](remote-session-plan.md#rozszerzenie-mcp-osobna-wersja-v5-w-realizacji)
nie jest jeszcze aktywnym transportem ani zgodą na zdalne wykonanie.

## Jednorazowa zgoda na wywołanie MCP w dzienniku runtime

session-runtime udostępnia oddzielny hostowy kontrakt McpInvocation,
McpPermissionChallenge, McpApproval i McpDispatchPermit. Nie są deserializowane
z żądania runnera i nie są poświadczeniami OAuth. Broker musi sam rozwiązać
provider/połączenie i sprawdzić ownership, allowlistę, schemat oraz świeżą authority.
Sam SDK nie wykonuje HTTP; integracja lokalna jest opisana powyżej.

Zgoda przypina provider_id, connection_id/revision, nazwę narzędzia, digest całej
zwalidowanej definicji i dokładne arguments. Prywatny request w istniejącej tabeli
permissions ma origin dunlin.mcp.v1; nie wymaga migracji bazy. Host generuje tool ID
z zarezerwowanego prefiksu dunlin:mcp:. Zwykłe raporty tool_call/tool_call_update ACP
i rejestracje ACP nie mogą użyć tej przestrzeni. Zwykłe consume_permission oraz
permission_delivered nie mogą zużyć ani potwierdzić zgody MCP.

register_mcp_permission tworzy trwały challenge i zwykły AHP toolCallReady z
pełną treścią wywołania i opcjami approve-once/deny. Istniejący dispatcher AHP
przyjmuje dokładną jednorazową decyzję użytkownika; edycja treści zatwierdzanego
wywołania nie jest wspierana. Zmiana dowolnego przypiętego pola wymaga nowej zgody.
consume_mcp_permission zwraca osobny, nieklonowalny permit tylko po zatwierdzeniu;
odmowa trwale unieważnia challenge bez permitu. validate_mcp_dispatch ponownie
sprawdza przypięcie, aktualny lease/epoch, authority, aktywną turę, anulowanie,
TTL i stan consumed, a także zwraca dowód urządzenia zatwierdzającego. Broker
musi niezależnie sprawdzić aktualne uprawnienia tego urządzenia i generację OAuth
przed i po sieci. Sam permit nie daje dostępu do narzędzia lub sekretu.

Identyfikator requestu jest jednorazowy w całej sesji, również po odmowie,
sukcesie, błędzie, niepewnym wyniku i ponownym otwarciu bazy. Dwie niezależne
rejestracje SQLite nie mogą utworzyć dwóch zgód dla tego samego requestu.
finish_mcp_dispatch zapisuje definite complete/failed oraz hostowy wynik AHP
w jednej transakcji. Wynik pozostaje danymi MCP w structuredContent.
Unknown i awaria po consumed pozostawiają trwałe unknown przy przerwaniu/recovery;
nie odtwarzają uprawnienia do automatycznego ponowienia operacji. Błąd zapisu
eventu lub przekroczenie limitu wycofuje także zapis delivered. Integrator musi
wtedy zakończyć/przerwać runtime, żeby consumed zostało oznaczone jako unknown.

Limity invocation: arguments jako obiekt do8KiB,1024 węzłów i głębokości32;
request ID do128 bajtów, provider do64, connection/tool do128/256, digest64 lowercase
hex, dodatnia rewizja do2^53−1. Retained result ma limit48KiB/4096 węzłów/depth32,
tak aby mieścił się w zdarzeniu dziennika do64KiB. Serializacja używa licznika
bajtów bez alokowania nieograniczonego bufora. Nadal obowiązuje wspólna pojemność
8192 zgód globalnie,256 na sesję,8 aktywnych i TTL1–300s.

Form i URL elicitation oraz trwały OAuth revoke/cleanup/retencja mają odbiory
opisane w dalszych sekcjach. Ogólne scoped secrets pozostają w E5.3, a odbiór
prawdziwych dostawców/platform w E10. Kontrolowany broker nie stanowi odbioru
całego E5.3 ani zewnętrznych bramek E10.

Podstawa kontraktu: [transport](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http),
[MRTR](https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/mrtr),
[elicitation](https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation).

## Trwałe odwoływanie zapisanych tokenów dostawcy

POST /v1/web/workspaces/{workspace}/mcp-connections/{connection}/revoke wymaga
cookie właściciela, same-origin i expected_revision. Nie przyjmuje endpointów,
tokenów, referencji ani zamiaru odwołania od klienta. Przed discovery, keyringiem
lub HTTP do AS zapisuje w jednej transakcji nową rewizję disabled, dziennik
odwołania i snapshot referencji obciążających aktywny limit (maksymalnie256).
Posprzątana historia migracji47 nie trafia ponownie do tego snapshotu; trwały
wynik AS pozostaje przy dokładnych metadanych referencji.
Oryginalna rewizja i generacja każdej koperty pozostają niezależne od rewizji
odłączonego połączenia. Migracja43 dodaje bounded JSON dziennika do połączenia;
nie zmienia tokenów ani dotychczasowych metadanych generacji.

Każda pozycja jest jednorazowo oznaczana unknown przed odczytem sekretu.
Tylko koperta o udowodnionym hashu, właścicielu, oryginalnej rewizji, generacji
oraz identycznym bindingu OAuth może trafić do przypiętego endpointu AS.
Odwołanie wysyła refresh token, następnie access token, z właściwym token_type_hint
i metodą uwierzytelnienia rejestracji. Przed i po każdym POST ponownie sprawdza
cookie/device/IdP, przypiętą rewizję członkostwa, disabled connection i cały
snapshot zamiaru. Nie utrzymuje Store mutex ani transakcji SQL podczas I/O.
Budżet pętli HTTP wynosi30s, pojedynczy rozpoczęty POST ma dotychczasowy timeout10s.
Po upływie budżetu nie rozpoczyna kolejnego POST. Slot blocking workera pozostaje
zajęty do rzeczywistego zakończenia, również gdy przeglądarka przestanie czekać.

GET połączenia zwraca wyłącznie bezpieczny receipt: state, revision, confirmed
oraz unconfirmed. Liczniki dotyczą kopert, nie liczby tokenów lub grantów.
confirmed_known_tokens oznacza potwierdzenie wszystkich znanych zapisanych kopert,
a nie całej zgody AS lub tokenów utraconych w odpowiedzi na rotację. Zero kopert,
nieznany staged/retired rekord, brak endpointu, zmiana konfiguracji, błąd vaulta
lub utrata odpowiedzi nie dają takiego potwierdzenia. SQL nigdy nie zawiera tokenów;
przeglądarka i runner nie otrzymują referencji, bindingu, hashów ani request state.

Zakończona pętla z niepewnym wynikiem ma state unconfirmed. Właściciel może jawnie
ponowić odwołanie na nowej rewizji albo rozpocząć nowe logowanie; nie ma retry
w tle. To nowe żądanie CAS, nie ponowienie starego HTTP zamiaru. Stare
expected_revision przegrywa również po utracie odpowiedzi i restarcie service.
Ponowienie zachowuje potwierdzenia identycznych kopert (reference, oryginalna
rewizja, binding, generacja, SHA) i nie wysyła ich do AS ponownie. Niezakończone
pozycje są kolejkowane według rewizji ostatniej próby: najpierw nigdy niepodjęte,
potem najdawniej próbowane. Te dane przetrwają restart, dzięki czemu ograniczony
budżet nie wymusza zaczynania od tych samych starych rekordów. Potwierdzenie ze
starej rejestracji może zostać zachowane bez wysyłania jej tokenów do nowej.
Kolejny /revoke na już potwierdzonej, nadal odłączonej rewizji jest odrzucany;
po nowym logowaniu nowa generacja może zostać odwołana osobno. Nowa autoryzacja
po receipt podnosi rewizję, aby poprzedni CAS nie mógł odłączyć nowej sesji OAuth.

Stan unfinished oznacza brak trwałego zakończenia, nie dowód działającego procesu.
Blokuje nowe logowanie i kolejne odwołanie; restart nie przejmuje takiego zamiaru
ani nie uruchamia HTTP. Nowa jawna ścieżka reconcile opisana poniżej wymaga
uzyskania lease wykluczającego żywego workera; legacy bez takiej ochrony
pozostaje osobną bramką migracji po potwierdzonym zatrzymaniu starej instancji.

Wspólny audit ma dotychczasowy limit100000 zdarzeń. Jego wyczerpanie nie blokuje
odłączenia i nie zwiększa tabeli ponad limit: prywatny bounded dziennik operacji
pozostaje trwały także wtedy, gdy nie ma miejsca na dodatkowy wpis team_audit.
Błędy samego SQL nadal powodują rollback przed jakimkolwiek sekretowym I/O.

UI ma odrębne „Odłącz w Dunlin” i „Odwołaj tokeny u dostawcy”, jawne ponowienie
zakończonego unconfirmed i status odtwarzany przez odświeżenie listy. Utrata
odpowiedzi wstrzymuje kolejne mutacje do odczytu statusu. Revoke nie usuwa kopert:
GC musi nadal zachować tombstones i powtarzać ujemne sprzątanie, ponieważ
spóźniony writer może zapisać sekret po wcześniejszym delete. Odbiór prawdziwego
AS/keyringu/platform pozostaje w E10. Retencję/odzyskiwanie limitu i operatorową
migrację legacy unfinished opisano w dalszych sekcjach.

## Lokalne sprzątanie odwołanych sekretów

POST /v1/web/workspaces/{workspace}/mcp-connections/{connection}/cleanup wymaga
cookie właściciela, same-origin, expected_revision połączenia oraz osobnego
expected_cleanup_revision (0 przed pierwszym przebiegiem). Migracja44 dodaje
osobny bounded JSON dziennika sprzątania. Odpowiedź i GET połączenia ujawniają
wyłącznie connection_revision, revision przebiegu, status, delete_acknowledged
oraz unconfirmed; referencje, bindingi, hashe i wartości pozostają na serwerze.

Przebieg wybiera tylko niezmienne koperty, których odwołanie u AS ma trwałe
potwierdzenie, z dokładną zgodnością metadanych referencji. Bieżąca aktywna lub
przygotowywana generacja jest chroniona. Sprzątanie może zatem usunąć starą
potwierdzoną generację po nowym logowaniu bez usuwania aktualnych credentials.
Nie potrzebuje konfiguracji dostawcy, nie wykonuje discovery/HTTP do AS i nie
odczytuje plaintextu sekretów. Callback delete jest ograniczony do prywatnego
namespace mcp-oauth:<UUID>, bez narzędzi lub polityk ogólnego vaulta.

Zamiar, kolejność i unknown claim są zapisywane przed I/O. Browser/device/IdP,
przypięte członkostwo, rewizja połączenia, numer przebiegu i bieżąca kwalifikacja
referencji są ponownie sprawdzane przed i po delete. Store lock i transakcja
SQL nie obejmują magazynu sekretów. Dwa osobne sloty blocking workera są
utrzymywane do rzeczywistego zakończenia także po utracie połączenia HTTP.
Pętla ma budżet30s na rozpoczynanie kolejnych delete; synchroniczne I/O backendu
nie jest przerywane ani uznawane za zakończone na podstawie upływu czasu.

sweep_complete oznacza potwierdzenia delete w danym przebiegu, nie trwałą
nieobecność danych lub bezpieczne usunięcie kopii zapasowych. Spóźniony writer
może odtworzyć wartość po delete. Kolejny jawny przebieg obejmuje również
wcześniej potwierdzone usunięcia; least-recently-attempted ordering zachowuje
postęp. Referencje nie są ponownie używane. Niepewne oryginalne metadane
pozostają tombstones; migracja47 pozwala wygaszać wyłącznie zweryfikowaną
historię przy osobnym dowodzie opisanym poniżej. Stare expected_cleanup_revision
przegrywa, a utrata odpowiedzi wymaga odczytu statusu, bez automatycznego retry.

unfinished lokalnego delete można jawnie zastąpić nowym przebiegiem: stary
nonce jest odcięty, a już rozpoczęte delete może dotknąć jedynie starego,
unikalnego UUID. Nie może usunąć nowej generacji. Nowe logowanie nie jest
blokowane lokalnym sprzątaniem; zmiana rewizji odcina pozostałe stare operacje.
Błędy delete, brak potwierdzenia i odebranie urządzenia pozostają widoczne
jako unconfirmed/unfinished, z możliwością jawnego ponowienia przez uprawnionego
właściciela. Wspólny audit zachowuje dotychczasowy limit100000, bez blokowania
tej negatywnej operacji na pełnej pojemności.

To sprzątanie potwierdzonych odwołań, nie pełna polityka wszystkich generacji.
Nie usuwa wartości niepotwierdzonych u AS i nie uruchamia automatycznego
sprzątania w tle. Odzyskiwanie pojemności i bounded retencję migracji47 opisano
poniżej; rozstrzyganie legacy unfinished odwołania AS pozostaje osobnym otwartym
wymaganiem. Odbiór OS keychain i rzeczywistego
AS/tailnetu/platform również pozostaje odrębny od kontrolowanego fixture.

## Rozstrzyganie przerwanego odwołania OAuth

Jawny POST /v1/web/workspaces/{workspace}/mcp-connections/{connection}/reconcile-revocation
przyjmuje tylko expected_revision. Właściciel połączenia musi mieć świeże cookie,
uprawnienia urządzenia, IdP i aktywne członkostwo; proof oraz przypięta rewizja
członkostwa są ponownie sprawdzane wewnątrz tej samej immediate transakcji SQL.
Nie potrzeba aktualnej konfiguracji providera. Endpoint nie odczytuje vaulta,
nie wysyła HTTP do AS i nie uruchamia kolejnej próby odwołania.

Produkcja trzyma lease od chwili przed begin_revocation aż do rzeczywistego
powrotu blocking workera. Utrata HTTP waitera i zastąpienie obiektu Service
nie zwalniają tego prawa. Lease ma lokalny gate przypięty do Store, a dla
plikowego SQLite także flock na stałym pliku obok kanonicznego pliku bazy.
Jest to jedna blokada dla odwołań w danej bazie: równoległe odwołanie innego
połączenia jest odrzucane zamiast oczekiwać. Inode locka nie jest usuwany;
CLOEXEC zapobiega odziedziczeniu przez harness. Symlink, hardlink, FIFO,
nieprawidłowy właściciel/prawa locka i hardlink samej bazy powodują odmowę.
Blokada musi być uzyskana także przez reconcile. Żywy worker daje conflict;
śmierć procesu zwalnia kernel lease, bez interpretowania timeoutu/PID jako dowodu.

Po uzyskaniu lease operacja wymaga bieżącej disabled rewizji i dziennika Running.
Zachowuje wszystkie koperty i potwierdzenia, zmieniając wyłącznie stan dziennika
na confirmed_known_tokens, jeśli wszystkie niepuste cele były już potwierdzone,
albo unconfirmed. Brakujące potwierdzenia nie są dopisywane. Nie zmienia rewizji
połączenia i nie reaktywuje go; stary intent traci prawo kolejnego claim/check/ACK.
Odpowiedź jest tym samym bezpiecznym receipt co revoke; powtórzenie zakończonego
reconcile daje conflict. Pełny wspólny audit nie blokuje tej ujemnej operacji.

UI pokazuje „Rozstrzygnij przerwane odwołanie” przy unfinished. Po utracie
odpowiedzi wstrzymuje mutacje do jawnego odświeżenia GET; nie odtwarza POST.
Dopiero po rozstrzygnięciu użytkownik może osobno wybrać nowe logowanie lub
ponowienie odwołania. Lease dowodzi zakończenia lokalnego wykonawcy, nie wyniku
żądania już wysłanego do dostawcy; niepewny efekt po stronie AS pozostaje unknown.

Zgodność wsteczna jest celowo zachowawcza: nowe dzienniki zapisują lease_protected,
a brak tego pola dekoduje się jako false. Receipt unfinished publikuje bezpieczną
flagę reconciliation_supported; UI nie pokazuje akcji dla legacy bez obsługi lease.
Running rozpoczęty przez starszy
binarny plik nie podlega temu endpointowi, bo wolny nowy lock nie dowodzi
zakończenia starego workera, który go nigdy nie trzymał. Dla takich rekordów
służy osobna operatorowa ścieżka migracji po zadeklarowanym wyłączeniu starej instancji;
nie traktujemy ich jako automatycznie odzyskanych. Pliki in-memory mają wyłącznie
lokalny gate, ponieważ ich stan nie jest współdzielony z innym procesem.

## Dowód zakończenia zapisującego OAuth

Migracja45 dodaje writer_protected do prywatnej ewidencji referencji; istniejące
rekordy mają false. Nowy rekord jest oznaczony wyłącznie wtedy, gdy Store ma
aktywny lease zapisującego. Nie jest to potwierdzenie usunięcia ani zgoda na GC:
kończący retencję musi osobno uzyskać ten sam lease i sprawdzić bieżące generacje.
Lease wyklucza lokalnego workera, ale sam nie potwierdza zakończenia operacji
przekazanej wcześniej backendowi sekretów. Retencja potrzebuje także trwałego
potwierdzenia zakończenia zapisu, a jego brak musi pozostać unknown.

Callback trzyma lease przed wymianą code i przez install/prepare/write/commit.
Przeglądarkowy katalog i runtime trzymają go tylko podczas
access_token_or_refresh, zwalniając przed MCP discovery/catalog/tools i przed
oczekiwaniem na zgody lub formularze. Guard zawiera osobny procesowy gate oraz
flock na stałym pliku .mcp-oauth-writer.lock obok kanonicznego SQLite. Jego zasady
inode/owner/mode/CLOEXEC i brak unlink są wspólne z lease odwołania. Dwie
instancje Store lub Service nie mogą ominąć żywego zapisującego. Krótkie konflikty admission mogą czekać do5s bez mutexu Store i ze świeżą
authority; runtime dodatkowo zachowuje oryginalny deadline/lifetime. Po budżecie
następuje odmowa. Nie jest to ponowienie wymiany/refresh ani oczekiwanie bez limitu.

Lease pozostaje w rzeczywistym blocking workerze po anulowaniu HTTP waitera.
Nie trzyma mutexu Store przez keyring lub sieć. Odłączenie i odwołanie tokenów
korzystają z osobnej authority i mogą nadal zwiększyć rewizję; późniejszy commit
starego zapisującego przegrywa. Powstająca wcześniej referencja i jej metadane
pozostają zachowane także po przegranym commit. macOS/Linux mają wspólny Unix
mechanizm; faktyczny odbiór native macOS pozostaje osobny.

Sam writer lease jest przygotowaniem do ograniczonej retencji. Migracja47
opisana poniżej realizuje następny krok; przed nią cleanup zachowywał wszystkie
tombstones i nie zwalniał limitu256. Wymagania tego kroku obejmują oddzielenie
aktywne referencje od posprzątanej historii, zachować potwierdzenia AS niezależnie
od ostatniego bounded journalu i usuwać najstarsze metadane tylko przy dowodzie
writer_protected oraz wyłączności zapisów. Starszych rekordów false nie wolno
uznać za bezpiecznie wygasłe po samym upływie czasu lub restarcie Service.

Wymagania retencji realizowane w migracji47 obejmują
odzyskiwanie limitu aktywnych referencji zamiast podnoszenia stałej256:

1. Trwałe potwierdzenie AS przy dokładnej referencji, oryginalnej rewizji,
   bindingu, generacji i SHA, niezależne od ostatniego bounded revocation journalu.
   Posprzątana historia nie może ponownie trafiać do AS po nowym logowaniu.
2. Oddzielna aktywna pojemność i ograniczona historia tombstones. Cleanup musi
   trzymać writer lease przez cały delete/ACK i odróżniać potwierdzenie usunięcia
   przy braku zapisującego od samego ACK bez tego dowodu.
3. Ograniczone partie sprzątania i trwały sprawiedliwy postęp, także po restarcie
   oraz po dopływie nowych referencji. Stare potwierdzenia nie mogą zniknąć po
   nadpisaniu ostatniego receipt lub zmianie rejestracji providera.
4. Wygaszanie najstarszej bezpiecznej historii dopiero po writer_protected,
   trwałym potwierdzeniu zakończenia zapisu backendu sekretów, wyłączności
   zapisów, potwierdzonym delete i ochronie bieżącej/staged generacji.
   Sam wiek rekordu, restart lub spadek licznika nie wystarczają. Legacy bez
   dowodu i niepotwierdzone AS wymagają osobnej jawnej polityki, bez domyślnego
   zapominania tokenów, które mogłyby nadal być ważne u dostawcy.
5. Odbiór przekroczenia256 zapisów bez trwałej blokady, zachowania ograniczeń
   całej historii, późnego zapisu, migracji/collision oraz aktualnej generacji.
   UI musi rozróżniać odzyskaną lokalną pojemność i potwierdzenie odwołania u AS.

## Trwały wynik pojedynczego zapisu sekretu

Migracja46 dodaje prywatne write_operation_id, write_claimed oraz
write_acknowledged do każdej referencji. Istniejące rekordy otrzymują NULL/0/0;
nie wnioskujemy wyniku ze stanu active, restartu, wieku ani odczytu keyringu.
Nowa rezerwacja zapisuje hostowy nonce operacji. Żaden z tych atrybutów nie jest
zwracany browserowi/runnerowi ani przyjmowany przez REST/MCP.

Publikacja w bibliotece ma kolejność prepare_write → claim_write → prywatny set →
acknowledge_write → commit. Prepare wiąże dokładną kopertę z SHA; claim jest
jednorazowy i zapisany przed I/O. Powtórny claim lub prepare po claim nie mogą
ponownie wysłać set. Błąd set, także po faktycznym efekcie, pozostawia claimed
bez acknowledged. Błąd zapisu receipt również blokuje commit; nie ma retry I/O
ani rekonstruowania potwierdzenia przy starcie. Callback i refresh używają tej
samej ścieżki; adapter Mutex nie trzyma blokady przez callback prywatnego I/O.

Ack jest wyłącznie ujemną ewidencją zakończonego wywołania set. Dokładny nonce,
referencja, właściciel/workspace, oryginalna rewizja, binding, generacja i SHA
muszą odpowiadać zapisanej rezerwacji. Dlatego może zapisać wynik po disconnect,
cofnięciu urządzenia lub wygaśnięciu runtime; nie wymaga już pozytywnej authority
tego starego zamiaru. Browser adapter nadal przypina pierwotnego użytkownika,
a runtime dokładny bound context. To zamknięta ścieżka hosta, nie endpoint do
potwierdzania cudzych zapisów. Nie zmienia rewizji połączenia, statusu, aktywnej
generacji, grantów ani tokenów i nie zwiększa wspólnego audytu.

Chroniona referencja wymaga writer lease przy claim, ack oraz commit. Commit
ponownie wymaga świeżej dodatniej authority i własnej bieżącej generacji, więc
Ack po revoke nie reaktywuje połączenia. Legacy bez writer_protected zachowuje
poprzednią zgodność niskopoziomowego journalu, ale nie uzyskuje dowodu wymaganego
do przyszłego pruning. Wszystkie produkcyjne zapisy callback/browser/runtime
są objęte writer lease. Ack potwierdza sukces zakończonego wywołania backendu,
nie cofnięcie zgody AS ani usunięcie lokalnych danych.

Pruning migracji47 opisanej poniżej wymaga równocześnie writer_protected,
acknowledged, wyłączności zapisów,
trwałego potwierdzenia AS, delete ACK oraz ochrony bieżącej/staged generacji.
Brak dowodu któregokolwiek kroku pozostaje unknown, bez domyślnego zapominania.


## Retencja i odzyskiwanie pojemności — migracja47

Bieżący kod rozdziela limit256 referencji oczekujących od historii sprzątania.
Jawne odwołanie tokenów u AS, cleanup i ponowna autoryzacja mogą odzyskać miejsca
na następne zapisy. Sam disconnect, upływ czasu, restart ani delete ACK bez
potwierdzenia AS nie zwalniają tej pojemności. Nie ma automatycznych wywołań AS
ani daemonu sprzątającego aktywne generacje; pełne E5 pozostaje otwarte.
Powyższe opisy limitu wszystkich referencji dotyczą wcześniejszych migracji.

Migracja47 dodaje as_confirmed, retired, delete_verified oraz trwały licznik
cleanup_attempted_revision. Nowe kolumny mają zachowawcze wartości0; migracja
nie rekonstruuje wyników z wieku lub statusu połączenia. Dokładne potwierdzenia
AS są zapisywane z oryginalną rewizją, bindingiem, generacją i SHA w tej samej
transakcji co wynik journalu. Przy rozpoczęciu kolejnej operacji importowane
są również znane potwierdzenia starego journalu. Nadpisanie ostatniego receipt
lub zmiana rejestracji nie wymusza ponownego odwołania znanych tokenów.
Nie odtwarzamy już usuniętych rekordów z historycznego journalu.

Service trzyma rzeczywisty writer lease przez cały cleanup. Bezpieczne wygaszenie
metadanych wymaga AS-confirmed, writer_protected, jednorazowego claim, trwałego
ACK zakończenia set, potwierdzonego delete i wyłączności zapisów. Chronione są
bieżąca i staged referencja połączenia, które nie jest disabled. Bez tych dowodów
rekord pozostaje w historii nierozstrzygniętej, nawet jeśli usunięcie otrzymało ACK.

| Kategoria na połączenie | Limit i zachowanie |
| --- | --- |
| Referencje obciążające pojemność |256; odmowa następnej rezerwacji przy pełnym limicie |
| Zweryfikowana historia |512 po zakończonym sprzątaniu; najstarsze wpisy mogą wygasnąć |
| Historia nierozstrzygnięta |256; brak automatycznego wygaszania |
| Przejściowe zweryfikowane wpisy |do256 podczas niedokończonego przebiegu |
| Wszystkie referencje SQL |maksymalnie1280 |

Delete ACK może przenieść rekord do historii nierozstrzygniętej i odzyskać aktywne
miejsce. Po zapełnieniu jej limitu kolejny niepewny rekord nadal obciąża aktywną
pojemność; sam ACK nie udaje odzyskanego miejsca. Nadal można ponownie sprzątać
zachowane wpisy. Partie mają do256 celów i dotychczasowy30s budżet rozpoczęcia
I/O. Trwałe numery prób zapewniają postęp także po restarcie; nowa rezerwacja
zaczyna od bieżącej rewizji sweep, zamiast stale wypierać stare wpisy priorytetem0.

GET połączenia publikuje wyłącznie bezpieczne liczniki storage: pending_references,
retained_history, uncertain_history, available_slots i cleanup_available.
Receipt cleanup dodaje reclaimed (odzyskane miejsca) i pruned (wygasłe bezpieczne
metadane). UI pokazuje pojemność osobno od odwołania AS i udostępnia sprzątanie
historii także bez ostatniego receipt AS. Nie ujawnia prywatnych referencji,
nonce, bindingów ani tokenów. Saturacja niepewnej historii wymaga jawnego
rozstrzygnięcia; nie jest obsługiwana przez ciche zapomnienie.

Testy tego przyrostu obejmują768 zapisów w trzech cyklach z restartami SQL,
rejestrację nowego bindingu, utratę ostatniego journalu, limity niepewnej historii,
migrację46→47 i rollback kolizji oraz ochronę nowej autoryzacji przy sprzątaniu
starej historii. Wyniki aktualnego odbioru są zapisywane w
[evidence/2026-11-13-mcp-retention/](evidence/2026-11-13-mcp-retention/).
Kontrolowane AS i backend sekretów nie zastępują realnego OAuth/keyringu,
native macOS ani odbioru tailnetu.


Odbiór migracji47: backend851, MCP/HTTP/vault153 i package Clippy,
frontend155, TypeScript, root all-targets Clippy oraz wybrany Chromium/helper1
PASS. [Manifest](evidence/2026-11-13-mcp-retention/verified-source.json) wiąże
bieżące źródła i logi; [szczegóły](progress.md) zachowują również pierwsze
nieudane próby. Aktualny odbiór E5.2 opisuje końcowa macierz; cały cel E0–E10
pozostaje otwarty.


## URL elicitation w runtime

Ten pion dotyczy zatwierdzonych tools/call. MRTR według oficjalnego kontraktu
jest dopuszczony wyłącznie dla tools/call, prompts/get i resources/read;
server/discover i metody list nie mogą zwracać input_required. SDK obsługuje
wszystkie trzy dozwolone operacje, a pełne UI resources/prompts jest osobnym
zakresem aplikacji. Wcześniejsze określenie interaktywnego discovery/list jako
nieodebranej ścieżki nie oznacza wymagania protokołu — taki MRTR jest zabroniony. Runtime jawnie reklamuje
teraz form i url; zgodność ze starszym transportem nadal
korzysta z jawnego adaptera, bez udawania obsługi nieznanych możliwości. URL jest
osobnym ChatInputRequest.url w standardowym AHP, bez questions i answers. Hostowy
journal wymaga null constraints dla tego trybu i nie przyjmuje danych odpowiedzi,
nawet pustego answers, przy accept/decline/cancel. Nie wymaga nowej migracji SQL:
tryb jest przypięty do niezmiennego request zapisanego z istniejącym permit/nonce.

SDK, mapowanie hosta i journal odrzucają adresy inne niż HTTPS, brak hosta,
embedded credentials, surowe białe/kontrolne znaki i backslash; limit adresu4096B,
wiadomości8192B. UI ponownie sprawdza adres przed utworzeniem linku, pokazuje
pełną oryginalną wartość jako tekst oraz domenę z portem w zapisie ASCII.
Link używa absolutnego URL wyliczonego przez parser bez adresu bazowego, aby
relatywna interpretacja specjalnego schematu w HTML nie zmieniła hosta. Punycode ma
osobne objaśnienie. Wiadomość providera pozostaje zwykłym tekstem, bez aktywnych
linków. Nie ma preview, prefetch, pobrania metadanych ani proxy z bearerem MCP.

Dopiero uprawniony użytkownik na żywym połączeniu może kliknąć link do nowej
karty. Link ma noopener/noreferrer i no-referrer. Otwarcie nie wysyła decyzji;
osobny przycisk Kontynuuj wysyła wyłącznie action accept, a Odmów/Anuluj
odpowiednio decline/cancel. Użytkownik może kontynuować ponownie, jeśli serwer
zwróci kolejną rundę; akceptacja nie jest dowodem zakończenia interakcji u dostawcy.
Decyzja steruje kontynuacją MCP; stan operacji wykonanej na zewnętrznej stronie
ocenia dostawca. Dunlin nie odczytuje zawartości zewnętrznej strony ani wpisywanych tam danych.

Oryginalny klient, Pending i requestState pozostają wyłącznie na hoście, z tym
samym deadline, limitem8 rund/16 żądań i kontrolami authority/generacji/tokena.
Decyzja jest przypięta do bieżącego permit oraz urządzenia odpowiadającego;
zmiana dostępu przed kolejnym HTTP odcina kontynuację. Journal deduplikuje decyzję
jednorazowo; restart nie odtwarza Pending ani nie ponawia żądania MCP. URL nie
zmienia OAuth client→MCP i nie jest kanałem do przekazywania tokenów dostawcy.

Podstawa kontraktu: [MCP URL elicitation i Safe URL Handling](https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation).
Odbiór: backend853, MCP/HTTP/vault153 i ich Clippy, frontend157, TypeScript,
root all-targets Clippy i23 wybrane zdalne acceptance z Chromium PASS.
[Manifest](evidence/2026-11-13-mcp-url-input/verified-source.json) wiąże źródła,
logi, PNG oraz odtwarzalny dowód trzech plików frontend zmienionych po snapshotcie
backendu. Kod Rust i backend fixtures są identyczne; aktualny bundle ma osobny
frontend/TypeScript/runtime odbiór. Szczegóły są w [progress.md](progress.md).
Kontrolowany dostawca/strona HTTPS nie zastępują rzeczywistych kont i platform.
Aktualny odbiór E5.2 opisuje macierz poniżej; E5.3 i cały projekt pozostają otwarte.


## Operatorowa migracja legacy unfinished

Starsze Running bez lease_protected nie może używać browser reconcile. Wolny
nowy flock nie mówi, czy stary worker nadal działa. Osobne lokalne polecenie
mcp-oauth-legacy wymaga operatora będącego właścicielem pliku bazy oraz jawnej,
aktywnej tożsamości owner/admin właściwego workspace. Nie jest endpointem HTTP,
narzędziem MCP ani funkcją runnera. Nie wykonuje sieci ani I/O magazynu sekretów.

Operator najpierw zatrzymuje wszystkie stare instancje relay i wyłącza ich
samoczynne ponowne uruchamianie, potwierdzając to w swoim środowisku usług.
Program nie potrafi technicznie udowodnić zakończenia workera sprzed lease.
Wymagany flagą fakt jest deklaracją operatora i tak pozostaje zapisany w receipt;
nie jest zamieniany na dowód jądra, AS ani zakończenia zapisu sekretu.

Przegląd jest tylko do odczytu, na jawnej istniejącej bazie bieżącego schematu:

```sh
dunlin-relay mcp-oauth-legacy show --db /ABS/relay.db --workspace-id WORKSPACE --connection-id CONNECTION --actor-user-id OPERATOR
```

Wynik zawiera rewizję, bezpieczne liczniki i review_sha256. Digest wiąże
kanoniczną ścieżkę, inode/dev/UID, workspace/połączenie/właściciela, aktora,
rewizję jego członkostwa oraz dokładny prywatny ledger. Nie ujawnia referencji,
bindingów, nonce ani zawartości poświadczeń. Po sprawdzeniu rekordu i zatrzymaniu
starych instancji operator jawnie przekazuje te wartości:

```sh
dunlin-relay mcp-oauth-legacy recover --db /ABS/relay.db --workspace-id WORKSPACE --connection-id CONNECTION --actor-user-id OPERATOR --expected-revision REVISION --review-sha256 REVIEW_SHA256 --change-id OPS-123 --ack-all-legacy-relays-stopped
```

Nie ma domyślnej bazy, flagi force, wyboru ostatniego zamiaru ani automatycznego
przepisywania schematu. Dowiązanie końcowe, hardlink, FIFO, inny UID i współdzielone
prawo zapisu powodują odmowę; nie naprawiamy plików automatycznie. Brakujący plik
nie jest tworzony. Starszy schemat wymaga standardowej migracji, a nowszy zgodnej wersji programu.

Recover uzyskuje oba obecne leases (AS i writer) i immediate transakcję SQL.
Ponownie sprawdza członkostwo, snapshot i rewizję. Dotyczy wyłącznie disabled,
Running, niechronionego dziennika bez wcześniejszej migracji. Zwiększa rewizję,
zmienia nonce i zamyka stan na confirmed_known_tokens tylko dla już kompletu
potwierdzeń; pozostałe wyniki są unconfirmed. Zachowuje dokładne targets i wszystkie
referencje/proofs, pozostawia lease_protected=false i nie dodaje write ACK ani AS ACK.
Nie aktywuje połączenia ani usuniętego członkostwa. Stary zamiar traci claim/check/
ACK/finish; nowe logowanie lub odwołanie wymaga osobnego, jawnego żądania użytkownika.

Prywatny operator_recovery ma UID, deklarowaną tożsamość aktora, change ID,
review digest i stare rewizje. Jest przenoszony do następnych dzienników, bez
nieograniczonej listy historii. Team audit ma oznaczenie operacji lokalnego
operatora. Przy pełnym limicie audytu bounded prywatny receipt pozostaje trwały;
błąd SQL cofa całą migrację. Dane operatora nie są publikowane w browser GET.
Po operacji należy jawnie odświeżyć status; inny działający proces serwera nie
otrzymuje lokalnego broadcastu narzędzia CLI. Niepewne zapisy legacy i retencja
bez dowodu zakończenia set pozostają unknown; ta operacja ich nie rozstrzyga.

Backend860, MCP/HTTP/vault153, root Clippy i production CLI acceptance1 PASS
na tym samym źródle. [Manifest](evidence/2026-11-13-mcp-legacy-recovery/verified-source.json)
zawiera logi, snapshoty i pierwszą nieudaną próbę testowego stale review. Odbiór
dotyczy syntetycznych baz; nie migrowano rzeczywistych danych, kont ani usług.
Niepewne write outcomes nadal pozostają unknown. Pełny E0–E10 pozostaje aktywny.


## Macierz wymagań E5.2 — odbiór

Punktem odniesienia jest dokładne zadanie E5.2 w README: transport2026 i adapter2025,
discovery/metadata, MRTR, subscriptions, OAuth i schematy z limitami. Profile,
capabilities i limity są opisane w sekcjach powyżej.

| Wymóg E5.2 | Implementacja i dowód |
| --- | --- |
| MCP2026 + adapter2025 | Modern-first HTTP JSON/SSE, per-request metadata, poprawne session semantics2025 i brak downgrade po błędzie auth/rozpoznanym modern error; pakiet MCP i backend |
| Discovery/metadata | Przypięta wersja/capabilities, atomowy katalog, paginacja, projekcja nagłówków i prywatny bounded cache; pakiet MCP i REST/broker |
| MRTR | SDK tools/call, prompts/get, resources/read; opaque jednorazowy Pending, state i dokładna operacja; form/URL z allowlisted runtime tools i AHP,23 zdalne acceptance z Chromium |
| Subscriptions | Jawny POST/listen, ACK/filter/subscriptionId, TTL/deadline, bounded SSE i scoped invalidation bez reconnect/replay; pakiet MCP |
| OAuth | PKCE/code/CIMD, przypięte authority/resource, immutable generations, claim/write ACK, refresh/revoke/cleanup/retencja; trwałe SQL i kontrolowany OAuth/runtime/browser |
| Legacy unfinished | Operatorowa ścieżka show/recover po jawnej deklaracji shutdown; backend860 i produkcyjny CLI1 PASS |
| Schematy i limity | Ograniczony profil JSON Schema, local refs bez fetch, budżety grafu/regex/payload, walidacja pin/arguments/output; pakiet MCP, backend i formularze |

Podstawa zakresu MRTR: [Supported Requests](https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/mrtr#supported-requests).
E5.2 nie wymaga interaktywnego discovery/list. Interfejs pełnych zasobów/prompts,
raport capabilities Buildera, ogólne scoped secrets i A2A pozostają w odpowiednich
punktach E3.7/E5.3/E5.4/E5.5. Testy realnych kont/tailnetu/keyringu i native platform
pozostają E10. Końcowe CLI runtime i root all-targets Clippy na źródle migracji
PASS. Na podstawie tej macierzy i manifestu oznaczono E5.2 jako wykonane:
formalnie21/68=30,9%. Cały projekt E0–E10 pozostaje aktywny i nieukończony.
