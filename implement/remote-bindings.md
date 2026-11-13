# Zdalne ProjectBinding i authority workera

Status: E4.2 i E4.4 w realizacji. Zaimplementowano przekrój odczytu metadata
przez WSS oraz konfigurację bindingów kontrolera; bieżąca weryfikacja trwa.
Nie jest to wykonanie zdalnego zadania. Wyniki rzeczywistych testów
notujemy oddzielnie w [progress.md](progress.md).

## Dwie niezależne decyzje

Logiczny projekt zespołu nie jest ścieżką systemową. Lokalny binding istniejącego
relay zachowuje swój dotychczasowy native grant i pin checkoutu. Zdalny binding
nie może reinterpretować jego revision ani automatycznie przełączać się na
lokalne wykonanie po błędzie workera.

Operator workera jawnie wybiera lokalny checkout i zatwierdza export. Kontroler
wiąże projekt z nieprzezroczystym ID tego exporta. Kontroler ani browser nie
wybierają ścieżki katalogu, dodatkowego sandbox root lub polecenia systemowego.
Raport capabilities, zatwierdzenie hosta i sam WSS nie nadają dostępu do plików.

Bieżący interfejs lokalnego operatora: [worker-exports.md](worker-exports.md).

## Trwały export po stronie workera

Tożsamość katalogu exportów jest przypięta do stabilnego controller node ID,
workspace ID, host ID i enrollment epoch. Controller boot ID nie jest trwałą
tożsamością exporta: restart ma wymagać nowego kanału i nowego execution fence,
bez automatycznego nadawania ani cofania lokalnych grantów. Reenrollment hosta
nie przejmuje starego scope; wymaga ponownej decyzji operatora.

Prywatny rekord workera przechowuje canonical checkout root, tożsamość katalogu
i metadanych Git oraz przypięte ustawienia. Descriptor przekazywany kontrolerowi
zawiera ID, rodzaj checkoutu, revision i digests, bez fizycznych ścieżek.
Zmiana tożsamości katalogu checkoutu, katalogów metadanych Git lub zatwierdzonej konfiguracji oznacza
konieczność jawnego ponownego związania. Zapisany settings digest jest jawną
zgodą operatora na konkretny hash, nie odczytem rzeczywiście zastosowanego
pliku konfiguracji. Przed wykonaniem worker musi porównać hash ograniczonej
konfiguracji, którą faktycznie zastosuje. Pin checkoutu nie zastępuje sandboxa
ani egzekwowania polityki Git hooks/filters/config includes i symlinków.

Lokalne granty wskazują dokładny principal kontrolera, z rozróżnieniem człowieka
i service principal. Nie powstaje automatyczne mapowanie na konto OS ani root.
Grant i export mają osobne CAS revisions oraz zachowane tombstones. Odwołanie
i późniejszy regrant nie ożywiają starych captured stamps. Cofnięcie uprawnień
nie może wymagać dostępnego checkoutu ani miejsca w zwykłym audycie.

Zwykłe repo z .git wewnątrz root i katalog bez Git są podstawowymi przypadkami.
Linked worktree wymaga osobnego adaptera oraz jawnego zatwierdzenia git-dir
i common-dir jako dodatkowych roots. Samo odkrycie .git file/commondir nie jest
zgodą na rozszerzenie sandboxa. Pełne E4.2 pozostaje otwarte do wdrożenia
i weryfikacji wymaganych adapterów i zdalnego powiązania.

## Zdalny binding po stronie kontrolera

Docelowy rekord przypina workspace/project, host oraz jego enrollment/credential
epoch, export ID/revision i jego tożsamości, własną revision i active tombstone.
Zapis wymaga świeżych ManageHosts i ManageProject, dokładnego scope i CAS.
Descriptor musi pochodzić z bieżącego uwierzytelnionego kanału workera; dowolny
JSON od browsera nie jest dowodem istnienia exporta.

Wybór celu jest jawnym BindingRef z ID i revision. Stare lokalne API i żądania
bez zdalnego ref zachowują dotychczasowe znaczenie. Brak hosta, stale export,
niezgodna revision lub odmowa uprawnień nie oznaczają zgody na inny host.
Idempotency receipt dla tworzenia sesji zachowuje pierwotny cel i profil.

