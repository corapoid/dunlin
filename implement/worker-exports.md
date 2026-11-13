# Lokalny katalog exportów workera

Status: implementowany fundament E4.2; weryfikacja bieżącego kodu jest zapisywana
w [progress.md](progress.md). Nie jest to jeszcze zdalny ProjectBinding,
transport katalogu do kontrolera ani wykonanie zadania. Docelowy przekrój:
[remote-bindings.md](remote-bindings.md).

## Decyzja lokalnego operatora

Komendy `worker export` działają lokalnie, bez credential stdin i bez połączenia
z LocalAPI lub kontrolerem. Używają zwykłego pliku konfiguracji workera opisanego
w [worker-channel.md](worker-channel.md). Stan przypina oba stabilne node IDs,
workspace, host oraz enrollment/credential revisions. Inny scope nie przejmuje
zawartości istniejącej bazy; ponowne zatwierdzenie wymaga jawnej decyzji
operatora i nowego katalogu stanu. Controller boot nie jest trwałą tożsamością
exporta.

Operator wskazuje osobny absolutny `--state-dir`. Na Unix katalog musi należeć
do bieżącego użytkownika i być prywatny; baza exports.sqlite3 jest zwykłym
prywatnym plikiem. Nie jest to istniejąca relay.db i nie importujemy do niej
automatycznie native grantów, kont OS lub kont kontrolera. Lokalny OS user/root
i system plików operatora pozostają granicą zaufania.

Każdy export ma nieprzezroczyste ID, revision, active tombstone, zatwierdzony
checkout i hash ustawień. Grant jest oddzielny, wskazuje dokładny human albo
service principal ID w przypiętym scope. Execute wymaga jawnego Read. Rebind
unieważnia wcześniejsze granty; operator musi ponownie je zatwierdzić.
Revoke i ungrant nie potrzebują dostępnego checkoutu.

## Komendy

W poniższych przykładach `EXPORT_ID`, `USER_ID`, `REVISION` i
`APPROVED_SETTINGS_SHA256` należy zastąpić rzeczywistymi wartościami. Hash jest
64-znakowym lowercase SHA-256 zatwierdzonej polityki ustawień. Obecny katalog
przechowuje zgodę na hash; nie odczytuje ani nie stosuje tej konfiguracji.

```sh
dunlin-relay worker export list --config /operator/worker.toml --state-dir /operator/private-worker-state
dunlin-relay worker export register --config /operator/worker.toml --state-dir /operator/private-worker-state --name Checkout --root /operator/checkout --settings-digest APPROVED_SETTINGS_SHA256
dunlin-relay worker export grant --config /operator/worker.toml --state-dir /operator/private-worker-state --id EXPORT_ID --principal-kind human --principal-id USER_ID --permissions read,execute --expected-revision 0
dunlin-relay worker export check --config /operator/worker.toml --state-dir /operator/private-worker-state --id EXPORT_ID --principal-kind human --principal-id USER_ID --permission execute
dunlin-relay worker export grants --config /operator/worker.toml --state-dir /operator/private-worker-state --id EXPORT_ID
dunlin-relay worker export ungrant --config /operator/worker.toml --state-dir /operator/private-worker-state --id EXPORT_ID --principal-kind human --principal-id USER_ID --expected-revision REVISION
dunlin-relay worker export revoke --config /operator/worker.toml --state-dir /operator/private-worker-state --id EXPORT_ID --expected-revision REVISION
```

Revision0 oznacza brak wcześniejszego grantu wyłącznie przy pierwszym grant.
Późniejsze zmiany wymagają dokładnego odczytanego CAS, także po odwołaniu.
Nie usuwamy tombstones i nie resetujemy liczników dla tego samego ID.
`rebind` wymaga ID, expected revision i tych samych jawnych checkout flags
co register. Nie ma cichego fallbacku do nowej revision lub innego checkoutu.

Check zwraca checkout_identity_current i grant_current oraz
execution_ready:false. To odczyt chwili weryfikacji, bez rezerwacji, blokady
checkoutu lub uruchomienia ACP. Descriptor/stamps na stdout nie zawierają ścieżek
i nie można ich deserializować do opaque lokalnej authority.

## Git i linked worktrees

Adapter przypina identity katalogu checkoutu i zatwierdzonych katalogów Git,
w tym objects. Zwykły katalog z .git wewnątrz root jest odrębny od linked
worktree. Symlink do dodatkowego objects root i niezatwierdzone external
metadata są odrzucane. Object alternates wymagają osobnego adaptera.

Linked worktree wymaga podania obu dodatkowych flag `--git-dir` i `--common-dir`
z absolutnymi ścieżkami zatwierdzonymi przez operatora. Adapter sprawdza .git
pointer, commondir, backlink do checkoutu oraz obsługiwany układ worktrees.
Zapisuje identity wszystkich zatwierdzonych katalogów. Wykrycie ścieżki w .git
nie jest automatyczną zgodą na dodatkowy root.

Te piny nie zastępują sandboxa, ograniczeń Git configuration/hooks/filters,
bezpiecznego I/O i guardiana przyszłego procesu. Wykonanie będzie wymagało
porównania faktycznie stosowanego settings hash oraz aktualnych grantów,
bindingu, authority kontrolera i niezależnego execution lease.

## Ograniczenia

Katalog jest bounded:64 exporty,64 principal grant records na export,4096
zwykłych wpisów audytu. Tombstones wliczają się do tych limitów. Pełny zwykły
audyt blokuje nadawanie nowej authority, lecz odwołanie ma obowiązkowy osobny
bounded latest receipt. Ostatnia reprezentowalna revision jest rezerwowana
na terminalną odmowę; nie jest ponownie nadawana pozytywna authority.