## Następny przekrój transportu

Pierwszy zdalny odczyt ma rozwiązywać konkretne export ID przez bieżący, już
potwierdzony WSS. DescribeExport/ExportDescribed to ograniczone
komunikaty metadata, z request ID, channel ID i lease epoch, dokładnym scope,
export revision i principalem wyprowadzonym ze świeżej authority kontrolera.
Nie wysyłamy fizycznej ścieżki, polecenia lub body konfiguracji. Brak katalogu
stanu po stronie workera daje jawną odmowę, bez automatycznego odkrywania plików.
Nowe komunikaty wymagają dokładnego subprotocol dunlin.worker.v2 i Hello2.
V1 pozostaje kanałem diagnostycznym; nie rejestruje metadata sender i nie
otrzymuje DescribeExport. HostCapabilities.protocol_version pozostaje1 jako
osobny kontrakt rejestru. Nie ma cichego downgrade do innego transportu.

Pending requests muszą mieć bounded kolejkę, deadline i korelację. Zamknięcie
lub zastąpienie kanału kończy oczekiwania; spóźniona odpowiedź nie może utworzyć
bindingu. Heartbeat i niezależne guards nie mogą czekać na odczyt katalogu lub
FS. Walidacja FS odbywa się poza blokadą SQLite, w ograniczonej liczbie jobs,
z permit trzymanym do rzeczywistego końca pracy również po anulowaniu odbiorcy.
Nie deklarujemy twardego anulowania dowolnego kernel I/O.

Kontroler ponownie sprawdza actor/project/host i CAS po odpowiedzi. Zapis
bindingu przechowuje zaobserwowany stamp i czas. Nie istnieje atomowa transakcja
obejmująca oba komputery: lokalny operator może odwołać grant zaraz po wysłaniu
descriptora. Dlatego przyszłe admission wykonania musi ponownie odczytać bieżącą
authority na workerze; zapis bindingu nie jest obietnicą nadal aktywnego grantu.
Kontroler zapisuje do16 bindingów na projekt, z CAS i tombstones oraz
przypiętym controller node ID. Panel konfiguracji jest w projektach zespołu.
List/create/save/revoke wymagają świeżych ManageHosts i ManageProject. Principal
pochodzi z bieżącego użytkownika, nie body żądania. Sesja/cookie i authority są
ponownie czytane w końcowej transakcji SQL przed zapisem; nie trzymamy tej
transakcji podczas WSS. Cofnięcie bindingu nie wymaga działającego kanału.

Wdrożono jawny typed BindingRef/resolver i kontrole targetu przy tworzeniu
sesji; backend/Chromium, runtime i końcowy Clippy PASS (szczegóły i zakres
w progress.md). Do pełnego E4.2 pozostaje integracja lokalnego katalogu
linked-metadata approval/grant ze wszystkimi konsumentami native Git i sandbox
roots. Remote adapter nie zastępuje brakującego local adaptera. Nowy fixture
sprawdza linked worktree przez reopen→WSS→binding→resolver oraz odmowę po
podmianie katalogu objects; jego aktualne wyniki są w progress.md.

## Jawny cel i dostęp bieżącego użytkownika

BindingRef ma wyłącznie id i dodatnią bezpieczną revision; nie ma ścieżki
ani automatycznie wybieranego hosta. POST
`/v1/web/workspaces/{workspace}/projects/{project}/remote-bindings/resolve`
przyjmuje body `{ "binding": { "id": "rbd-ID", "revision": 1 } }`.
Wymaga aktualnych ViewProject i CreateSession. Użytkownik nie potrzebuje
ManageHosts/ManageProject do rozwiązania znanego ref, ale musi mieć własny
worker Read grant. Grant administratora zapisującego binding nie jest używany.

Resolver czyta dokładny ref i aktualne host/config piny w pierwszym SQL cut,
wysyła DescribeExport z Human ID faktycznego wywołującego, po odpowiedzi ponownie
sprawdza cookie/device/OIDC, logical authority revisions, binding, controller
node, host epochs, konfigurację exporta i lease. Nie wybiera innego bindingu
przy błędzie. Finalny opaque ResolvedRemoteTarget nie jest serializowalnym
pozwoleniem wykonania; HTTP zwraca wyłącznie observed summary dla tego użytkownika,
required_worker_permission:read i execution_ready:false. Nie zmienia bazy,
nie rezerwuje capacity i nie publikuje nowego catalog revision.

UI ma jawne sprawdzenie dostępu dla wybranego ref. Zmiana wyboru, baseline,
mutation lub authority invalidation usuwa wynik; nie odpytuje workera cyklicznie.
Odmowa worker Read usuwa wynik check, nie zmienia historycznych metadanych
konfiguracji, których odczyt ma osobne uprawnienia.

CreateRequest obsługuje opcjonalny ProjectTargetRef: `local` z revision albo
`remote` z BindingRef. Pominięty/null target zachowuje stare lokalne znaczenie
i dokładny dotychczasowy fingerprint idempotency receipt. Jawny Local jest
osobnym zamiarem: przypięta revision musi nadal odpowiadać lokalnemu bindingowi
przed każdym lookup receipt i admission. Stary ref daje konflikt409.
Przejście pominięty↔jawny Local z tym samym kluczem daje konflikt body.

Do czasu wdrożenia E4.4 jawny Remote daje stabilne501
remote_execution_not_supported przed lokalnym resolverem/receipt/capacity
oraz zapisami. Nie odtwarza starego lokalnego receipt i nie uruchamia procesu
na zastępczym hoście. Jest to jawna brakująca capability, nie wykonanie zdalne.

## Otwarty adapter local standalone

Linked worktree w standalone wymaga własnego typed approval dodatkowych
katalogów Git przez operatora i osobnego grantu użytkownika, przypiętego
do approval revision oraz native grant revision. Nie wystarczy dodać
automatycznie commondir do istniejących ACP additionalWorkspaceRoots: te
roots są używane również do zwykłego workspace FS i rozszerzyłyby stare granty.
Root metadanych nie może automatycznie stać się workspace folder, attachment
root, vault scope ani dostępem browser FS.

Approval musi istnieć na native project niezależnie od team_local_bindings,
żeby legacy Git/ACP/PTY/jobs nie omijały odwołania. Do przypięcia i ponownej
walidacji wymagają go runtime admission/actor/ACP guardian, native I/O i
prepared Git operations, source/derived jobs oraz PTY authority. Sandbox
process roots potrzebują osobnego parametru metadata roots. Obecny projektowy
PTY nie staje się przez tę deklarację sandboxem; brak nowego dowodu confinement.

Wspólny czysty capture identity z adapterem worker może być ponownie użyty,
ale WorkerStore grants nie są native grants. Nowe odwołanie ma być możliwe
bez FS, również przy pełnym zwykłym audycie i terminalnej revision. Nie wolno
kopiować starszego zachowania local-binding revoke wymagającego audit capacity.
Trwały katalog zgód i grantów oraz operator-local CLI są wdrażane poniżej.
Integracja wszystkich konsumentów nadal pozostaje otwarta.

### Lokalny katalog zgód i grantów

Wersja bazy33 dodaje native_metadata_approvals/grants/audit/revocations oraz
osobny terminalny receipt zwykłego local bindingu. Nowy katalog jest niezależny
od WorkerStore i team_local_bindings. Nie odnawia dawnych native grants.
Pełny backend, actual binary CLI/bwrap i Clippy tego przekroju PASS (zakres
i logi w progress.md). Nie jest to jeszcze aktywacja lokalnego linked execution
ani zabezpieczenie wszystkich historycznych ścieżek Git.

Operator używa `dunlin-relay local-metadata` z jawną istniejącą absolutną bazą
`--db`, `--project-id` i aktualną rewizją CAS. `approve` wymaga
`--expected-revision none`, `--git-dir` oraz `--common-dir`; `rebind` wymaga
aktualnej dodatniej rewizji. Capture odbywa się poza transakcją, a commit
ponownie sprawdza native path i settings digest. Zapis nie nadaje grantu.

`grant` wymaga `--user-id`, `--expected-revision none|N`,
`--expected-approval-revision N`, `--expected-native-grant-revision N` i
`--permissions read` albo `read,write`. Zapis wspólnych metadanych wymaga też
jawnego `--ack-repository-wide-metadata-write`. Approval, native grant i grant
metadanych mają niezależne epochs. Rebind/regrant nie ożywia starego stempla.