Worker v2 z jawnym exports_state_dir odczytuje wskazany export przez bieżący
zweryfikowany WSS; nie tworzy przy tym state directory, bazy ani schematu.
Kontroler ma konfigurację obserwowanego bindingu i panel w projektach zespołu.
Typed BindingRef i resolver metadanych celu sesji należą do odrębnej warstwy
kontrolera. Brakuje durable dispatch/inbox/outbox, przyjęcia próby oraz guardiana
procesu zdalnego. Zdalne wykonanie E4.4 pozostaje otwarte. Odbiór dwóch realnych
hostów i platform należy do E4.7/E10.

## Operator-owned execution policy snapshot (przygotowanie E4.4)

To fundament polityki dla przyszłego executora. Nie uruchamia procesów, nie
aktywuje Remote target i nie zmienia `execution_ready:false`. Sam dawny
`--settings-digest` pozostaje dowolnym zatwierdzonym tokenem metadanych; nie
jest dowodem istnienia wykonawczej polityki ani faktycznie zastosowanej konfiguracji.

`register` i `rebind` przyjmują dokładnie jedną z flag `--settings-digest HASH`
lub `--execution-policy /ABSOLUTE/operator-policy.json`. Przykładowa polityka:

```json
{
  "schema_version": 1,
  "harnesses": ["codex"],
  "sandbox_required": true,
  "credential_mode": "isolated",
  "max_seconds": 300,
  "max_output_bytes": 1048576,
  "max_parallel": 1,
  "acknowledge_repository_wide_metadata_write": false
}
```

Lista harnessów musi zawierać 1–3 unikalne ID z `opencode`, `codex`, `claude`.
Wymagany jest sandbox i tryb credentials `isolated`; to ograniczenie przyszłego
executora, bez zaimplementowanego tutaj przepływu sekretów. Limity wynoszą
1–3600 sekund, 1024–16777216 bajtów wyjścia i 1–8 równoległych prób. Są górnymi
limitami polityki; nie stanowią obietnicy dostępnej pojemności. Nieznane pola,
w tym env, argv i dodatkowe roots, są odrzucane. Przyszłe profile/model/mode i
limity zadania muszą zostać przecięte z tą polityką, bez rozszerzania uprawnień.

Przygotowanie zdalnego MCP dodaje osobną politykę `schema_version:2` z wymaganym
`"mcp_broker":true`; pozostałe pola i ograniczenia są takie same. Schema1 nie
przyjmuje tego pola, nawet z wartością false. Schema2 bez jawnego true jest
odrzucana. To zgoda operatora na używanie mostu, nie grant do repozytorium,
konta OAuth ani konkretnego narzędzia. Nadal wymagane są własne aktualne Read
i Execute dla wskazanego principal, zgodny snapshot ustawień oraz żywy kanał
z ofertą brokera. Zmiana pliku wymaga rebind i ponownego nadania grantów.
Sam zapis polityki2 nie włącza negocjacji v5 ani proxy MCP. Nowy ledger przyjęcia
sesji przechowuje wybrane limity brokera w opcjonalnych pinach; stare sesje
zachowują poprzednie bajty i fingerprinty. Starszy program może odrzucić nową
politykę lub nowe piny; nie należy traktować downgrade jako wznowienia sesji.

Linked worktree z polityką wykonania wymaga jawnego
`acknowledge_repository_wide_metadata_write:true`. To zgoda na zapis we wspólnych
metadata Git, obejmujących refs/objects innych worktrees tego repozytorium.
Pole można pominąć (domyślne false) dla zwykłego checkoutu. Metadata-only linked
export zachowuje dotychczasowe zachowanie. Rebind polityki zwiększa revision
exportu i odwołuje wcześniejsze granty. Dopiero nowy jawny grant `read,execute`
może reprezentować zgodę na przyszłe wykonanie z zapisem w checkout i tych
zatwierdzonych wspólnych metadata. Nie poszerza to ogólnych workspace FS/vault
roots i nie daje authority innym principalom.

Plik polityki musi być własnością efektywnego UID operatora, zwykłym plikiem
z jednym hardlinkiem i bez zapisu dla grupy/innych (np. `chmod 600`). Ścieżka
musi być absolutna, bez symlinków także w katalogach nadrzędnych. Loader otwiera
kolejne komponenty przez deskryptory z NOFOLLOW; FIFO i inne specjalne pliki
są odrzucane bez blokującego odczytu. Maksymalny rozmiar to 32768 bajtów.
Zmiana metadanych pliku podczas odczytu powoduje odmowę.

`settings_digest` jest SHA256 **dokładnych odczytanych bajtów** polityki:
bez normalizacji JSON, usuwania newline, BOM lub zmiany whitespace. Parser
sprawdza te same bajty, które są hashowane i utrwalane. Modyfikacja pliku po
rejestracji nie zmienia snapshotu; wymaga jawnego rebind i ponownych grantów.

WorkerStore schema2 przechowuje jedną politykę na export w
`export_execution_policies`, wraz z dokładną export revision. Register/rebind,
polityka, checkout, odwołanie grantów i audyt mają jeden commit. Każdy odczyt
polityki ponownie waliduje raw JSON, digest i revision; niespójność odmawia
pozytywnej authority, ale nie blokuje odwołania exportu. Metadata-only rebind
usuwa dawną politykę. Nie powstaje automatyczna polityka dla starych rekordów.

Lokalne polecenie operatora migruje schema1 do schema2 atomowo. Peer-request
`open_existing` nigdy nie tworzy katalogu/tabel i nie migruje: schema1 pozostaje
obsługiwana wyłącznie jako metadane, bez snapshotu polityki wykonania. Ten etap
nie implementuje durable dispatch/inbox/outbox, execution lease ani executora;
kolejne zależności opisuje [plan wykonania](worker-execution-plan.md).