`check --user-id ID --permission read|write` przechodzi przez SQL candidate,
walidację FS poza Store i świeży SQL cut; zwraca path-free obserwację z
`execution_ready:false`. Sprawdza także, czy oryginalna ścieżka projektu nadal
prowadzi do zatwierdzonego katalogu, zamiast wyłącznie odwiedzać dawny target
symlinku. `show`/`list` pokazują utrwaloną konfigurację bez FS i bez
przedstawiania jej jako aktualnego dostępu.

`revoke --expected-revision N` i `ungrant --user-id ID --expected-revision N`
nie wymagają checkoutu ani nowego native grantu. Obowiązkowy ostatni receipt
pozostaje niezależny od pojemności zwykłego audytu. Terminalny epoch jest
zarezerwowany na odmowę; importowane aktywne MAX można cofnąć. Uszkodzony
capture/permissions JSON nie może zablokować cofnięcia. Limity lifetime:
1024 approvals,64 granty na projekt,4096 wpisów zwykłego audytu; tombstones nie
znikają przy ponownym przyznaniu. Nie ma zdalnego endpointu zatwierdzającego
ścieżki ani automatycznej detekcji nadającej uprawnienia.

Metadane nie trafiają do ACP additionalDirectories, browser FS, attachments
ani vault. Otrzymuje je osobny typed adapter sandboxa. Lokalny linked binding
ma teraz jawny pin approval; integracja pozostałych konsumentów jest opisana
poniżej i przechodzi weryfikację wykonawczą.

### Integracja Git i ACP

NativeGitIo/Proof przypina native epoch, ustawienia, kształt/tożsamość checkoutu
oraz opcjonalną lokalną zgodę i własny grant metadanych. API historii i operacji
Git, prepared plans, porównania i procesy checks używają tej authority;
plain NativeIo zachowuje wyłącznie workspace FS. Legacy ACP otrzymuje oddzielne
typed sandbox metadata roots oraz guardian. Backend405, workspace189, rzeczywisty
Linux bwrap/guardian, binary restart i Clippy PASS; zakres/logi w progress.md.
Dowód kernela dotyczy ACP, nie nowego confinement komend checks ani macOS.
Kolejny przekrój jobs/PTY/interactive/local linked binding jest zaimplementowany;
pełny odbiór wykonawczy tego przekroju pozostaje w toku (progress.md).

Nowy projekt pochodnego worktree ma własny native grant, lecz nie dziedziczy
approval/grantu metadanych źródła. Compare/create sygnalizuje
`git_access:requires_operator_approval`. Odczyt Git wymaga oddzielnej jawnej
zgody i własnego grantu. Nie należy traktować pomyślnego utworzenia katalogu
jako gotowości Git ani wykonania; ewentualny przyszły adapter pochodnej authority
musi przypinać także źródłowe epochs i ich odwołanie.

### Kontrakt integracji lokalnego adaptera

Bieżący kod realizuje następujący model; wyniki testów i otwarte bramki
są zapisane osobno w progress.md:

- Approval jest wersjonowanym rekordem native project z prywatnym capture
  checkout/git-dir/common-dir/objects i settings digest. Grant użytkownika
  przypina approval revision, aktualny native grant revision i własną revision.
  Rebind approval nie odnawia grantów. Revoke zachowuje tombstone i niezależny
  receipt; nie wymaga FS ani miejsca w zwykłym audycie.
- Opaque NativeCheckoutStamp musi przypinać również zwykły checkout. Zmiana
  zwykłego `.git` w plik worktree albo dodanie `commondir` nie może rozszerzyć
  authority już działającego procesu. SQL candidate, walidacja FS poza Store
  i końcowy świeży SQL cut są osobnymi krokami.
- ACP obecnie łączy process sandbox roots z Tracker.allowed_roots,
  additionalDirectories oraz mapowaniem projektów/vault. Metadata roots
  wymagają oddzielnego parametru wyłącznie do konstruktora sandboxa; nie mogą
  trafiać do tej istniejącej listy ani ProjectSettings.additionalWorkspaceRoots.
- Ordinary admission fingerprint pozostaje zgodny z dawnymi receipt.
  Nowy linked intent przypina wszystkie approval/grant epochs; historyczny
  receipt nie produkuje nowej authority ani nowego aktora. Job bez wymaganego
  utrwalonego stempla jest odrzucany, zamiast przejmować nowe approval.
- Zapis common-dir obejmuje wspólne refs/objects innych worktree tego samego
  repozytorium. Zgoda/grant musi jawnie obejmować ten zakres; sam worktree nie
  zapewnia izolacji zapisu Git. Wyjątki sandboxa nie mogą usuwać istniejących
  odmów dostępu do innych projektów, keyringu ani danych relaya.
- Derived job przypina osobno źródłowy checkout na czas clone i wynikowy
  checkout/grant. Obecny clone --no-local/--no-hardlinks tworzy zwykły `.git`;
  ten fakt nie usuwa wymogu autoryzacji źródła.

Migracja34 zachowuje stare job payloads i ordinary bindingi. Job bez utrwalonego
source stamp wymaga nowej zgody. Local binding `metadata_approval_revision`
jest opcjonalny tylko dla zwykłego checkoutu; linked wymaga jawnej bieżącej
wersji i własnego grantu Read przy zapisie/odczycie konfiguracji. Save nie
nadaje grantów ani nie uruchamia agenta. Wykonanie wymaga własnego Write.
UI zachowuje zapisany binding po utracie dostępu, lecz oznacza konfigurację
jako nieaktualną dla bieżącego użytkownika.

Project PTY pozostaje host shell. Interactive linked metadata wymagają
włączonego sandboxa; nie ma fallbacku do procesu bez sandboxa. Source job
korzysta z Read, a jego zwykły clone otrzymuje niezależny Write stamp;
nie dziedziczy linked approval źródła. Status delivery Linear również
ponownie sprawdza oba stampy i aktualną authority urządzenia.

Obecność tych ścieżek w kodzie nie oznacza zamknięcia E4.2 ani odbioru
kernel macOS/operatora. Pełne wyniki nowej suite nadal oczekują.

## Wykonanie wymaga kolejnej warstwy

Proponowany, jeszcze niezaimplementowany pion inbox/outbox, execution lease i
guardiana opisano w [worker-execution-plan.md](worker-execution-plan.md).

Export, lokalny grant i binding są warunkami koniecznymi, lecz ich descriptor
nie uruchamia procesu. Przyjęcie konkretnej próby musi dodatkowo przypinać:

- aktualną authority użytkownika/urządzenia/OIDC i logicznego projektu albo
  jawnie ograniczoną service authority;
- host/enrollment/credential, wersję exporta i lokalnego grantu;
- bieżący zweryfikowany kanał, oba boot IDs i jego lease epoch;
- trwały attempt ID, wersję agenta/polityki, ograniczenia sandboxa i budżet.

Worker potrzebuje trwałego inbox/outbox, ograniczonego admission i niezależnego
monotonicznego execution lease z guardianem rzeczywistej grupy procesów. Heartbeat
transportu nie zastępuje kontroli wykonania. Cofnięcie authority, utrata kanału,
zmiana checkoutu i restart wymagają odmowy nowej pracy oraz zatrzymania lub
uzgodnienia bieżącej próby zgodnie z jej stanem. Niepewnego wyniku nie ponawiamy
automatycznie po utracie ACK lub restarcie.

## Dowody potrzebne do odbioru

Testy muszą obejmować reopen, concurrent CAS, revoke/regrant ABA, granice
liczników, podmianę katalogu/Git roots i ustawień, scope między kontrolerami,
workspace i enrollmentami, redakcję ścieżek oraz odwołanie przy niedostępnym FS.
Zdalny przekrój wymaga rzeczywistego prywatnego TLS/WSS fixture obu ról,
przypięcia descriptorów do kanału i odmowy stale target bez lokalnego fallbacku.
Wykonanie wymaga dodatkowo rzeczywistych procesów/sandboxa i recovery prób.
Akceptacja dwóch realnych maszyn i polityki tailnetu pozostaje w E4.7/E10;
nie zastępujemy jej syntetycznym LocalAPI ani tą dokumentacją.
