# Dziennik implementacji

## 2026-11-10 — rozpoczęcie

- Utworzono plan E0–E10 i zadania w `README.md`.
- Uwzględniono korektę Tailscale-only + wyjątek loopback, live WebSocket,
  brak publicznego proxy/Funnel i brak SSH jako transportu.
- Sprawdzono oficjalne Aperture MCP i Tailnets API; źródła i ograniczenia
  zapisano w `tailscale.md`.
- Zapisano architekturę, zakres UI, dwa workflow i kryteria odbioru.
- Brak zmian w tailnecie, credential stores, desktopie lub danych użytkownika.
- Realizacja aplikacji i testy: w toku. Żaden test manualny nie jest zaliczony.

## 2026-11-10 — E0 zrealizowany, E1 częściowo zaimplementowany

### Zrealizowane punkty

- **E0.1–E0.4:** zapisane decyzje, oficjalne źródła Tailscale, kontrakty live,
  model docelowy, macierz ekranów i oba workflow. Odbiór dokumentacyjny, nie
  odbiór AHP ani dostawców zewnętrznych.
- **E1.1:** niezależny pakiet `apps/web`, React/TypeScript/Vite, dev proxy tylko
  do loopback. Marketing `web/` i desktop pozostają odrębne. Kontrola typów
  wykonana statycznie; produkcyjny build nie został uruchomiony.
- **E1.2a:** publiczny registry i adaptacja darmowego ReUI Frame na podstawie
  rzeczywistego kodu i API. Zachowano MIT/copyright; build ma emitować notice.
  Brak premium blocks/templates/icons. MCP/skill i odbiór renderowanego UI otwarte.

### Napisany kod E1 — jeszcze bez odbioru wykonawczego

- `crates/relay-protocol/src/web.rs`: jawnie osobny od AHP, wersjonowany kontrakt
  read-only katalogu, snapshot/resync/error, limity i wspólny Rust/TS fixture.
- `apps/server/src/api/web.rs`: same-origin browser pairing, osobny losowy
  browser token (tylko hash w pamięci), HttpOnly/SameSite cookie, 8h expiration,
  logout/revocation, zachowanie polityki hostowych kont Git.
- `/v1/web/live`: WebSocket z autoryzacją po cookie, kontroli Origin/Host,
  ping/pong, limitami i resync. Snapshot obejmuje wyłącznie granty i własne sesje.
- `apps/server/src/store.rs`: payload-free push przy zapisach katalogu w tym
  procesie. Zmiany zapisane innym procesem wymagają jeszcze reconnect.
- Serwowanie jawnie wskazanego buildu przez `--web-root`, TOML/env, whitelist
  assets, CSP, brak dostępu do source/config/secrets i brak fallbacku dla API.
- Web: parowanie, katalog projektów, wybór projektu, indeks własnych sesji,
  filtrowanie, status live, reconnect/backoff, ochrona przed spóźnionymi danymi
  i zmianą tożsamości po reconnect. Brak atrap sesji, chatów lub liczników runs.
- Dodano **9 testów browser API, 3 testy kontraktu Rust i 11 testów TS**.
  To liczba napisanych testów, **nie liczba testów zaliczonych**.
- CI ma niezależny `app-web` (locked dependencies bez lifecycle scripts,
  typecheck/test/build) uwzględniony w wymaganym gate `test`. Dostosowano test
  kontraktu CI i dołączono `implement/`/README aplikacji do kontroli dokumentacji.

### Weryfikacja faktycznie wykonana

| Kontrola | Wynik i granica |
| --- | --- |
| TypeScript `tsc --noEmit` | PASS: statyczna kontrola wszystkich źródeł/testów/config Vite. Użyto zainstalowanego TS 6.0.3 i deklaracji z istniejącego `web/node_modules`, z tymczasową konfiguracją resolvera. Nie uruchamiano kodu aplikacji/testów ani Vite config. Standardowe `npm run check` po `npm ci` nadal wymaga potwierdzenia. |
| `rustfmt --check --edition 2024 --config skip_children=true` na 8 zmienionych plikach Rust | PASS: parser/format, nie kompilacja ani typecheck Rust. |
| `actionlint -shellcheck= -pyflakes= .github/workflows/ci.yml` | PASS: statyczny workflow lint, nie wykonanie CI ani wbudowanych skryptów. |
| JSON/AST/linki | PASS: 5 plików JSON, 2 Python AST, 4 lokalne linki planu; dokładne wersje root dependencies zgodne z lockfile. Bez importowania i wykonywania kodu projektu. |
| `git diff --check` | PASS dla zmian śledzonych. |

Lockfile aplikacji został wygenerowany z istniejących lokalnych metadanych
`web/package-lock.json` (82 pakiety, te same wersje/integrity); nie instalowano
pakietów i nie uruchamiano lifecycle scripts. Poprawność rzeczywistego `npm ci`
pozostaje bramką, nie jest uznana na podstawie samego JSON.

### Historyczna blokada walidacji wykonawczej (2026-11-10)

Próba launchera dla `cargo test -p dunlin-relay-protocol --offline --locked`
zatrzymała się w konfiguracji limitu pamięci (`RLIMIT_AS`), **przed uruchomieniem
Cargo**. Smoke konkretnego, zbyt restrykcyjnego profilu `sandbox-exec` kończył się
`SIGABRT`; późniejszy test z poprawionymi regułami odczytu działał. Nie oznaczało
to ogólnej niedostępności macOS sandbox. Docker był wtedy zatrzymany; obecny stan
walidacji opisuje wpis z 2026-11-11 poniżej.

Dlatego **E1.3–E1.7 i E2–E10 pozostają otwarte**. Rust nie został skompilowany,
nie uruchomiono testów jednostkowych, WebSocket E2E, produkcyjnego buildu ani
browser acceptance. Zielony lint/typecheck nie zastępuje tych kontroli.

### Następna kolejność pracy

1. Przywrócić środowisko izolowane: sieć wyłączona, pusty environment/HOME,
   readonly source/toolchain, ograniczony scratch i CPU/RAM/pids/dysk/wall time.
2. Na dostępnych już zależnościach zweryfikować `cargo test -p dunlin-relay-protocol`
   oraz `cargo test -p dunlin-relay api::web` i Clippy. Naprawić ewentualne błędy.
3. Zweryfikować locked dependency graph, standardowy frontend typecheck,
   `npm test` i `npm run build`; dodać izolowany test realnego WebSocket i assets.
4. Odbiór dwóch browserów (stale scope, revocation, reconnect) i Tailscale Serve
   po zgodzie operatora. Dopiero wtedy zaznaczać pozostałe punkty E1.
5. Kontynuować E2/E3: zespoły i odłączenie lifecycle ACP od połączenia klienta;
   nie budować edytora workflow na pozornie ukończonym runtime.

Nie zmieniono prawdziwych baz, keyringu, tailnetu, instalacji agentów ani
ustawień desktopu. Cała implementacja celu użytkownika **nie jest ukończona**.

Końcowa kontrola statyczna treści 26 nowych plików aplikacji/planu/fixture:
PASS (JSON, Python AST, końcowe newline i brak trailing whitespace). Tymczasowy
wolumen walidacyjny odmontowano i usunięto wyłącznie własne pliki scratch.

## 2026-11-11 — izolowane testy i fundamenty zespołów/workflow

### Zmiany i granice

- Po jawnej zgodzie operatora uruchomiono istniejącą Colimę `default`, bez zmiany
  globalnego kontekstu Docker i konfiguracji profilu. Nie zmieniano kontenerów
  innych projektów. Usunięto wyłącznie własny osierocony kontener `dunlin-check-*`.
- `scripts/test_web_sandbox.py` wykonuje testy na prywatnym snapshotcie źródeł,
  bez sieci, sekretów, HOME użytkownika, `.git` lub socketu Docker; root/source
  readonly, uid 65534, dropped capabilities, CPU/RAM/PID/file/scratch/wall limits.
  Probe izolacji przeszedł. Kontener kompilacji amd64 pod emulacją przekroczył
  timeout — to nie był zaliczony test Rust.
- Pobrano oficjalny obraz `rust:1.99-slim-bookworm` dla ARM64
  (`sha256:f2c004e3d2d8b547e8cf7ffbc29205385f4e71d23e53e4e383065db5e9f822cd`).
  Natywne wykonanie odblokowało testy; kompilacja pozostaje offline.
- E2 częściowo: model pięciu ról i przecięcia role/grant; revision-bound authority,
  human-only approvals, separation of duties i jawne współdzielenie sesji.
  Migracja 9 dodaje opt-in workspaces/memberships/audit, CAS zmian i ochronę
  ostatniego właściciela. Cookie API i ekran zespołów obsługują te metadane.
  Nie wdrożono jeszcze OIDC/zaproszeń ani podpięcia projektów i runtime do ról.
- E7 częściowo: wersjonowany model grafu, typed ports, limity, walidacja cykli,
  reachability i bounded Repeat. To nie jest jeszcze silnik wykonawczy.
- Dodano testy rzeczywistego WebSocket: scoped push, reconnect, hidden mutations,
  odwołanie sesji i odrzucanie Origin; oraz test odpowiedzi HTTP dla assets.
  Wynik całego relay jest jeszcze w toku.

### Dowody wykonania

Wspólne flagi launchera: `--docker-host unix:///Users/corapoid/.colima/default/docker.sock`
oraz `--tmp-root /Users/corapoid/git_priv/dunlin/target/web-checks`.

| Kontrola | Wynik |
| --- | --- |
| `python3 -I scripts/test_web_sandbox.py --suite frontend` | PASS: 15 testów TS, snapshot `80ebac3ecb9da7ef63d783c7c435d11805574c67b7f61993b61ed6537e795aa8`. |
| `--suite typecheck` | PASS: standardowy `tsc --noEmit`, także z Linux dependencies po `npm ci`; snapshot `3a548843c69d0df7fa2fefc8f43aa56a0e3b896e4b8ca6655b8424cdf771a893`. |
| `--suite domain --rust-image rust:1.99-slim-bookworm --rust-home /usr/local/rustup` | PASS: 40 domain + 6 relay-protocol, 0 failed; snapshot `e8956510f187768a3b1309287984bb27573edca0499f865106b57e226a9eb902`. |
| Linux `npm ci --ignore-scripts --no-audit --no-fund` | PASS: 41 packages; wyłącznie manifest/lock i katalog zależności w task scratch, bez lifecycle scripts/poświadczeń. |
| Produkcyjny build Vite | Pierwsza próba z readonly dependencies ujawniła zapis `.vite-temp`; launcher przełączono na native config loader. Ponowna weryfikacja w toku. |

E1 nie jest jeszcze zamknięty: trwa walidacja renderowanego UI. Etapy E2–E10
pozostają otwarte; testy zewnętrznego tailnetu,
keyringu/OAuth/harnessu i odbiór operatorski nie zostały wykonane.

### Kolejny zweryfikowany przekrój (2026-11-11)

- Produkcyjny build: **PASS** (Vite 8.3.6, 22 moduły), snapshot
  `fd26c0f7e400127f3a473e9fcffcb5b27bd9833d22799a8b18201ca8688b9ffa`.
  Native config loader unika zapisu do readonly `node_modules`.
- Pełny relay: **121 PASS, 0 failed**, `--suite server`, snapshot
  `f17820068520ee1142b4335737533de6a6e728dbcf4ad8f75e1b97eed2ab3f2f`.
  Obraz narzędzi `dunlin-web-checks:rust-1.99`,
  `sha256:fcba6ff27fb040f357a5286fd73ff47c381c243c2b09699cb109f2369c78850b`.
  Zawiera Git i Python do istniejących fixtures; testy są nadal offline.
  Poprzedni slim-image run: 84 PASS/32 FAIL z powodu brakujących narzędzi;
  wcześniejsza próba miała SIGKILL przy limicie 1536 MiB. Obecnie serwer ma
  2304 MiB RAM, 768 MiB scratch, 1 CPU i pojedynczy build/test worker.
- Ten run obejmuje rzeczywisty WebSocket/HTTP, kompatybilność migracji,
  workspace/member API, logical project grants oraz trwałe wersje workflow.
  Nowszy test HTTP workflow i browser test dodano później — ten hash ich nie waliduje.
- Zrealizowano **E1.3–E1.5 oraz E2.1**, z powyższymi dowodami. Wymagania pełnego
  E1/E2 nie są tym samym ukończone.
- Migracja 10: logiczne projekty i jawne CAS granty z tombstones; role są sufitem
  grantów, cofnięcie członkostwa blokuje odczyt/zapis. Brak nadania dostępu do
  istniejących checkoutów lub prywatnych sesji. API i UI katalogu są odrębne od
  natywnego API v1.
- Migracja 11: CAS drafts, immutable published workflow versions z digestem SHA-256,
  bounded storage oraz idempotent publication tej samej rewizji. Projektowe
  role/granty są odczytywane przy każdej operacji. Nie ma jeszcze wykonania grafu,
  provider JSON Schema validation ani edytora React Flow; E7 pozostaje otwarty.
- Scoped workspace revision jest fingerprintem widocznych zdarzeń zamiast globalnego
  numeru audytu; zdarzenia workflow są związane z logicznym projektem. Sam licznik
  nie wystarczał przy revocation usuwającym widoczność historycznego zdarzenia.
- Najnowszy frontend: **16 PASS**, snapshot
  `68f02ead867a28f54a38dba095df789fc2180edb45d51e2b68ad8f4773fddec5`;
  standardowy Linux typecheck **PASS**, snapshot
  `d1c08da3856f89f173a4986ee2ae0e29a3f100fea3ed5bd2e8c79b6ce8afe6ea`.
- Dodano jawny test produkcyjnego UI z Playwright/Chromium przeciwko realnemu
  Rust relay, bez mocked fetch/WS. Driver 1.65.0 ma locked install **PASS**.
  Uruchomienie browser testu i Clippy są w toku; nie oznaczono ich jako zaliczonych.

### Produkcyjny browser acceptance — PASS

- `python3 -I scripts/test_web_sandbox.py --suite browser --node-modules
  target/web-checks/linux-node-deps/node_modules` ze wspólnymi flagami socket/tmp:
  **1 PASS, 0 failed**, snapshot
  `dcc924fc3f579267641276e73ea64239c1b275002be43d533ffa102f045ea278`.
- Chromium 156.0.8078.79 i Playwright Core 1.65.0; obraz
  `sha256:77b5375d59066f5a67d407814f1a66bad75771dce938b3ecafcfd44902c2d854`.
  Produkcyjny Vite build: 23 moduły, assets/CSP z realnego Rust HTTP serwera.
- Wykonano dwa odrębne konteksty browsera/użytkowników: jednorazowe pairing,
  HttpOnly/SameSite cookie, realny WebSocket, prywatne sesje i projekty,
  live zmiana członkostwa bez refresh/pollingu, brak dostępu do nieprzydzielonego
  projektu logicznego, reload i izolowane logout. Pusty local/sessionStorage,
  brak błędów JavaScript, light/dark i brak poziomego overflow przy 1280/390 px.
  To automatyczny Chromium acceptance, nie ręczny audyt wyglądu/a11y ani Safari/Firefox.
- Naprawiono konfigurację testów natywnych: usunięto emulacyjne ustawienia
  `MALLOC_CONF` zatrzymujące zwalnianie pamięci. Wcześniejsze SIGKILL nie zaliczają
  browser testu. Natywna kompilacja z domyślnym alokatorem zakończyła się w 81 s.
- CI `app-web` jawnie instaluje locked browser driver i uruchamia test oznaczony
  `--ignored`; bez tego ordinary `cargo test` nie jest dowodem wykonania browsera.
- `--suite tooling`: **13 PASS**, snapshot
  `8e0e8a66d9c55875cdcb84c32f85e4f393d8e42e7f73d4f37acc02ebc4c205bc`.
  Obejmuje kontrakty required CI gate (także failure/cancel/skip) i browser kroku.
  `actionlint -shellcheck= -pyflakes= .github/workflows/ci.yml`: **PASS**.
- Zrealizowano **E1.6–E1.7**. E1.2b (uwierzytelniony ReUI MCP/skill) nadal otwarty;
  nie zamknięto całego E1. Clippy i regresja nowszych testów backendu w toku.

### Pełna regresja i trwałość browser credentials

- Snapshot `dcc924fc3f579267641276e73ea64239c1b275002be43d533ffa102f045ea278`:
  **123 relay tests PASS, 0 failed, 1 browser test ignored**; browser test z tego
  samego snapshotu wykonano oddzielnie i zaliczono powyżej.
- `--suite clippy`, ten sam snapshot: **PASS** dla relay/domain/relay-protocol,
  `--all-targets --offline --locked --no-deps -- -D warnings`. Poprawiono
  `collapsible_if` w walidatorze grafu; nie wyłączano lintów.
- `--suite docs`: **PASS**, 112 plików Markdown, snapshot
  `1141f7fe49b2047fb78b8d921c18a4ac17198a5b11907dbdc9ad56042d393cff`.
- Następna część E2.2: migracja 12 zastępuje pamięciowy browser registry osobną
  tabelą hash token/device/absolute expiry; cookie zachowuje ważność po restarcie.
  Limit 8h bez sliding renewal, maks. 1024 aktywne credential records, usuwanie
  przeterminowanych/odwołanych rekordów przy rejestracji i sprawdzanie revocation
  bez cache. Brak współdzielenia credential table z native bearer API.
- Dodano testy reopen/restart, expiration/logout, limitu registry i separacji
  native/browser credentials. Walidacja tej nowszej migracji i browser regresja
  są w toku. E2.2 nadal nie jest ukończone: brak OIDC i zaproszeń.

- Migracja 12: `--suite server` **127 PASS, 0 failed, 1 browser test ignored**,
  snapshot `bc88f4531f7aaf45ea5ce36ed6eb6391904bf9b381aeb5b3f432c37466bda272`.
  Zalicza HTTP cookie po odtworzeniu AppState/bazy, separację bearer API, expiry,
  revocation, cleanup i limit 1024 rekordów. Pierwsza próba tej migracji wyczerpała
  pamięć linkera; `--no-keep-memory` pozwoliło zakończyć ją bez zwiększania limitu RAM.
- Launcher obsługuje SIGTERM/SIGINT/SIGHUP i timeout z usuwaniem wyłącznie własnego
  losowego kontenera. `--suite tooling`: **13 CI + 4 launcher tests PASS**,
  snapshot `dd764bf210267af76facda5e53ed266194a515b2f0053d508e94e2331ce73f70`.
  Zalicza też readonly source-copy, wykluczenie `.env`/cache i rozmiar wejścia.
  Mock cleanup tests nie wymagają prawdziwego Docker socketa w kontenerze.
- Ponowny rzeczywisty `--suite probe`: **PASS**, readonly source/root, brak
  zewnętrznej sieci/HOME/socketu/.git i hard limits. Żadnych testów na bazie
  operatora, realnym keyringu ani tailnecie.
- Aktualne domain/protocol: `--suite domain` **40 + 6 PASS**, snapshot
  `37436382cc123bc35bf5338e7345a2a9fbafc869935711f1e67ed2ff390360fd`.
- Browser po migracji 12: **1 PASS**, snapshot
  `37436382cc123bc35bf5338e7345a2a9fbafc869935711f1e67ed2ff390360fd`.
  Następnie wyrównano katalog roboczy Vite w browser suite do `apps/web`, jak
  w zwykłym buildzie/CI (aby Tailwind nie skanował całego repozytorium).
- Clippy z migracją 12: **PASS**, snapshot
  `8910d1030047786333c10d2f7752cb19b3d934847408c4a89ac7ab51200ef283`.
- Standardowy produkcyjny build `--suite build`: **PASS**, snapshot
  `7affbef1e3b04096171144e388d5484a5420948bef9d2994399e96a4e9fa224c`;
  23 moduły, 15.75 kB CSS / 245.67 kB JS, notice ReUI obecne.
- `cargo fmt --all --check`: **PASS**. Ponowny checker dokumentacji:
  **112 Markdown PASS**, snapshot
  `f83cd19ac592250498ea31a8f82ed21edb32c77d9e535186ac16885d2d9a65a9`.
- Ostateczny browser run z poprawnym cwd `apps/web`, po migracji 12:
  **1 PASS, 0 failed**, snapshot
  `8910d1030047786333c10d2f7752cb19b3d934847408c4a89ac7ab51200ef283`.
  Produkcyjny build: 23 moduły, 16.02 kB CSS / 245.67 kB JS; test fixture/driver
  jest obecny w snapshotcie, jak w pełnym checkoutcie CI. Weryfikacja nadal
  dotyczy Chromium, a nie Safari/Firefox ani realnego tailnetu/OAuth.

### Nadal wymagane — bez oznaczania jako ukończone

1. E1.2b: uwierzytelniony ReUI MCP/skill. Chromium acceptance nie zastępuje
   brakującego dostępu do tego narzędzia ani pełnego audytu dostępności.
2. E2: OIDC/zaproszenia, service principals, powiązanie grantów z host authority
   i rzeczywiste współdzielenie sesji. Obecne modele/granty nie izolują nieufnego
   procesu przez samą rolę i nie nadają dostępu do hostowego Git.
3. E3: server-owned interactive runtime, AHP 1.0.0 + conformance, projekcja ACP,
   replay/zgody i UI rozmowy. Istniejący job runner nie jest pełnym runtime sesji;
   legacy ACP bridge nadal ma lifecycle związany z WebSocketem.
4. E4–E10: host bindings/enrollment/fencing, adaptery i integracje, trwałe runs,
   scheduler/wykonanie grafu, edytor, oba workflow produktowe i pełny odbiór.
   Przechowywanie definicji/publikacja wersji nie oznacza wykonania ani wdrożenia.

Cel obejmujący wszystkie punkty do E10 **pozostaje nieukończony**. Żadne testowe
konto ani zgoda na realne provisioning/OAuth nie zostały założone w imieniu operatora.

## 2026-11-11 — kontynuacja: zaproszenia E2.2

- Sprawdzono plan E0–E10, istniejące zmiany i dowody. Brak AGENTS.md w repo.
  Cel całości pozostaje aktywny. Nie nadpisano dotychczasowych zmian i nie
  wykonano commit/push.
- Migracja 13: ograniczony katalog zaproszeń dla istniejących tożsamości relay,
  token zapisany wyłącznie jako hash, 24h expiry, wskazany odbiorca, jednorazowa
  atomowa akceptacja, odwołanie wszystkich zaproszeń odbiorcy w workspace.
- API cookie/same-origin: POST `/v1/web/workspaces/{id}/invitations`, DELETE
  `/v1/web/workspaces/{id}/invitations/{recipient}`, POST
  `/v1/web/invitations/accept` z tokenem w JSON body. Token tworzenia zwracany
  raz w odpowiedzi no-store; nie należy go umieszczać w URL ani browser storage.
- Akceptacja ponownie sprawdza authority wystawcy oraz dokładne rewizje
  wystawcy i odbiorcy. Zmiana członkostwa unieważnia stare zaproszenie.
  Owner transfer pozostaje osobną operacją CAS; zaproszenie nie przenosi własności.
  Audit obejmuje wystawienie, odwołanie i osobę akceptującą.
- E2.2 pozostaje otwarte: brak OIDC, provisioning nowych użytkowników przez
  zewnętrznego dostawcę i UI zaproszeń. Nie wysyłano wiadomości ani zaproszeń
  do rzeczywistych osób. Brak zmian w realnym tailnecie/keyringu/integracjach.
- Dodano testy store i HTTP dla recipient binding, replay, expiry, revocation,
  stale authority/CAS. Walidacja wykonawcza w toku; nie zaliczono jej na
  podstawie obecności testów. `cargo fmt --all` i `git diff --check`: PASS.
- Pierwsza próba server, snapshot
  `2118df21687cfd0e5f38aaa3e22d8019a2a6882e64a7227cd9e2cbd14c8bf820`:
  kompilacja źródeł zakończona, linker zabity sygnałem 9 (limit zasobów),
  testy niewykonane. Druga próba obejmuje finalny audit akceptacji, test HTTP
  i reopen/hash test; nie uruchamiać równolegle kolejnych ciężkich buildów.
- Końcowa regresja `python3 -I scripts/test_web_sandbox.py --suite server
  --docker-host unix:///Users/corapoid/.colima/default/docker.sock --tmp-root
  /Users/corapoid/git_priv/dunlin/target/web-checks`: **131 PASS, 0 failed,
  1 browser test ignored**, snapshot
  `380b07dff6b5a162330ec375ae5b32241075abffe3b2565e1c901fafebd91e20`.
  Obejmuje 4 nowe testy zaproszeń, migrację starej historii i dotychczasową
  regresję relay. Kompilacja 2m07s, wykonanie testów 40.60s. Browser test nie
  był w tym przebiegu wykonywany. Clippy końcowej wersji w toku.
- Końcowy `--suite clippy` (te same socket/tmp flags i snapshot
  `380b07dff6b5a162330ec375ae5b32241075abffe3b2565e1c901fafebd91e20`):
  **PASS**, relay/domain/relay-protocol, all-targets, offline/locked,
  `-D warnings`, 1m09s. `python3 -I scripts/check_docs.py`: **112 Markdown PASS**.
  Pierwszy linker failure nie powtórzył się w drugiej próbie; nie zwiększano
  limitów kontenera ani nie zmieniano izolacji.

## 2026-11-11 — UI zaproszeń E2.2

- Poprzedni turn celu: postęp (migracja 13, API, 131 relay tests PASS,
  Clippy PASS), nie blokada. Kontynuacja oparta na bieżących plikach.
- Panel zespołów umożliwia tworzenie zaproszenia dla istniejącego user ID,
  wybór roli bez transferu ownership, jednorazowy odczyt kodu, ukrycie kodu,
  odwołanie zaproszeń odbiorcy i przyjęcie zaproszenia przez jego cookie sesji.
- Token wyłącznie w pamięci zamontowanego panelu; brak URL/browser storage.
  Po submit pole odbiorcy jest czyszczone. Po opuszczeniu scope/user/revision
  panel kodu i trwające żądania są usuwane/anulowane. Brak wysyłki wiadomości.
- Rozszerzono realny Chromium test dwóch użytkowników o utworzenie, odwołanie,
  odrzucenie cofniętego tokenu, przyjęcie nowego i odrzucenie replay.
  Wykonanie browser testu w toku. Standardowy izolowany `--suite typecheck`
  PASS dla snapshotu `2f9cbe7b8d73dcf9bd511681976700a446f9090e264ac69fb3f54d9aaabd1457`.
- Dalszy OIDC: oficjalna specyfikacja i API `openidconnect` 4.1.0 sprawdzone:
  https://openid.net/specs/openid-connect-core-1_0.html oraz
  https://docs.rs/openidconnect/4.1.0/openidconnect/ . Docelowo code flow + PKCE,
  browser-bound state/nonce, weryfikowany ID token i lokalne mapowanie issuer/sub.
  Brak automatycznego łączenia kont po emailu; OIDC nadal niezaimplementowane.
- Realny `--suite browser` ze wspólnymi flagami socket/tmp i Linux node deps:
  **1 PASS, 0 failed**, snapshot
  `7231fee6a4e7c18ac8ab641066a31c8d9a69fb70220e1d7c18f6873aa948d1c5`.
  Produkcyjny Vite build 24 moduły, Chromium, prawdziwe HTTP/WS i dwie sesje.
  Sprawdzono flow zaproszeń przez UI: revoke, rejected revoked code, accept,
  rejected replay, czyszczenie pola po submit i ukrycie kodu. Nadal PASS dla
  prywatnych projektów/sesji, logout, pustego local/sessionStorage, responsive
  light/dark i braku JS errors. Czas browser scenariusza 4.89s.
- Po tym snapshotcie dodano user ID do React key panelu administratora, by
  wymusić usunięcie kodu także przy zmianie tożsamości. Nie twierdzimy, że
  browser test symulował zmianę tożsamości w tym samym zamontowanym panelu.
- Końcowy frontend: `--suite frontend` **16 PASS**, snapshot
  `a4db0a219067c294ed310a5562de7f7247825a62e1fd1b0e17fbf2cf6018bc63`;
  standardowy `--suite typecheck` **PASS**, snapshot
  `ae7df636121c45a1a8e45c341df0d2777de812e3a9b5a39074c91503e6890797`.
  Oba obejmują finalny React key. Docs 112 Markdown i diff whitespace PASS.
  Nie zmieniano kodu Rust od poprzedniego zaliczonego Clippy/131 relay tests.

## 2026-11-11 — implementacja OIDC E2.2 (weryfikacja w toku)

- Poprzedni turn: postęp, ukończone UI zaproszeń i Chromium acceptance.
  Obecnie wdrożono rzeczywisty code flow OIDC, nie atrapę pairing.
- Dodano przypięte `openidconnect = 4.1.0` bez domyślnego klienta HTTP; transport
  używa istniejącego dunlin-http, timeout 8s/request, limit 1 MiB odpowiedzi,
  bez proxy environment i bez podążania za redirectami. Pobranie zależności
  Cargo uzyskało odrębne zatwierdzenie; kompilacja/testy pozostają offline.
- Config TOML `oidc`: issuer, client ID, dokładny prywatny redirect URI,
  nazwa env var sekretu, jawne dodatkowe endpoint origins i mapa subject ->
  istniejący user ID. Brak łączenia po emailu, samorejestracji i nadawania ról
  z claims. Bootstrap administratora przez istniejące lokalne CLI i jawny binding.
- POST start wymaga same-origin, callback wymaga zgodnego Host, jednorazowego
  state i flow cookie HttpOnly/Lax. PKCE S256 i nonce po stronie serwera.
  Pending logins: max 128/5min, max 2 równoległe operacje z utrzymaniem permit
  do końca blocking task również po anulowaniu żądania HTTP.
- Walidacja ID tokenu przez bibliotekę: podpis (RS256/ES256/EdDSA), issuer,
  audience, nonce, exp, at_hash jeśli obecny. Dodatkowo jawna kontrola azp
  i świeżości iat (-10min/+60s). Brak ekspozycji provider tokenów i refreshu.
- Migracja 14: trwała provenance OIDC device/subject/config digest. Cookie
  po restarcie jest nadal sprawdzane wobec bieżącej konfiguracji/bindingu;
  zmiana configu lub usunięcie OIDC odwołuje authority wcześniejszych cookie
  OIDC. Pairing pozostaje osobny. TTL min(8h, ID token expiry), bez sliding.
  Utworzenie device/provenance/browser credential jest jedną transakcją.
- API i UI logowania, instrukcja operatora w apps/web/README.md. Callback
  usuwa flow cookie, redirect na `/`, no-store/no-referrer. Lokalny logout
  nie wylogowuje IdP. Nie ma back-channel logout: odwołanie u samego IdP
  nie skraca automatycznie lokalnego expiry.
- Dodano lokalny podpisujący testowy IdP i testy błędnego issuer/aud/nonce/exp,
  podpisu, azp/iat, at_hash, nieprzypisanej tożsamości, browser binding, replay,
  cross-origin/Host, restartu/provenance i rollbacku nieudanej sesji.
  PEM w tests/fixtures/oidc jest wygenerowanym kluczem wyłącznie fixtures.
  Dodano także ignored produkcyjny Chromium OIDC acceptance wybierany istniejącym
  filtrem `production_browser_acceptance` w CI/launcherze.
- Pierwszy Clippy snapshot `1ac49670a1d021b0a69b14a7413696976c9f75e5f99f2cd444ae39a02e2c493b`:
  błąd brakującej bezpośredniej zależności dunlin-http; poprawiono manifest/lock.
  Server snapshot `1f5b7b3d6ca7794f8dc1e57715299d10196f345839fafb0ac77d431a4fa66804`:
  scratch 768 MiB wyczerpany przy kompilacji zależności, testy niewykonane.
  Server/browser launcher ma teraz 3072 MiB RAM i 1280 MiB scratch,
  nadal readonly source/root, sieć none, user nobody i hard limits.
- Bieżąca regresja server w toku. Żadnego nowego testu OIDC ani browsera
  nie zaliczamy przed zakończeniem wykonania. Odbiór prawdziwego IdP/tailnetu
  i pełny E2 nadal otwarte; nie zmieniano prawdziwych kont ani tailnetu.
- Server snapshot `0ed50d2cd71929c4660eb91715ec350ec8814ef3c9fd355359720364079664b8`:
  rustc zabity SIGKILL przy 3 GiB RAM. Wydzielono produkcyjną implementację
  do crates/identity (dunlin-identity), z osobnym testem konfiguracji. Relay
  używa prostego publicznego API; openidconnect w relay jest tylko dev dependency
  lokalnego podpisującego IdP. Sandbox server testuje oba pakiety, Clippy również.
- Snapshot po wydzieleniu `431a61152efdee2ecf444248665ff5dbb9169019cb4e7fbb4a62ea7d5f274757`:
  kompilacja Rust przeszła do linkowania; GNU ld został zabity SIGKILL.
  Testów tego przebiegu nie wykonano. Odczyt Docker info wykazał 4096561152 B
  pamięci VM; nie zmieniano jej konfiguracji. Potwierdzono obecność ld.lld
  w istniejącym toolchainie Rust i przełączono launcher na LLD z --threads=1,
  bez sieci/instalacji nowego linkera ani podnoszenia limitów kontenera.
- Frontend OIDC: 16 testów **PASS**, snapshot
  `e5005667d1f3562434562c19e37e6aab958ea90fa3c451a17ce23f7629899839`;
  typecheck **PASS**, snapshot
  `b5e435d0085e7d90950ad5268ad939ca490be571b1aeb1d5adc8fecbf6ddc214`.
  Docs 112 Markdown, diff whitespace i 4 launcher regression tests: **PASS**.
- LLD zadziałał bez zwiększania limitów: snapshot
  `431a61152efdee2ecf444248665ff5dbb9169019cb4e7fbb4a62ea7d5f274757`
  skompilowany w 1m41s. Wynik: 1 identity PASS; relay 133 PASS, 2 FAIL,
  2 ignored. Dwa FAIL wynikały z formatu fixture PEM (PKCS#8 zamiast PKCS#1
  oczekiwanego przez CoreRsaPrivateSigningKey), nie z produkcyjnego klucza
  użytkownika. Własny test-only key przekonwertowano do PKCS#1.
- Końcowy browser launcher buduje UI i uruchamia zwykłą regresję identity/relay,
  a dopiero po jej zaliczeniu oba ignored Chromium acceptance. Nie traktuje
  zwykłego `cargo test` z ignored testami jako dowodu działania przeglądarki.
  Weryfikacja poprawionego snapshotu nadal w toku.
- Poprawiony snapshot `4b840ce7c7164cac42e0c2a92dff85cdcbd8a84c11f7e9165228ae0006e4d30b`:
  **135 relay PASS, 0 failed, 2 ignored; 1 identity PASS**. Obejmuje wszystkie
  nowe testy OIDC i pełną wcześniejszą regresję. Dotychczasowy Chromium flow
  pairing/zespoły/zaproszenia nadal **PASS**.
- Nowy Chromium OIDC przeszedł logowanie, callback i live catalog, ale asercja
  cookie odpytywała root URL zamiast zakresu Path=/v1/web, zwracając brak cookie.
  Poprawiono wyłącznie kontrolę drivera (`context.cookies()`); cookie aplikacji
  zachowało dotychczasowy ograniczony Path. Dodano celowany `--browser-test oidc`
  do ponawiania tego testu bez powtarzania zaliczonej regresji backendu.
  Celowany przebieg w toku; powyższy częściowy wynik nie zalicza browser OIDC.
- Końcowy celowany Chromium OIDC **PASS**, snapshot
  `a2e44f160de64e8c863c62de15b74f9c1e5e623e410f080260c98f54965ba2e0`,
  `--suite browser --browser-test oidc` ze wspólnymi flagami socket/tmp i Linux
  node deps. Test rzeczywiście wykonał discovery/JWKS/code exchange z PKCE
  i podpisem RS256, callback cookie, live catalog, reload i logout. Brak
  provider tokenów w browser storage, brak JS errors. Browser scenariusz 3.21s.
- Produkcyjny Vite build z OIDC: **PASS**, 25 modułów, 16.02 kB CSS / 251.75 kB JS.
  Zaliczone testy przeglądarkowe dotyczą lokalnego loopback IdP w Chromium,
  nie realnego cross-site IdP, Tailscale Serve, Safari/Firefox lub operatora.
  Nie oznaczono pełnego E2.2 ani całego E2 jako zakończonych.
- Końcowy `--suite clippy` **PASS**, snapshot
  `a2e44f160de64e8c863c62de15b74f9c1e5e623e410f080260c98f54965ba2e0`:
  identity/relay/domain/relay-protocol, all-targets, offline/locked,
  no-deps, `-D warnings`, 1m06s. `cargo fmt --all --check`: **PASS**.
  13 CI workflow/packaging tests i 4 launcher tests: **PASS**. Docs 112
  Markdown i `git diff --check`: **PASS** po aktualizacji stanu architektury.
- Następna praca: dalsza realizacja E2 (pełne onboarding/provisioning,
  host grants i service principals, współdzielenie i izolacja), następnie
  server-owned ACP/AHP runtime E3. Plan E0–E10 pozostaje w całości aktywny.
  Nie wykonywano commit/push, logowania do prawdziwego IdP ani zmian tailnetu.

## 2026-11-11 — E2.3: dedykowane service principals

- Poprzedni turn celu: postęp, zweryfikowane OIDC z fixture i Chromium.
  Kontynuacja na bieżących źródłach; pełny cel E0–E10 pozostaje otwarty.
- Migracja 15 dodaje osobne, ograniczone tożsamości automatyzacji: jeden projekt,
  jawne view_project/optional run_workflow, hash tokenu dsp_, absolutny expiry
  do 30 dni, CAS revocation i tombstone. Nie są rekordami users/devices ani
  credentials OIDC/browser/native. Utworzenie/odwołanie ma zdarzenie audytu
  związane z projektem; brak sekretów w metadanych/listach/audycie.
- Cookie API owners/admins: create/list/revoke. Dedykowany bearer API czyta
  wyłącznie opublikowaną wersję workflow z autoryzacją scope przy każdym odczycie.
  Używa PrincipalKind::Service i roli-ceiling Member, bez human approvals.
  Grant run_workflow jest przygotowany dla przyszłego silnika, nie ma endpointu
  wykonania i nie oznaczono E7 jako zrealizowanego. Host grants nadal otwarte.
- Dodano testy store (restart/hash-only/scope/expiry/revocation/CAS/approval)
  i HTTP (published read, rozdzielenie native/browser, natychmiastowe odwołanie).
  Weryfikacja wykonawcza w toku. Nie zaliczamy ich przed zakończeniem procesu.
- Instrukcja operatora w apps/web/README.md; bez tworzenia rzeczywistych tokenów
  w bazie użytkownika, bez commit/push i bez zmian realnych usług.
- Pierwszy server snapshot
  `818241056a0c4b9559daf36e401f9eebdfd03fef2acea3a59e447f5cd2ceae56`:
  błąd kompilacji nowego testu HTTP (mieszane &String/&str w tablicy).
  Poprawiono jawne as_str; regresja ponowiona. Żaden test tego pierwszego
  przebiegu nie jest oznaczony jako zaliczony.
- Końcowy `--suite server` **138 relay PASS, 0 failed, 2 browser tests ignored**;
  identity **1 PASS**, snapshot
  `883eb6d7b75c21075d522aaa5e238af0d3955fb16a3dbd7090ed5ba01a902de7`.
  Wspólne flagi: Docker socket Colima default i target/web-checks jak wcześniej.
  Regresja obejmuje nową migrację, stare migracje/transkrypty, HTTP publikacji,
  OIDC i wszystkie nowe testy service principals. Czas testów relay: 41.28s.
  Browserów nie uruchamiano ponownie przy tej zmianie backendu; pozostają
  oddzielnymi dowodami poprzedniego przekroju. Clippy w toku.
- Końcowy Clippy **PASS**, ten sam snapshot
  `883eb6d7b75c21075d522aaa5e238af0d3955fb16a3dbd7090ed5ba01a902de7`:
  relay/identity/domain/relay-protocol, all-targets, offline/locked,
  no-deps, -D warnings, 1m05s. Formatowanie, whitespace i kontrola 112
  dokumentów Markdown **PASS**. Nie zamknięto całego E2.3: host grants oraz
  wykorzystanie service authority w silniku workflow nadal wymagają pracy.

## 2026-11-11 — E3: trwały dziennik i kolejka sesji (część runtime)

- Poprzedni turn: postęp, service principals i 138 relay/1 identity PASS.
  Sprawdzono aktualny most ACP: nadal owns child per WebSocket i connection-bound
  lease/grants/coordination. Nie usunięto tej ochrony ani nie zmieniono API v1.
- Zweryfikowano ponownie oficjalny tag AHP v1.0.0 przez raw README/spec źródła
  https://github.com/microsoft/agent-host-protocol/tree/v1.0.0 . SDK klienta
  nie jest serwerem AHP; nie ogłoszono własnego katalogu/journal jako AHP.
- Nowy crate dunlin-session-runtime zawiera serwerowy dziennik SQLite, prywatną
  własność sesji, FIFO prompt queue, idempotency/fingerprint, serial claim przed
  ACP stdin, ograniczone eventy i atomowe snapshot/replay. Nie uruchamia harnessu.
- Migracja 16 dodaje interactive_sessions/turns/events obok legacy sesji.
  Lease zawiera runtime instance i monotonic epoch; stamp membership/grant
  zmieniający rewizję blokuje dispatch na starej authority. Adapter musi pobrać
  bieżący AccessContext i utrzymać wspólną blokadę Store na całą operację;
  nie wolno traktować cached context jako ponownej autoryzacji.
- Startup recovery wywoływane tylko z serwera po skutecznym bindzie listenera,
  nie przy otwieraniu bazy przez CLI. Active session -> interrupted,
  dispatching/running -> unknown, queued zachowane; ponowne acquire jest jawne,
  starego workera blokuje fencing. Unknown turn nie jest automatycznie requeued.
  Completed oznacza koniec tury, nie sukces zadania/publikacji/workflow.
- Limity: 256 sessions global/64 per owner, 256 turns/32 queued per session,
  prompt/output chunks do 32 KiB, 1024 events i 8 MiB journal per session.
  Zarezerwowano przestrzeń dla recovery także po wypełnieniu normalnej kwoty.
  Replay max 128 events/page; przyszły adapter AHP musi obsłużyć również
  paginację dużych transkryptów, a nie emitować nieograniczonego snapshotu.
- Dodano testy serial dispatch/idempotency/output, reopen/recovery/fencing,
  aktualnej authority/prywatności, kolejek/kwot/rezerwacji recovery. Serwer
  i test suite korzystają z tego samego schematu i startup recovery entry point.
  Weryfikacja wykonawcza w toku, brak nowo zaliczonych testów na tym etapie.
- Nadal otwarte: rzeczywisty proces ACP należący do aktora, host bindings,
  guardian revocation/policies/budgets, AHP/projection/conformance i chat UI.
  Nie ma nowego endpointu wykonania ani przycisku wysyłającego do niegotowego
  runtime. Capabilities chat/workflow pozostają false. Cały E3 nieukończony.
- Pierwszy snapshot runtime
  `4740e6ffcf82a900d4e9cc7d7ec223cff0eb5c60050cdb524e152e8583801a76`:
  błąd kompilacji u64 FromSql/ToSql w rusqlite 0.41.1. Poprawiono sprawdzane
  konwersje signed SQLite integers <-> nieujemne sekwencje/epoch. Nie wykonano
  testów tego przebiegu; aktualna regresja w toku.
- `--suite server` **PASS**: 138 relay, 1 identity i 4 session-runtime tests,
  0 failed; 2 browser acceptance tests ignored (nie uruchamiane w tym przebiegu).
  Snapshot `1941da9da668fc82eb99187b24f1d6826823ac32f32e66afeda3b4416df730eb`.
  Kompilacja 1m33s; relay tests 41.16s, journal tests 0.05s.
- Wzmocniono test prywatności: obcy workspace owner ma poprawny grant projektu,
  więc odrzucenie wynika z prywatnej własności sesji, nie z błędnego grant scope.
  `--suite sessions` **4 PASS**, snapshot
  `464a656e96882c19d236bd3fe54dccc1c5eef4baf5fcf892eb4401ed97e9f827`.
  Nowy celowany suite uruchamia sam journal bez kompilowania całego serwera;
  źródła nadal readonly, network none, 1536 MiB RAM/768 MiB scratch.
- Clippy **PASS** dla relay/session-runtime/identity/domain/protocol,
  all-targets, offline/locked, no-deps, -D warnings; ten sam końcowy snapshot,
  1m11s. `cargo fmt --all --check`, whitespace i 112 Markdown: **PASS**.
  4 regression tests launchera: **PASS**. Brak commit/push lub realnych harness,
  OIDC, tailnet/policy mutations. E3.1–E3.7 i cały cel pozostają nieukończone.

## 2026-11-11 — lokalny ProjectBinding jako warunek podłączenia ACP

- Poprzedni turn: postęp, trwały journal/queue/fencing i testy recovery PASS.
  Sprawdzono obecne źródła; legacy ACP nadal owns child per WebSocket.
- Migracja 17 dodaje jawny binding logicznego team project do istniejącego
  lokalnego native project. Nie tworzy root grantu, nie przenosi prywatnych
  sesji i nie wystawia transportu omijającego Tailscale. host_id=local oznacza
  bieżący koordynator, nie udawany enrollment zdalnego hosta.
- Zapis wymaga bieżącej roli relay admin + workspace ManageHosts + jawnego
  ManageProject oraz istniejącego native grantu autora. CAS revision i audit
  obsługują set/rebind/revoke. Odwołany binding nie rozwiązuje się do checkoutu.
- Odczyt konfiguracji wymaga jawnego logicznego i native grantu. Resolver
  ponownie kontroluje oba granty, canonical path, device/inode tożsamość root
  i lokalnego .git directory oraz digest project settings. Podmiana katalogu,
  symlink target lub ustawień wymaga jawnego rebind z nowszą rewizją.
- Cookie API GET/PUT binding i POST revoke; filesystem przeniesiony na bounded
  spawn_blocking, permit utrzymywany do końca zadania. Device/expiry/OIDC/role
  i host-account policy ponownie czytane pod tą samą blokadą Store co operacja.
  JSON nie ujawnia physical root ani filesystem identity. Pole
  configuration_current oznacza zgodność konfiguracji/pinu, nie gotowość ACP.
- Adapter obejmuje zwykły katalog lub samodzielny .git directory pod rootem.
  Git metadata poza rootem, linked worktree i commondir wymagają osobnego
  adaptera z jawnymi dodatkowymi grantami; obecny adapter ich nie odblokowuje.
  Wersjonowane host grants, ABA revocation/regrant w legacy native grants,
  remote host bindings i guardian działającego procesu nadal otwarte.
- Dodano testy store grant intersection/revocation/restart/CAS/root replacement,
  settings change, Git metadata i symlink retarget oraz HTTP config/revoke/path
  privacy. Weryfikacja wykonawcza w toku. Nie uruchomiono jeszcze nowego ACP
  actora ani AHP; chat/workflow capabilities pozostają false.
- `--suite server` **143 relay PASS, 1 identity PASS, 4 session-runtime PASS**, 0
  failed; 2 ignored browser tests (nie wykonywano ich w tym backendowym przebiegu).
  Snapshot `37329d729b5e6eb57bc8d522eee7b13df3ffdbf20f7fe6441e4216aa8b777097`.
  Kompilacja 1m33s, relay tests 41.78s, journal 0.07s. Zalicza nową migrację,
  dotychczasowe migracje i stary history/transcript/native API test suite,
  policy/grant isolation, stale CAS, root replacement i HTTP binding API.
- Aktualizacja statusu E4.2 oznacza częściowy lokalny adapter, nie ukończony
  multi-host routing ani proces ACP. Dokumenty architektury i operatora
  odzwierciedlają aktualne granice. Końcowy Clippy w toku.
- Końcowy Clippy **PASS**, snapshot
  `2ae313d2190af3509acdb9ed1a79f5574ebe63b0e69b01004c4a8d6770130845`:
  relay/session-runtime/identity/domain/protocol, all-targets, offline/locked,
  no-deps, -D warnings, 1m05s. Od snapshotu testów zmieniła się tylko instrukcja
  operatora, bez zmian kodu Rust. Formatowanie/whitespace/112 Markdown **PASS**.
  Nowy adapter nadal nie oznacza ukończonego E4.2 ani niezależnego procesu ACP.
  Brak commit/push, zmian baz operatora, realnego tailnetu lub kont/harnessów.

## 2026-11-11 — E3: eksperymentalny actor ACP należący do serwera

- Proces/grupa ACP ma lifecycle niezależny od obserwatorów. Nowy actor jest
  odrębnym opt-in od legacy /acp; nie zmieniono ownership starego API v1.
  Wymaga store_transcripts, jawnego bindingu, aktualnych grantów i sandboxu
  poza kontrolowanymi fixture testowymi. Flag experimental_interactive_runtime
  domyślnie false; chat/workflow capabilities nadal false.
- Initialize/session-new/set-config-option weryfikują model i mode; zapis
  harness_ready zawiera remote session id, binding revision, profil i limity.
  FIFO przechodzi przez dispatching przed zapisem do stdin. Serial actor zapisuje
  bounded agent updates. end_turn kończy turę protokołu, nie dowodzi wykonania
  zadania/publikacji. Przerwanie lub niepewny błąd daje interrupted/unknown,
  bez automatycznego ponowienia promptu.
- Guardian kontroluje device/OIDC/host policy, logiczny/native grant, binding,
  tożsamość checkoutu i settings. FS kontrole poza Store lock w ograniczonej puli;
  kontrola authority po powrocie z FS. Limity: 8 aktorów, 1 MiB/frame, 4096
  frames/turn, 1 KiB–4 MiB output/turn, 1–1800 s/turn, 1 h lifetime/60 s idle.
  To nie są limity kosztów modeli. ReadOnly blokuje start do czasu adaptera
  kernelowego. Client tools/permission requests odmawiane do implementacji zgód.
- RAII zabija grupę procesów i zapisuje interrupt także po utracie authority.
  Stop wymaga właściciela i bieżącej epoki. Shutdown SIGINT/SIGTERM blokuje nowe
  starty i zatrzymuje aktorów; crash/SIGKILL wymaga startup recovery.
- Cookie/origin protected REST: create, metadata bez raw promptów, stop.
  Brak publicznego enqueue/prompt endpointu; to bootstrap/diagnostyka, nie polling
  UI ani własny transport podszywający się pod AHP. AHP SDK/wire projection,
  conformance, browser reconnect/live chat, approvals, koszty i odbiór prawdziwego
  harnessu pozostają otwarte. E3 i cały plan do E10 nie są ukończone.
- Pierwszy przekrój fixture (fragmentowane ramki ACP) **149 relay PASS, 1 identity
  PASS, 4 session-runtime PASS**, 0 failed, 2 browser tests ignored. Snapshot
  `c0ae65d9ee60f34fb15f5a68bf8ac81748ada71230e63ad57b6d5009374ec1c6`.
  Testy: dwóch promptów FIFO po odłączeniu wszystkich odbiorców, revoke device,
  binding/settings/root replacement, permission/scope/model/oversize rejection,
  time budget i kill potomków procesu. To test observera wewnętrznego, nie AHP
  browser E2E. Nie uruchomiono prawdziwego agenta/modelu ani realnych integracji.
- Dodano następnie shutdown, HTTP bootstrap/izolację/metadata/CAS stop,
  ReadOnly refusal i brak pustych invalidacji przy bezczynnej kolejce.
  Przebieg `7c6614eb3127c3d9af2e583f72335a69c3898e8ae6b1d5157fe6d3e147faa4d7`
  zatrzymano przed testami po korekcie mutacji Arc<Config> w nowym teście HTTP.
  Końcowa regresja aktualnych źródeł w toku.
- Końcowy `python3 -I scripts/test_web_sandbox.py --suite server --docker-host
  unix:///Users/corapoid/.colima/default/docker.sock --tmp-root
  /Users/corapoid/git_priv/dunlin/target/web-checks`: **152 relay PASS, 1 identity
  PASS, 4 session-runtime PASS**, 0 failed, 2 browser tests ignored (nie były
  wykonywane w tym backendowym przebiegu). Snapshot
  `8d481bff378b74f0a2169f6a2ca6a0cd6dcec282f0867f3364f08c0f0314bd27`.
  Kompilacja 1m38s, relay tests 45.11s. Dziewięć nowych testów actora zaliczonych.
  `cargo fmt --all --check`, whitespace i 112 Markdown: **PASS**.
- Ponownie odczytano oficjalny przypięty kontrakt AHP v1.0.0 (transport,
  lifecycle, subscriptions, versioning). Zapisano wymagania następnego
  przekroju w architecture.md: server-wide serverSeq/outbox zamiast podmiany
  per-session sequence, typed projection/reducers, atomic snapshot/subscribe,
  reconnect oraz durable client action deduplication. Nie ogłoszono tych
  wymagań jako wdrożonej implementacji AHP. Końcowy Clippy w toku.
- Pierwszy Clippy końcowego snapshotu zgłosił wyłącznie dwa collapsible_if
  (cleanup registry oraz test procesu zombie). Uproszczono warunki do let-chain
  bez zmiany zachowania; ponowiony Clippy w toku. Regresja wykonawcza powyżej
  dotyczy kodu sprzed tych dwóch zmian stylistycznych.
- Końcowy Clippy **PASS**: relay/session-runtime/identity/domain/protocol,
  all-targets, offline/locked, no-deps, -D warnings, 1m03s. Snapshot
  `d455a86969802f979cf8f119d7b7f7279da56f8eca7cc6e92ba4d2a8132fb54c`.
  Różnica kodu względem zaliczonych testów to wyłącznie powyższe dwa let-chain.
  Formatowanie/whitespace/112 Markdown **PASS**. Brak commit/push, zmian realnego
  tailnetu, poświadczeń, baz operatora lub wywołań płatnych usług.

## 2026-11-11 — E3.4: trwały porządek źródłowy między sesjami

- Poprzedni turn: postęp — server-owned actor ACP, 152 relay/1 identity/4 runtime
  PASS i Clippy PASS. Obecny worktree potwierdza brak implementacji AHP wire;
  nie potraktowano starego katalogu ani broadcast observera jako odbioru AHP.
- Migracja 18 tworzy trwały clock i referencyjny stream index dla dzienników
  interactive_sessions. Trigger SQLite nadaje wspólny porządek w tej samej
  transakcji co zdarzenie i zmiana kolejki. Błąd indeksu/clocka i przepełnienie
  JS-safe integer wycofują całą operację. Nie kopiuje się treści do outboxu.
- Dla v17 brak historycznego porządku cross-session: backfill ustala replay_floor
  i bazę snapshotu; niższe kursory wymagają snapshotu. Nie wymyślamy kolejności
  starych sesji. Identity strumienia i clock przeżywają reopen.
- Wewnętrzne snapshot_stream/replay_stream ponownie autoryzują każdą prywatną
  subskrypcję. Snapshot ma jeden SQLite read cut. Replay filtruje w SQL przed
  limitami, zwraca wszystkie pasujące zdarzenia do high-water mark albo
  SnapshotRequired. Nie zwraca częściowej strony jako pełnego reconnectu.
  Limity: 32 subscriptions, 128 events i 1 MiB serializowanego budżetu;
  zbyt duży snapshot wymaga adaptera paginacji. Kursor nie stanowi authority.
- To log źródłowy, nie AHP actions, publiczny endpoint ani ukończony reconnect.
  ACP event może wymagać wielu typed actions, które potrzebują własnego trwałego
  porządku. Projekcja/SDK/reducers, global wire sequencing, snapshot-subscribe,
  durable client dedupe i rzeczywisty WS/browser conformance nadal otwarte.
  Chat/workflow capabilities i eksperymentalny actor default pozostają false.
- Początkowy `--suite sessions`: **9 PASS**, snapshot
  `90ef24547f02085e125a78042d1e39120fb91cb0571ba89fa0f270730010d4e8`.
  Następnie dodano dokładny serialized budget, test JSON escaping/clock exhaustion
  oraz rzeczywistą migrację Store v17→v18 po ponownym otwarciu SQLite. Pełna
  regresja aktualnych źródeł w toku. Nie zmieniono baz operatora ani realnych
  kont/agentów/tailnetu. Bez commit/push; cały plan do E10 pozostaje aktywny.
- Końcowy `python3 -I scripts/test_web_sandbox.py --suite server --docker-host
  unix:///Users/corapoid/.colima/default/docker.sock --tmp-root
  /Users/corapoid/git_priv/dunlin/target/web-checks`: **153 relay PASS, 1 identity
  PASS, 10 session-runtime PASS**, 0 failed; 2 browser acceptance tests ignored
  (nie wykonywano ich w tym backendowym przebiegu). Snapshot
  `d77c9370c6c19e8e94061a85e6549a488f5e1ba17a206814ae7949a983c960a5`.
  Relay tests 45.77s, runtime 0.17s. Potwierdzono także legacy v5 migration,
  future-version refusal, actor FIFO/revoke/stop/shutdown i browser-cookie scope.
- Clippy **PASS** dla tego samego snapshotu: relay/session-runtime/identity/
  domain/protocol, all-targets, offline/locked, no-deps, -D warnings, 1m09s.
  Formatowanie/whitespace/112 Markdown **PASS**. Nie zmieniano kodu po testach.
- Do badania następnego przekroju pobrano oficjalny source archive v1.0.0 do
  `target/ahp-reference/` (poza kodem dystrybucji). SHA256 archiwum
  `9805d962cbab1d56579e458a000aa7fa98c906eb16971dd484c9bb2a50aaf5d1`.
  Zawiera upstream Rust ahp-types/ahp reducers oraz TS types/reducers; nie dodano
  ich jeszcze jako dependencies ani nie zaliczono ich conformance.
  Restore tej samej bazy wymaga osobnej generacji/snapshotu w E10; samo UUID
  wewnątrz backupu nie wykrywa późniejszego rozgałęzienia historii.

## 2026-11-11 — E3.3: trwała projekcja ACP do typowanego AHP chat

- Poprzedni turn: postęp — wspólny stream źródłowy i migracja 18, 153 relay /
  1 identity / 10 runtime PASS. Zweryfikowano worktree i przypięte oficjalne
  source archive AHP v1.0.0; sam stream nadal nie był wire actions.
- Dodano niezmienione źródła ahp-types/ahp 1.0.0 z MIT i oryginalnymi manifestami.
  Normalizacja manifestów usuwa workspace inheritance/dev examples. Dokładne
  piny i root patches podlegają istniejącemu verify_vendor/provenance.json,
  z archive/tree/per-manifest SHA256; 12 vendor trees PASS. Offline lock update
  dodał tylko dwa SDK packages, bez odświeżania wersji istniejących dependencies.
- Migracja 19: odrębny AHP clock, ChatState i action outbox. Journal w tej samej
  transakcji buduje typed StateAction i stosuje oficjalny reducer. Odmowa/no-op
  reducera przerywa operację; nie ogłasza się pustego sukcesu. Każda akcja ma
  osobny globalny serverSeq; text chunk tworzący część to responsePart + delta.
- Pending queue/dispatching/terminal, text, reasoning i raporty tool_call/update
  mają mapowanie i zachowują kolejność. Diff/resource content pozostaje w
  structuredContent z pełnym raportem ACP, bez wymyślania wire variants lub
  hostowanych artefaktów. Standardowe not-needed oznacza brak dodatkowego AHP
  client confirmation dla już raportowanej operacji; meta confirmationEvidence
  pozostaje agent-report-only. Nie tworzy się user-action/setting. Projekcja
  nigdy nie wykonuje narzędzia; actor nadal odmawia client permission requests.
- Unknown/recovery → chat/error, resumable=false, bez powtórzenia promptu.
  Actor zapisuje elapsed milliseconds z monotonic clock; unknown duration ma
  jawny meta flag, nie wylicza się go z UTC. Nowe zdarzenia mają recorded_at UTC;
  brakujące historyczne timestampy oznaczane są jako imported.
- Backfill w partiach 64 i per-record savepoint ustala replay_floor. Stare
  nieobsługiwane treści pozostają w źródle, a projekcja sesji odmawia odczytu
  jako unavailable. Nie blokuje się całej bazy ani nie zwraca pustego chatu jako
  kompletnego importu. Snapshot chat/fromSeq czytane atomowo; odczyt akcji to
  wewnętrzna strona per-session, nie pełny reconnect response.
- Początkowy SDK runtime: **10 PASS** bez nowych testów projekcji, snapshot
  `c31a0911e9c9103f689498764c4f1eb01f2e26d9ea55548462dfee6b73ef6849`.
  Nowe testy dały następnie 14 PASS/1 FAIL: rollback savepoint unieważniał
  otwarty SELECT cursor migracji. Zmieniono fold na zamknięte, bounded batches;
  poprawiony przebieg **16 runtime PASS**, snapshot
  `ddb3424ea07b07d6fa7a4a3b855ae74eac04061edfe6c2dceb15aa3a64caf5eb`.
  Po nim dodano test starych timestampów, floor i typed Snapshot roundtrip.
- Testy projekcji sprawdzają JSON→SDK ActionEnvelope, redukcję i odtworzenie
  drugiego odbiorcy z kursora po reopen SQLite, text/reasoning/text order,
  monotonic global action order, revocation/private ownership, rollback
  source/queue/clock/idempotency, failed tool i brak fikcyjnej human approval,
  unsupported legacy import oraz brak complete/replay dla unknown outcomes.
  To model odtwarzania stanu, nie rzeczywisty WebSocket/browser reconnect.
- Cały E3 pozostaje otwarty: root/session summaries, nowy ACP content, client
  origin/reconciliation/dedupe, paginacja, AHP WS i conformance z SDK/browserem.
  Chat/workflow capabilities pozostają false. Pełna regresja serwera w toku.
  Bez realnych integracji/modeli/poświadczeń/tailnetu, commitów i pushowania.
- Pełny server build snapshotu
  `b1e9685ee50faba6a503f7feb92d4f6e4663afabff8b148e1beb9f2aa7a4dd40`
  zakończył rustc SIGKILL podczas budowy relay test binary, bez uruchomienia
  testów; nie oznaczono go jako PASS. Lokalny Docker VM ma ok. 4 GiB i inne
  aktywne obciążenia. Ponowiono ten sam kod z codegen-units=1, limitem 2048 MiB,
  niezmienionym bounded tmpfs/network-none/readonly/nobody/cap-drop i LLD.
  Launcher wypisuje teraz memory.peak/events po głównej komendzie dla dowodów
  ograniczeń pamięci. 4 regression tests launchera PASS. Regresja w toku.
- Migracja jest testowana jako zamknięcie starego Store i uruchomienie nowego.
  Ochrona upgrade przed równoległym starym writerem oraz backup/restore fork
  pozostają bramką E10.4/E10.5; nie zaliczono ich samym testem v17→v19.
- Próba 2048 MiB/codegen=1 również zakończyła się SIGKILL: memory.peak
  1817686016, memory.max hits=0, oom=0, oom_kill=1. To wskazało OOM wspólnej VM,
  a nie przekroczenie limitu samego kontenera. Utworzono osobny lokalny profil
  Colima dunlin-checks (4 GiB RAM, 2 CPU, dysk danych 8 GiB/root 4 GiB),
  autoActivate=false, sshConfig=false, ssh-agent=false, port-forwarder=none.
  Montuje tylko readonly target/web-checks i .cargo/registry. Skopiowano cached
  obraz Rust bez registry pull. Domyślnego profilu i jego usług nie zatrzymywano.
- Końcowy `--suite server --docker-host
  unix:///Users/corapoid/.colima/dunlin-checks/docker.sock --tmp-root
  /Users/corapoid/git_priv/dunlin/target/web-checks`: **PASS** — relay suite
  (153 testy, 2 browser acceptance ignored), identity (1), session-runtime (17),
  0 failed. Snapshot
  `949b94132bcbd339fa90fd88425fcf66b3b4f9bc6542fcee26c2f75940d69219`.
  Kompilacja 1m57s, runtime tests 4.93s. 3072 MiB/1280 MiB tmpfs, LLD,
  codegen-units=1, peak 2651779072 bytes, oom/oom_kill=0.
- Końcowy Clippy **PASS** dla tego samego snapshotu: relay/session-runtime/
  identity/domain/protocol, all-targets, offline/locked, no-deps, -D warnings,
  1m18s, peak 1208905728 bytes, oom/oom_kill=0. 6 vendor verifier tests,
  4 launcher tests, 12 vendor tree hashes, format/whitespace/112 Markdown PASS.
  Bez zmian kodu po testach. Pełny AHP transport/browser conformance i cały
  plan do E10 pozostają nieukończone. Profil testowy jest zatrzymywany po
  weryfikacji; zachowany cache pozwala ponowić kolejne testy bez provisioning.
- Potwierdzono cleanup: colima list pokazuje dunlin-checks Stopped i default
  Running. Osobny profil można wznowić `colima start --profile dunlin-checks
  --activate=false`; testy kierować jawnie na jego docker.sock. Nie zmieniono
  domyślnego kontekstu ani usług aplikacyjnych. Cache profilu pozostaje lokalny.

## 2026-11-11 — AHP session channels, transport i panel obserwatora

- Migracja 20: typed SessionState, kanały session/chat we wspólnym porządku
  AHP oraz trwałe device-scoped negocjacje klienta (TTL 8h, bounded records).
  Journal commit source/chat/session jest atomowy; invalidation następuje po
  commit. Backfill zachowuje stare źródła/quarantine i replay floor. Sprawdzono
  także stare terminalne projekcje w przestrzeni zarezerwowanej na recovery.
- `/v1/web/ahp`: prawdziwy WebSocket opt-in obu flag, cookie/Origin/Host,
  fresh private-owner/project/device/OIDC/host authority. Wydane typed
  initialize/subscribe/reconnect/listSessions, root/session/chat snapshots,
  root summary notifications, 32 kanały, 128 replay actions/900 KiB, ograniczenia
  ramek/rate/timeout/socket permits. Nieudany RPC nie zmienia Connection i nie
  gubi pending deltas starej subskrypcji. Opaque paging cursors per connection.
- Root opisuje adapter obserwatora dunlin-acp, bez obietnicy executable model
  discovery. Wszystkie metody zapisu pozostają niedostępne. `chat=false`,
  `workflow=false`; discovery ma osobną capability `ahp_observer` i endpoint.
- React „Sesje”: rzeczywisty AHP WS, streaming/replay/snapshot, read-only
  transkrypt text/reasoning/tool/error, świeża lista po reconnect, brak REST
  pollingu. Po pairing pojedynczy GET session. Revocation czyści dane; late
  callbacks i snapshot poprzedniej sesji nie nadpisują nowego wyboru. Brak
  baseline po przerwanym subscribe powoduje ponowny odczyt snapshotu.
- ACP rawInput jest pełną aktualizacją: zachowywana jest ostatnia wartość,
  przekazywana do standardowego inline toolInput. Nie skleja się kolejnych
  kompletnych JSON-ów. UI usuwa partialInput po ready/complete i pokazuje
  toolInput; ContentRef wyświetla wyłącznie jako tekst bez pobierania.
- Równoległy podział autoryzowany przez użytkownika: frontend, fixture browser
  oraz read-only review. Review wykrył utratę baseline, utratę pending actions
  przy failed subscribe i rawInput concatenation; dodano poprawki/regresje.
- Rzeczywiste testy WS obejmują dwóch obserwatorów, oficjalny klient SDK,
  restart Store/AppState (snapshot nowego bootu), replay overflow, paging,
  wersje/error codes, revocation i prywatność między administratorami.
  Browser fixture steruje tylko źródłowym Journal przez stdio Rust↔Node;
  produkcyjny router, cookie, WebSocket i React pozostają rzeczywiste.
- Nie zamknięto E3: wydany SDK nie agreguje statusu parent SessionState po
  chatUpdated. Adapter oblicza go i emituje root summary updates, ale generic
  session-only SDK mirror może różnić się od świeżego snapshotu. Ponadto brak
  client dispatch/origin/reconciliation, approvals, dużej historii/fetchTurns,
  pełnej ACP→browser ścieżki z rzeczywistym harness oraz odbioru operatorskiego.
- Formalny postęp listy E0–E10: **12/68 = 17,6% zamkniętych punktów**; liczne
  częściowe implementacje nie są dodatkowym formalnym zaliczeniem.

### Weryfikacja tej iteracji

- Pierwszy compile nowych testów wymagał `use ::ahp as sdk` z powodu modułu o
  tej samej nazwie. Clippy wykrył trzy collapsible_if; poprawiono. Nie oznaczono
  tych nieudanych przebiegów jako PASS.
- Server snapshot `a3f4bfbc79143414cc7a1df6dd357cfb4a0bf19c54b8a4500d97bccf5853d8f2`:
  **158 relay, 1 identity, 20 runtime PASS**, 2 browser ignored.
- Rozszerzony snapshot `d7caef037d1c446eb8a4bb845284b030283574b81067ec243fb2b5b414813d8f`:
  **160 relay, 1 identity, 21 runtime PASS**, 2 browser ignored; peak
  2789449728 bytes, oom/oom_kill=0. Po tym dodano failed-subscribe regression,
  discovery i toolInput; ich końcowa weryfikacja opisana poniżej po wykonaniu.
- Frontend w izolowanym Node 22: **28 PASS**, source
  `1d6c770347409d28450ac63b00fd15120938a5b3b56a33d929c82bab01b89df7`;
  TypeScript **PASS**, source
  `d95ecc85412bc451a5ab1811952ef69ab6945ac181dd7194a80d62dd7e843acd`.
  Launcher frontend odkrywa wszystkie `tests/*.test.ts`, zgodnie z npm test;
  wcześniejsza jawna lista pomijała nowy plik AHP i została poprawiona.
- Reused dedykowany dunlin-checks, offline cached obrazy Rust/Node/browser,
  readonly source/registry, nobody, network-none, bounded scratch i pamięć.
  Bez instalacji zależności, zmian default profilu/usług i prawdziwych danych.
- Server snapshot `1bb1bfe4ec218febea1d6f243b5bb6bf5913c70aed0368f7803fca9d483dde1d`:
  **161 relay, 1 identity, 21 runtime PASS**, 3 browser ignored; peak
  2688184320 bytes, oom/oom_kill=0. To przebieg przed ostatnią korektą rawInput.
- Końcowy `--suite browser` snapshot
  `fe39d456b3792d3a84e86f0271b4ac77cf6403c254933263658e0e235b76eb9c`:
  produkcyjny build Vite **PASS**, następnie **161 relay + 1 identity + 21
  runtime PASS**, w tym regresja rawInput pending A→B→ready bez inputu.
  Dodatkowo wszystkie **3 Chromium acceptance PASS**: pairing/catalog/team,
  lokalny fixture OIDC z rzeczywistym PKCE/RS256 oraz nowy AHP observer.
  AHP potwierdza snapshot/stream/reconnect/completion, private user isolation,
  tylko read methods, mobile overflow i device revoke z usunięciem transkryptu.
  Kompilacja 2m05s, relay 47.98s, runtime 5.76s, browser 10.93s; peak
  2846072832 bytes, oom/oom_kill=0. Brak mockowanego WebSocket lub browsera.
- 13 CI workflow tests, 6 vendor verifier tests, 4 launcher tests oraz 12
  vendor tree hashes **PASS**. Test fixture nie zastępuje prawdziwego IdP,
  prawdziwego harnessu/modelu, tailnetu ani manualnego odbioru operatorskiego.

### Następny pion E3

Tekstowe queued `dispatchAction` do istniejącej własnej sesji z działającym
actorem: trwały ledger `(device,clientId,clientSeq)` z digestem całego payloadu,
client message id oddzielony od turn id, wspólna transakcja
source/queue/projection/origin/ledger oraz typed accepted/rejected echo.
Nie wolno zapisywać ledger osobnym commit po dzisiejszym `enqueue()`.
Dedupe nie może wygasać wraz z rekordem negocjacji klienta. Przed publicznym
promptem potrzebne testy identycznego retry/reopen, konfliktu payloadu,
rollback, authority/actor gates i reconciliation optimistic state.
Snapshot sam nie rozstrzyga losu wiadomości skonsumowanej przez actor;
protokół retry/reconnect należy jeszcze sprawdzić względem przypiętego SDK.
- Sprawdzono przypięte źródła AHP v1.0.0: `docs/guide/reconciliation.md:81`
  wymaga wyczyszczenia predykcji pendingActions po reconnect; Swift client README
  opisuje osobny outbox i retry z niezmiennym clientSeq. To rozdzielne warstwy:
  baseline/predykcje odbudować, osobny pamięciowy outbox rozliczyć przez origin
  i dopiero ponowić niepotwierdzone wpisy. Origin rozliczać przed filtrem starych
  serverSeq, ale nie stosować starej akcji ponownie do świeżego snapshotu.
  Trwały ledger/re-emit jest polityką Dunlin; nie wymyśla się nowego wire ACK.
- Końcowy Clippy **PASS**: snapshot
  `b46eba91950707c576284bd756b454f327afd5fb8536cb45bab8def9b147d3db`,
  relay/runtime/identity/domain/protocol all-targets offline/locked no-deps
  `-D warnings`, 1m21s, peak 1216999424 bytes, oom/oom_kill=0.
  Różnica względem browser snapshot obejmuje dokumentację; kod wykonawczy
  po końcowym browser E2E nie był zmieniany. Format i whitespace PASS.
  Cały E0–E10 pozostaje aktywny i nieukończony. Bez commitów i pushowania.
- Cleanup potwierdzony `colima list`: dunlin-checks Stopped, default Running.
  Zachowano cache; nie zatrzymywano usług użytkownika z domyślnego profilu.

## 2026-11-11 — AHP queued text dispatch i reconciliation

Poprzedni cykl zmienił kod i dostarczył nowe dowody (observer/browser PASS),
więc jest postępem, nie oczekiwaniem ani powtórzeniem statusu. Obecny cykl
kontynuuje pełny cel E0–E10, bez zamykania częściowo wykonanych punktów.

- Migracja 21 dodaje origin/rejection w action log, trwały outcome ledger
  device/clientId/clientSeq i mapowanie client message ID→server turn ID.
  Payload hash obejmuje typed kanał/akcję. Dedupe poprzedza aktualny actor gate,
  ale następuje po fresh owner/CreateSession auth. Przyjęcie kolejki, source,
  obie projekcje, origin oraz ledger mają jeden commit. Brak automatycznego
  expiry ledgeru (8192 global/1024 device); błędy finalnego insert cofają całość.
- `dispatchAction` jest notification, nie RPC request. Obsługiwany wyłącznie
  nowy queued plain user text <=32KiB, bez override/attachments/meta. Root
  waliduje raw JSON przed typed deserializacją; runtime ponownie sprawdza pełną
  minimalną semantykę. Inne typed akcje mają durable rejection bez redukcji;
  malformed/auth errors kończą połączenie. Duplicate payload zwraca dawny
  envelope bez nowej tury; zmieniony payload z tym samym seq jest konfliktem.
- Existing actor gate sprawdza starter device, owner, host/OIDC, harness,
  binding/native grant, shutdown/stop i pełny lease/authority stamp. Gate nie
  wczytuje transkryptów. Actor nadal sprawdza checkout i guardian przed ACP.
  Dispatch nie uruchamia nowego procesu, nie zmienia tożsamości actora i nie
  powtarza niepewnej tury po recovery. Accepted oznacza queued, nie wykonanie.
- Root summaries mają per-session/user `_meta["dunlin.queueText"]`;
  canonical ChatState jest full, a composer wymaga również rzeczywistego gate.
  Brak gate oznacza nadal widok tylko do odczytu. Zapisano full przy migracji
  wraz z replay_floor, aby stary read-only baseline nie udawał aktualnego.
- React ma memory-only outbox exact identities, confirmed base i optimistic
  overlay. Origin rozliczany przed filtrem serverSeq (także stare ACK po nowszym
  snapshot). Rejection nie przechodzi reducer, przywraca draft. Reconnect
  odbudowuje baseline i ponawia identyczne unacknowledged commands. Unknown
  identity z outboxem zatrzymuje auto retry; reload nie wznawia wysyłania.
  Offline zachowuje draft i blokuje send. Frontend sprawdza UTF-8 i pełny
  encoded JSON frame, zachowując tekst i clientSeq przy przekroczeniu limitów.
- Review wykrył trwałe capacity/conflict mapowane początkowo na retry1013.
  To mogło zapętlać reconnect; teraz są terminalne, a tylko strata replay
  wymusza nowy snapshot. Podejrzenie nadmiarowego replay było false positive:
  istniejący limit900KiB liczy całe envelope i mieści wrapper w FRAME1MiB.
- Równoległy podział: runtime ledger/projection, frontend i browser fixture;
  root wykonał transport, Store migrację, actor gate oraz testy WS/SDK.
  Browser fixture używa prawdziwego HTTP bootstrap z cookie, rzeczywistego
  AHP i procesu ACP (kontrolowany lokalny peer, bez modelu/dostawcy).
- Pierwszy browser snapshot
  `ddd6ed46790b9bc25fa7abe6a3c44a784ef87a8e5b746af99a4a5a67823c1e10`
  zbudował produkcyjny frontend, lecz relay compile wykrył brak klamry po else
  wewnątrz tokio select. Poprawiono; ten przebieg NIE jest PASS.
- Izolowany frontend **34 PASS**, source
  `5d2d3bbe4e81b33ce8b6026103b5ce851bfe395ab34202a3f1a1ea8194e1f585`.
  TypeScript **PASS**, source
  `0d6d2a016ea86071fcd39770eeeb349377b645def0ce7a94aef4269ec452436c`.
- Pełne server/browser/Clippy po ostatnich poprawkach: weryfikacja w toku,
  wynik zostanie zapisany po zakończeniu. Nie zaliczono pełnego E3: create UI,
  steering/cancel/upsert/approvals, kontekst/diff/artefakty, duże transkrypty,
  aggregate status conformance SDK i prawdziwy harness acceptance otwarte.
- Drugi snapshot `e5d15c4bfb65f2a820e33a14a560a7cae3fb4928c24c002236df225808770957`
  zbudował relay, lecz compile runtime testów wykrył brak jawnego drop(Journal)
  przed zamknięciem pożyczonej Connection w teście reopen. Dodano drop(j);
  brak PASS tego przebiegu. To błąd testu, nie dowód wykonania jego asercji.
- Trzeci snapshot `b8c6ed3721a18015682377b454042001b428ecb80d49fc27df533315de97caa6`
  skompilował cały kod i przeszedł relay, lecz runtime miał **27 PASS/1 FAIL**:
  test historycznych timestampów odtwarzał tylko schemat do v20, po czym wołał
  reader v21 wymagający origin/rejection. Test migracji teraz wykonuje również
  DISPATCH_SCHEMA/backfill_dispatch przed odczytem (jak rzeczywisty Store startup).
  Nie dodano runtime obsługiwania nieukończonego upgrade jako aktualnego schematu.
  Chromium tego przebiegu nie uruchomiono, bo prerequisite runtime nie przeszedł.
- Końcowy browser snapshot
  `b96d8299d4b80a0d1e3f8be082b9a71732e3252170a464aa959de80fcd6bb9f2`:
  **PASS** — Vite produkcyjny build, **163 relay + 1 identity + 28 runtime**,
  następnie **4 Chromium acceptance**. Kompilacja 2m16s, relay 49.66s,
  runtime 6.48s, browser 17.14s; peak 2841620480 bytes, oom/oom_kill=0.
- Nowy browser scenariusz gubi dokładnie jeden rzeczywisty accepted envelope
  w native WS message delivery, przepuszcza późniejszy serverSeq i zrywa socket.
  UI wykonuje faktyczny reconnect oraz >=2 identyczne dispatchAction sends
  (deepEqual clientSeq/channel/message ID/payload). ACP received-prompts ma
  dokładnie jeden wpis, a Journal dokładnie jedną Completed turę. Potem proces
  kończy turę podczas kolejnego rozłączenia; reconnect odtwarza completion bez
  ponownego dispatch, a revocation usuwa transkrypt i blokuje cookie.
  Nie podmieniano payloadów ani implementacji WebSocket/routera; ACP peer
  pozostaje jawnie fixture bez modelu.
- Siedem nowych runtime tests obejmuje durable retry/reopen/message mapping,
  cross-channel seq identity/capacity, queuefull/unsupported/rejection replay,
  rollback projection i final ledger insert oraz lease/stamp/stop. Nowe testy
  prawdziwego WS i oficjalnego SDK potwierdzają typed rejection i dwóch observerów.
  Odtworzenie historycznych timestampów obejmuje pełny aktualny upgrade.
- Odbiór wykorzystuje kontrolowany ACP peer, nie komercyjny model ani prawdziwy
  provider login. OIDC/browser nadal mają lokalny IdP fixture; realny IdP/tailnet,
  keyring, harness i akceptacja operatorska nie zostały zaliczone.
- Końcowy Clippy **PASS dla tego samego snapshotu b96d8299…**:
  relay/session-runtime/identity/domain/protocol, all-targets, offline/locked,
  no-deps, `-D warnings`, 1m30s; peak 1216999424 bytes, oom/oom_kill=0.
  Format/whitespace i 112 Markdown **PASS**; 4 launcher regression tests oraz
  12 vendor tree hashes **PASS**. Kod po tych testach nie był zmieniany.
- Formalna lista pozostaje **12/68 zamkniętych punktów**: dodatkowo wykonano
  część E3.3–E3.5/E3.7, ale nie zamknięto pozostałych operacji ani conformance.
  Cały cel do E10 pozostaje aktywny. Bez commitów, pushowania, zmian tailnetu,
  keyringu, prawdziwych baz użytkownika i komercyjnych wywołań modeli.
- Cleanup potwierdzony: dunlin-checks Stopped, default Running. Cache zachowany.

## 2026-11-11 — E3: kolejka i anulowanie z potwierdzeniem ACP

Poprzedni cykl był postępem: dodał trwały dispatch i dowód utraty ACK bez
powtórzenia promptu. Ten cykl rozszerza ten sam cel E0–E10, nie zmienia jego zakresu.

- Migracja 22 / CONTROL_SCHEMA: fenced cancellation intents requested/confirmed/
  unconfirmed. Nowy dispatch_action zachowuje wspólny durable ledger i wrapper
  dispatch_text; queued upsert/remove/reorder oraz active turnCancelled mają
  atomic source/SQL/projection/origin/outcome. Client duration nie jest dowodem:
  root mierzy elapsed monotonicznym zegarem actora lub jawnie oznacza unknown.
- Edycja nie zmienia pozycji. Usunięta queued wiadomość pozostaje SQL tombstone
  Cancelled, bez tworzenia fikcyjnej historycznej tury. Nowy enqueue używa
  max(position)+1. Canonical reorder zawiera pełną kolejkę (requested known,
  potem missing w poprzedniej kolejności), omijając potwierdzony błąd SDK 1.0.0
  sortującego missing IDs leksykograficznie. Legalny client_control NoOp może
  dostać trwały outcome. Nie modyfikowano vendor source.
- Cancel natychmiast kończy host-side processing chatu, lecz SQL pozostaje busy.
  Actor sprawdza intent przed promptem i podczas oczekiwania. Wysyła pojedynczy
  ACP session/cancel, czeka na oryginalny prompt response do min(existing
  deadline,now+5s), nie pobiera następnego promptu przed settlement.
- Atomowe terminal transition vs cancel: jeśli cancel wygra, późniejsza odpowiedź
  agenta settles SQLCancelled bez drugiego terminal AHP; jeśli complete wygra,
  stale cancel zostaje rejected. Settlement wykorzystuje recovery reserve, aby
  ACK nie został odrzucony wyłącznie przez pełny zwykły quota.
- Niewspółpracujący peer powoduje kill process group, runtime Unknown i zachowanie
  queued work bez wykonania. Interrupted/unconfirmed session ma Error status
  nawet bez activeChat. Metadata/UI rozróżniają pending i unknown outcome.
  Cancel nie oznacza cofnięcia tool side effects. Późne raw updates pozostają
  w źródle i nie redukują anulowanego chatu; rebuild zachowuje pre-cancel historię.
- Lokalny dowód not_sent ma prywatny typ RequestReply. Usunięto początkowy pomysł
  znacznika JSON, który peer mógłby sfałszować. Nowy kontrolowany peer celowo
  wysyła takie nieufne pole przy normalnym end_turn, a drugi prompt mimo tego
  kończy się Completed. ACP nie może wygenerować lokalnego dowodu dostarczenia.
- Frontend: edit/save/remove/up/down i cancel active, capability controlTurns,
  immutable generic outbox, optimistic controls, late ACK reconciliation,
  odrzucenia i reconnect bez zmiany identyfikatora. Nie zakłada, że każda akcja
  ma message.text. Niezakończone narzędzia terminalizuje jak SDK cancelled/skipped;
  UI nie pokazuje ich po cancel jako nadal pracujących.
- Równoległy runtime/frontend/browser; root wykonał transport, actor, migrację
  i dwie regresje prawdziwych procesów cooperative/noncooperative. Wspólny
  tests/fixtures/acp/control_peer.py ma jedno eventloop, bez modelu/narzędzi/sieci.

### Faktyczna weryfikacja

- Pierwszy Clippy snapshot `50c59646ac806ba93abf212e30338cdcbad0981b804b2b97015547d5127806fd`
  wykrył nieużywany import StateAction po refaktorze; import przeniesiono do testów.
- Drugi Clippy snapshot `8dcac265b04c6728528f884e989136748d9bba41cb8f644f5400472b1b7f662f`
  skompilował kod relay, ale test reopen miał brak drop(j) przed drop(Connection).
  Poprawiono. Tych dwóch przebiegów NIE oznaczono jako PASS.
- Wcześniejszy frontend 39 PASS, potem poprawka terminalizacji narzędzi:
  końcowy izolowany Node 22 **41 PASS**, source
  `01996f4f779072ee3d181aa6fca4922a80f217798d1a7b31f5175c73a4c17026`.
  TypeScript **PASS**, source
  `165dac8f2421d163a59238645c252c3cb22dc59843e3d331d7b6f47fc37becfe`.
- Końcowy `--suite browser` snapshot
  `377550916f28b3b9831790b6c8ac72c55a2c611efc7422c13dd651c72feff6e9`:
  Vite production build **PASS**, **165 relay + 1 identity + 35 runtime PASS**,
  następnie **5 Chromium acceptance PASS**. Kompilacja2m13s, relay53.93s,
  runtime7.41s, browser20.33s; peak2916700160 bytes, oom/oom_kill=0.
- Nowy browser test rzeczywiście tworzy actor przez cookie HTTP bootstrap,
  kolejkuje B/C podczas aktywnego A, edytuje C→D, zmienia kolejność D/B, usuwa B,
  anuluje A przez native AHP WS i sprawdza barierę przed odpowiedzią ACP.
  Peer log dokładnie: promptA,cancel,cancelled-result,promptD,completed.
  B nie jest wysłane; A/B mają SQLCancelled, D Completed. Dodatkowo stare cztery
  scenariusze (OIDC, observer, dropped ACK dispatch, pairing/teams) nadal PASS.
- Siedem nowych runtime tests: FIFO/typed reducers, missing reorder/NoOp,
  cancel/complete race, raw late evidence, no-next-before-settle, recovery unknown,
  quota reserve, rollback final ledger insert dla wszystkich kontroli,
  scope/dedupe/reopen. Dwa nowe actor tests używają rzeczywistego peer procesu:
  jedno cancel i dalsza praca; drugie timeout/kill/Unknown i queued work niewysłane.
- Cały E3 i cel do E10 pozostają nieukończone: jednorazowe zgody, steering,
  pozostałe operacje/conformance, create UI/model discovery, artefakty/kontekst,
  duża historia, multi-host i odbiór rzeczywistych integracji nadal otwarte.
  Bez commitów, pushowania i realnych modeli/IdP/tailnetu/keyringu.
- Końcowy Clippy **PASS**, snapshot
  `301601b4353e986091ca9d47feb4dfc41efb8e57ea739bb629dcc2f8bc7846db`:
  relay/runtime/identity/domain/protocol all-targets offline/locked no-deps
  `-D warnings`, 1m28s, peak1204334592 bytes, oom/oom_kill=0.
  Różnica snapshotu względem browser dotyczy dokumentacji; kod po testach
  wykonawczych nie był zmieniany. Format/whitespace/112 Markdown, 4 launcher
  regression tests i 12 vendor tree hashes **PASS**. Bez nowych dependencies.
- Formalnie nadal 12/68 pełnych punktów: kolejne elementy E3.3–E3.5/E3.7
  wykonane, ale cały cel pozostaje aktywny. Nie oznaczono zewnętrznych bramek
  ani pełnego AHP conformance jako zaliczonych.
- Cleanup potwierdzony: dunlin-checks Stopped, default Running; cache zachowany.

## 2026-11-11 — E3: jednorazowe zgody narzędzi ACP/AHP

Poprzedni cykl był postępem (kontrola kolejki/cancel z dowodem ACP i browser).
Ten cykl realizuje dalszy fragment pełnego celu E0–E10, bez zmiany definicji sukcesu.

- Migracja 23 / PERMISSION_SCHEMA: interactive_permission_challenges z nonce,
  exact request/tool/options digest, turn/tool, runtime/epoch/authority stamp,
  starter device, binding/harness, expiry, decider device/client/seq i etapem
  pending→decided→consumed→delivered albo invalid/unknown.
- Jedynie ACP allow_once/reject_once mapowane na opaque host option IDs; brak
  allow_always/reject_always. Virtual deny przy braku opcji odmowy. Żądanie
  <=32KiB, <=8 unresolved/session, total256/session/8192global; root TTL<=120s
  w istniejącym deadline tury (library TTL1..300). Malformed request fail-closed.
- ToolCallReady pending-confirmation pokazuje konkretny raport. Minimalny
  ToolCallConfirmed wymaga aktualnej opcji/nonce, human private-owner i fresh
  CreateSession. To nie zmienia workflow Review/SoD ani nie pozwala service
  principal zatwierdzać. Modelowy raport nie tworzy human user-action.
- Pierwsza decyzja atomowo zapisuje SQL/source/projection/origin/outcome ledger.
  Duplicate zwraca poprzedni wynik; stale nonce lub konkurencyjna decyzja jest
  odrzucana. Reuse peer requestId/toolCallId w nowej turze ma nowy nonce.
- Actor nie blokuje ACP reader na czekaniu na człowieka; bounded pending tickets
  są sprawdzane obok aktualizacji i prompt response. Completion/cancel/expiry
  unieważnia nierozwiązane żądania. Old superseded ticket nie zmienia nowego toolu.
- Przed consume i zewnętrznym zapisem sprawdzane są fresh decider device/OIDC/host,
  exact consumed nonce, lease/stamp, expiry, turn/cancel. Consumed jest write-ahead;
  delivered dopiero po flush. Restart/interruption consumed-not-delivered daje
  unknown, nigdy ponowną zgodę. Nie obiecuje się transakcji exactly-once z ACP.
- Raw late reports zachowane, ale nie zamieniają odmowy w sukces ani agent-report
  w human proof. Human meta/nonce zachowane po zgodzie; canonical revoke metadata
  zastępuje wcześniejszy live stan również we frontendzie. UI osobno pokazuje
  pending human/delivery i unknown forwarding, nie obiecuje wykonania.
- Lifecycle używa recovery reserve; normalny source quota rezerwuje16events/8KiB
  przy niezmienionym total1024events/8MiB, żeby8pendingchallenges mogło się zamknąć.
- Równoległy runtime, frontend i permission peer/browser; root wykonał actor,
  transport, Store migrację i deterministyczne testy revoke-before-consume oraz
  cancellation-while-awaiting-human. Pole `_meta.dunlin.approveTools` jest per
  session/user i wymaga aktualnego actor gate. Zgoda nie rozszerza OS sandboxu.

### Faktyczna weryfikacja

- Pierwszy Clippy snapshot
  `f937261f64d36a39436bde88c92ccdbbdff2960e9bea68e08c343929f6a927d8`
  skompilował kod, ale2reopen tests miały brak drop(Journal) przed drop(Connection).
  Dodano jawne drop(j). Tego przebiegu nie uznano za PASS.
- Wcześniejszy frontend47PASS; po review naprawiono Complete canonical metadata
  i finite SDK shape (revocation nie zostawia dawnego humanproof). Końcowy Node22
  **50 PASS**, source
  `85482fd84f8270e0b6ae21b827b1948b1bb0d98004ad26c98223e1762ffa1a46`.
  TypeScript **PASS**, source
  `55bf57738834d3504cf257d310379abd568c72eb4e4cc05587344867a8300ba5`.
- Końcowy browser snapshot
  `b3b858dc2d13dd7a35dc336e0c28d3c97cfaed8444b4815c0d1f098628f74b30`:
  Vite production **PASS**, **167 relay + 1 identity + 41 runtime PASS**,
  następnie **6 Chromium acceptance PASS**. Relay54.79s, runtime6.12s,
  browser27.96s; peak2937913344 bytes, oom/oom_kill=0.
- Nowy browser faktycznie uruchamia actor przez cookie HTTP bootstrap, otwiera
  dwa obserwatory, czyta pending permission, widzi brak zapisu przed decyzją,
  pozwala raz i potwierdza dokładnie jeden marker. W kolejnej turze peer ponownie
  używa permission-1/tool-1; host nonce jest inny. Trzecie native WS wysyła starą
  opcję do nowej tury i dostaje rejection bez nowego ACP grant. Drugi obserwator
  odmawia, a drugi plik nie powstaje. Dotychczasowe5scenariuszy nadal PASS.
- Nowe dwa actor tests: zaakceptowana decyzja + revoke decider device w jednej
  Store critical section przed consume daje ACPcancelled bez sideeffect;
  cancel podczas czekania na człowieka odwołuje challenge i kończy turę bez grant.
- Sześć nowych runtime tests: first-wins/one-use/reopen, nonce supersession/deny,
  expiry/stamp/cancel, consumed-unknown recovery, rollback registration/projection/
  final ledger i malformed/permanent options/scope/tamper. Frontend sprawdza
  jednoczesne pendingtools, currentnonce, optimistic/rejection/retry/oldACK oraz
  zastąpienie metadata revoke i brak późniejszego przekształcenia odmowy w sukces.
- To kontrolowany lokalny ACP peer, nie prawdziwy komercyjny model, provider IdP,
  tailnet czy keyring. Cały cel i pełny E3 pozostają otwarte (sharing ACL, pozostały
  conformance, create UI/model discovery, steering, duża historia, integracje).
  Bez commitów, pushowania i zmian rzeczywistych danych/poświadczeń użytkownika.
- Końcowy Clippy **PASS**, snapshot
  `b97afcc7e34b7ca786f89d6e263c614f8a66db06a06c156bfa4423a54c91b629`:
  relay/runtime/identity/domain/protocol all-targets offline/locked no-deps
  `-D warnings`,1m22s,peak1205297152 bytes,oom/oom_kill=0.
  Różnica względem browser obejmuje dokumentację; kod po browser nie zmieniany.
  Format/whitespace/112 Markdown i12vendor trees PASS. Brak nowych dependencies.
  Formalne punkty nie są domykane samym prototypem prywatnej sesji; pełny cel
  pozostaje aktywny i nieukończony.
- Cleanup potwierdzony: dunlin-checks Stopped, default Running; cache zachowany.
  Cztery regression tests launchera również PASS.


## 2026-11-11 — E2.4: jawne udostępnienie odczytu (integracja)

- Backend/runtime, frontend oraz testy browser/HTTP/WS wykonane równolegle przez
  trzech agentów; integracja i weryfikacja przez root. Bez commitów/pusha.
- Migracja 24: prywatne domyślnie sesje interaktywne, lista do 32 odbiorców,
  CAS, stampy authority obu stron, rewizje grantów i metadane audytu. Właściciel
  wymaga ShareOwnSession, odbiorca Human + ViewProject/ViewSharedSession.
  Role administracyjne nie tworzą niejawnego dostępu do transkryptu.
- Centralna walidacja obejmuje snapshot, projekcję i replay. Współdzielenie nie
  nadaje żadnych mutacji. Dodano jawny owner gate zatrzymania aktora: wcześniejsza
  prywatność snapshotu była pośrednią ochroną tej operacji.
- AHP zapisuje klucze widoczności przy durable client; odwołanie zamyka WS1008,
  revoke/regrant przed invalidacją wymusza WS1013 i świeży reconnect snapshot,
  nawet gdy boot/źródłowy serverSeq nie zmieniły się. Każda ramka ma fresh guard.
- UI właściciela: jawny zapis, konflikt bez automatycznego replay PUT, nieaktywne
  wpisy odznaczone, ograniczony katalog, audit last20. Odbiorca ma tylko odczyt;
  revoke usuwa wybraną rozmowę z DOM. Regrant udostępnia pełną historię.
- Limity: 1024 zwykłe zmiany +32 zarezerwowane cofnięcia; katalog256+existing
  entries; bounded display names. Duży/importowany katalog nie blokuje revoke.
- Testy dodane: 6 runtime (private/admin, brak callbacku, wszystkie odczyty i
  zakaz mutacji, authority/ABA, CAS/reopen/rollback i capacity);3 HTTP (owner/reader,
  STOP, CAS, origin, revoke/identity oraz oversized catalog);real WS visibilityABA;
  Chromium owner/reader/stranger, UI grant/live/revoke/regrant i audit.
- Weryfikacja: 58 frontend tests i TypeScript PASS. Pełne wyniki serwera
  i Chromium oraz skróty snapshotów zapisano poniżej.

- Pierwszy pełny snapshot `29b3bc48c996381e883ffd61a6e57744b7d01bb15a145395235854e816bdf53b`:
  build PASS;171 relay +1 identity +47 runtime PASS;7 Chromium PASS (w tym nowe
  współdzielenie);peak2972221440 bytes,oom/oom_kill=0. Po nim ograniczono nazwy
  odbiorców do128 Unicode scalars i dodano duże nazwy do testu oversized katalogu.
- Frontend sandbox `f9b608f5216d3ad4f4632f0efc4773975a469bf87d7b3c779725c576f0827f81`:
  58/58 PASS;TypeScript `f0ae42106ed280af8e22737e110ed98a348002e1dd4b8dcfc1b1bbcfa22554f4`
  PASS. Frontend źródła po tych testach niezmieniane.
- Wstępny Clippy `08150142d345a67a31607338f46f9580c4af62f63a9e3eb9d5d1bcdd5ec23256`
  PASS,1m31s,peak1509842944 bytes,oom/oom_kill=0. Końcowy przebieg obejmie też
  dodane później testy i bounded names.
- Formalna lista E0–E10: **13/68 = 19,1% pełnych punktów**. E2.4 zamknięte dla
  wspieranych sesji interaktywnych; historie desktopu pozostają prywatne.
  Całość nadal aktywna i nieukończona. Następny przekrój: native grant epoch i
  actor fencing, następnie host identity/enrollment oraz authenticated worker.
  Read-only inventory wykazało brak tombstones/revision w native project_grants;
  naprawa samego actora nie zamknie jeszcze ABA długich legacy operations/jobs.

- Końcowy browser snapshot
  `903ac8c209c90a8f61ad79cc6edfbd0492c6e7f5f7b2edfc83516c695bbef32d`:
  production build PASS;171 relay +1 identity +47 runtime PASS (55,69s/8,22s);
  wszystkie7 Chromium PASS (34,87s), w tym real UI sharing i regresje dawnych
  kontroli/zgód/OIDC. Peak2815672320 bytes,oom/oom_kill=0. Snapshot obejmuje
  bounded names i końcowy test oversized katalogu; po nim brak zmian kodu.

- Końcowy Clippy: ten sam source digest
  `903ac8c209c90a8f61ad79cc6edfbd0492c6e7f5f7b2edfc83516c695bbef32d`,
  relay/runtime/identity/domain/protocol all-targets offline/locked no-deps
  `-D warnings` PASS,1m21s,peak1211875328 bytes,oom/oom_kill=0.
- Końcowe fmt/whitespace,112 Markdown,12 vendor trees i4 regression launchera
  PASS. Brak nowych dependencies; brak commitów/pusha i operacji na realnym
  tailnecie/IdP/keyringu/modelach.
- Cleanup potwierdzony: dunlin-checks Stopped; default Running. Cel E0–E10 aktywny.


## 2026-11-11 — native grant epochs: actor, ACP, Git plans i jobs

- Poprzednia tura: konkretny postęp (E2.4 wdrożone i zweryfikowane), bez blokady.
  Kontynuacja pełnego E0–E10. Trzej agenci równolegle: Store/migracja/vault,
  actor/realACP, HTTP Git/status tests; root integracja ACP/CLI/jobs/Linear.
- Migracja25 backfill native grants revision1active; trwałe tombstones, CAS,
  capture/revalidate exactstamp, no-op bez invalidacji, terminalna rewizja na
  revoke. grant wrapper kompatybilny; lokalne ungrant i opcjonalne CLI CAS.
  Browser/native APIv1 format Project niezmieniony; historia scope nadalopaque.
- Native stamp w Task/Admission obejmuje actor queue, permission decision,
  consumption i delivery. Nowa sesja wymaga jawnego create z nową authority.
- Legacy ACP przypina primary+additionalroots, sprawdza każdy input/output,
  invalidację i guardian, zabija grupę procesów. Coordination i vault używają
  świeżego callbacku bez przechwytywania AppState/Vault Arc cycle. Nowy token ze
  starego Trackera nie odświeża epoki. Vault deny jest sticky, callback bez locków
  rejestru sesji. Testy brokera/memory nie używają realnego keyringu/modeli.
- History/oneuse prepared Git plans przypinają user/epoch/path i konkretny
  dodatkowy grant WorktreeRemove. Stary scope/plan nie odżywa po regrant.
- Jobs Task ma optional persisted stamp; oldNone failclosed bez dopisywania
  współczesnej authority do historycznego consent. Rejestracja checkoutu ma
  exactstamp pod tym samym lockiem. Native stale status delivery nie mutuje
  Linear; zmiana w trakcie odpowiedzi wymaga manual review, bez retry.
- Wstępny Clippy snapshot
  `4745f33173ffe53fb0f3c21ef102ca0c1ac98ead2cf97b6416737782e2a17ef6`
  wykrył brak kwalifikacji User/Device w fixture testu; poprawiono import scope.
  Końcowe źródła zawierają też późniejsze realWS/status tests oraz final guard
  rejestracji checkoutu. Pełna weryfikacja w toku.
- Otwarte konkretne trasy: terminal.rs ws/bridge; api.rs FS/explorer i starsze
  git_* routes; prepared restore/comparison, check_run i pozostałe długie
  odczyty/artefakty. Następny pion: wspólny guarded I/O helper, te trasy i PTY.
  Clone/copy management wymagają oddzielnego fresh admin/device policy (nie
  konsumują istniejącego native grantu). E2.3/cała klasa ABA nadal nieukończona.

- Browser snapshot `f98fd11cbf44a907e840e043b228b58a2979f4fef0b1ab2b512c3ed3c12bb7b3`
  skompilował kod i przeszedł nowe actor/ACP/Git tests, ale zawisł w fixture jobs:
  tymczasowy Store MutexGuard w polu Task żył do końca struct literal, a kolejne
  pole ponownie czytało Store. Potwierdzono żywy proces w dedykowanym kontenerze;
  przerwano ten kontener (exit143), przeniesiono capture do oddzielnego let i
  uruchomiono nowy snapshot. Nie uznajemy przerwanego przebiegu za PASS.
- Na prośbę użytkownika wykonano `cargo clean --profile dev`:9764 files/5,1GiB
  usunięte, wolne miejsce4,2→8,7GiB. Zachowano aktywne dependencies/snapshots
  target/web-checks; test działa w oddzielnym bounded tmpfs i nie został przez
  clean przerwany. Hostowe target po clean230MiB.
- Dodatkowa granica na następny pion: jobs pinują grant źródłowego projektu;
  osobnego przypięcia wymaga odwołanie grantu pochodnego checkoutu podczas pracy.

- Końcowy browser snapshot
  `75af828ab1809bfc9b4a4fd9a83954d264398963d14a8c7eeed4ead3d68af00c`:
  production build PASS;191 relay +1 identity +47 runtime PASS (62,62s/6,71s),
  wszystkie7 Chromium PASS (38,65s). Peak2935238656 bytes,oom/oom_kill=0.
  Wszystkie20nowych regressions PASS; fixturedeadlock naprawiony. Po snapshot
  nie zmieniano kodu. Frontend źródła niezmienione względem58/58+TS z E2.4.

- Clippy tego samego snapshotu zakończył się jednym `collapsible_if` w no-op
  native grant; zastąpiono zagnieżdżony warunek równoważnym let-chain bez zmiany
  zachowania. To jedyna zmiana kodu po zielonym browser snapshot. Końcowy lint
  uruchomiony ponownie; brak potrzeby powtarzania niezmienionych scenariuszy UI.

- Końcowy Clippy snapshot
  `27398664c5a73e4bc01cace85206b9ab0173dd4cfb1bcf865bd9bc4d98c2652b`:
  relay/runtime/identity/domain/protocol all-targets offline/locked no-deps
  `-D warnings` PASS,1m22s,peak1212706816 bytes,oom/oom_kill=0.
  Różnica kodu względem browser75af to wyłącznie równoważny let-chain no-op.
- Format/whitespace,112 Markdown i12vendor trees PASS; brak nowych dependencies,
  commitów/pusha, realnych integracji/poświadczeń. Formalna lista nadal13/68:
  znaczący fragment E2.3/E3 zweryfikowany, ale pozostałe I/O/PTY/hostenrollment
  i cały cel E0–E10 pozostają aktywne. Kolejna tura: guarded I/O+PTY oraz epoch
  pochodnego checkoutu jobs; sprawdzić też fingerprints dawnych continuationów.
- Cleanup: dunlin-checks Stopped, default Running. Wolne miejsce8,8GiB; target170MiB po usunięciu snapshotów testowych.


## 2026-11-11 — guarded I/O, PTY i pochodne checkouty

- Poprzednia tura przyniosła konkretny postęp: native epochs i ich weryfikację,
  a na prośbę użytkownika także cargo clean. Cel E0–E10 pozostaje aktywny.
  Na początku tury dysk miał 8,8 GiB wolnego miejsca; target zajmował 170 MiB.
- Trzech agentów pracowało równolegle nad 29 trasami FS/explorer/audit/legacy Git,
  PTY oraz grantem pochodnego checkoutu. Root integrował NativeIo, testy i launcher.
- NativeIo przypina grant/path/user/device. Sprawdza aktualne urządzenie, politykę
  kont hosta, OIDC i dokładną epokę przed I/O oraz przed odpowiedzią. Blocking
  worker zachowuje permit po anulowaniu caller future. Odmowa przed operacją
  pozostaje 403/401; utrata authority po rozpoczęciu mutacji daje 409 z informacją
  o możliwym częściowym skutku i zakazie automatycznego retry. Zachowano mtime 409,
  quota 429 i wewnętrzne błędy uploadu 500. Brak obietnicy transakcji SQL + FS.
- Preview ma końcowy guard po base64. Upload sprawdza grant przed utworzeniem
  katalogów i pliku attachment oraz przed zapisem Git exclude. Przegląd integracji
  wykrył błędne nadpisanie preflight 403 przez 409 i brak bezpośredniego guardu
  katalogów; oba poprawiono i objęto regresjami. Dodatkowy test rzeczywistej
  odmowy uprawnień FS sprawdza zachowanie statusu 500.
- Migracja 26 dodaje nullable checkout_grant do relay_jobs, bez zmiany zatwierdzonego
  Task ani fingerprintu. job_running zapisuje stamp, project, branch i status
  atomowo. Runner weryfikuje source oraz derived grant/path. Stare checkouty bez
  pinu nie otrzymują automatycznie współczesnej authority.
- PTY przypina grant/path/identity, sprawdza je przed spawn i I/O, ma invalidację
  oraz niezależny guardian co 1 s i policy close 1008. Admin setup wymaga aktualnego
  admina i urządzenia; testy nie uruchamiają logowania dostawcy. hard_terminate
  kończy potwierdzone grupy shell/foreground tej sesji PTY. Detached sessions/groups
  nadal wymagają adaptera izolacji OS. Desktopowe write/kill zachowują semantykę.
- Launcher server/browser/clippy obejmuje także dunlin-terminal. Dodano krawędź
  zależności Unix do już istniejącego libc 0.2, bez nowej wersji lub pobierania.
- Pierwszy browser snapshot
  `cdfdbe2639010786a048b3c2a1bd4dbdf2baac765817426fff9408f1ec0b2e92`
  skompilował kod i przeszedł nowe NativeIo oraz PTY grant/device tests do przypadku
  zablokowanego wejścia. Ten test ujawnił prawdziwy hang: procesy były zabite,
  ale spawn_blocking write_all pozostał w kernel wait_woken i blokował shutdown.
  Potwierdzono żywy kontener i /proc task state; zatrzymano wyłącznie ten kontener
  (exit 143). Przerwanego suite nie uznajemy za pełny PASS.
- Poprawka wprowadza relay-only O_NONBLOCK, obsługę WouldBlock w readerze,
  przerywalne zapisy częściowe oraz closing flag sprawdzany co 5 ms. Osobny
  RAII owner kończy sesję również przy retained Arc lub anulowaniu bridge future.
  Nie usunięto trudnego testu. Przegląd wykrył dodatkowo disconnect podczas
  zablokowanego zapisu przy ważnym grancie; dwusekundowy budżet zapisu ramki
  ogranicza również ten przypadek i ma oddzielny test procesu oraz permitu.
- Dodano opcję --rust-test do server/sessions/domain. Walidacja bounded identifier
  i shell quoting odrzuca metaznaki oraz opcje Cargo. Pięć testów launchera PASS.
- Focused snapshot `2eb107dc289653b6ec0eef46d20c9b9784a451b4e492e81b5a000d93ceab8d88`:
  --suite server --rust-test terminal_revocation_watchdog_still_kills_a_pty_with_a_blocked_input_writer
  PASS: dokładnie 1 test relay, 0,21 s, proces zakończył się prawidłowo.
  Kompilacja 2m06s; peak 3034390528 bytes, oom/oom_kill=0. Pozostałe testy były
  świadomie filtrowane; pełny suite uruchomiono oddzielnie na końcowym snapshotcie.
- Niezależny audyt E3.6 potwierdził recovery core i testy file-reopen, ale wykazał
  brak dowodu prawdziwego crash/relaunch procesu serwera podczas ACP. Test AHP
  restart otwiera Store/AppState po zapisanych danych, nie zabija żywego serwera.
  E3.6 pozostaje otwarte do tego testu; operator reconcile/resume jest dalszą
  funkcją produktu, nie substytutem zakazu automatycznego replay.

- Pełny snapshot `4537271a73ced48c77aacb2a2e8ebe01f4d7221735bda4215cdff1e961fc2d09`
  przeszedł testy relay/runtime, w tym oba blocked-PTY przypadki. Dodane do suite
  starsze testy terminala wykryły jednak ograniczenie środowiska: /tmp istniało,
  lecz było readonly; dwa testy budowania argv SSH nie mogły utworzyć prywatnego
  control directory. Nie uruchamiają one SSH ani połączeń sieciowych.
- Bez zmiany aplikacji udostępniono osobny /tmp tmpfs: 16 MiB, noexec/nosuid/nodev,
  bez mountów hosta. /scratch pozostaje ograniczone osobno. Pięć regression
  launchera i rzeczywisty probe PASS, w tym rozmiar /tmp i zachowane readonly
  root/source, puste środowisko, brak host home/socket/git oraz network none.
  Pełny suite ponowiono na identycznym source digest; nie pomijano dwóch testów.

- Incydent dokumentacji: przy dopisywaniu wyników `open(p,'w').write(s)` otworzyło
  plik przed błędem undefined s i wyczyściło progress.md. Nie dotyczyło to kodu ani
  trwającego suite. Odtworzono pełny baseline 273 linie i 66 literalnych operacji
  zapisu z lokalnej historii tego wątku, bez wykonywania starych poleceń. Nie było
  brakujących replace targets; historyczny tail zgadzał się z kandydatem.
  Checkpoint odtworzenia: 1362 linie, 96177 bajtów UTF-8, SHA256
  `a755f6aa2fa25475eb8d4cbcdb9f1db9037bf56222e32e80d59dc7f0b18fc21e`.
  Przywrócono go atomowo po weryfikacji hash i zachowano lokalną kopię recovery
  w target/web-checks/doc-recovery. Niniejsze wyniki dopisano już po odtworzeniu.
- Końcowy pełny browser snapshot (ten sam source digest po poprawce środowiska):
  `4537271a73ced48c77aacb2a2e8ebe01f4d7221735bda4215cdff1e961fc2d09`.
  Production build PASS; 205 relay +1 identity +47 runtime +27 terminal PASS
  (relay 65,63 s, runtime 6,03 s, terminal 0,02 s). Wszystkie 7 Chromium PASS,
  32,65 s. Peak 3073933312 bytes, oom/oom_kill=0. Kod po tym snapshotcie niezmieniany.
- Clippy tego samego digestu: relay/terminal/runtime/identity/domain/protocol,
  all-targets offline/locked no-deps -D warnings PASS, 1m50s.
  Peak 1243992064 bytes, oom/oom_kill=0. Zakres obejmuje nowe metody PTY.
- Formalna lista nadal 13/68 =19,1% pełnych punktów. Rozszerzono znacząco E2.3,
  ale nie domknięto prepared restore/comparison/checks, pozostałych odczytów,
  administracyjnej publikacji clone/copy ani remote host/service authority.
  E3.6 czeka na real crash/relaunch proof. E0–E10 pozostaje aktywne i nieukończone.
  Nie było commitów/pusha ani realnych operacji tailnet/IdP/keyring/model.
- Końcowe fmt, git diff --check, 112 Markdown i 12 vendor trees PASS.
  Cleanup potwierdzony: dunlin-checks Stopped, default Running; target 170 MiB,
  wolne miejsce 8,6 GiB. Hostowe artefakty Rust nie zostały ponownie zbudowane.

### Prepared I/O, publikacja administracyjna i rzeczywisty restart (2026-11-11)

- Poprzedni obrót celu: progress — na żądanie użytkownika `cargo clean` usunął
  5603 pliki /153,8 MiB, pozostawiając 8,6 GiB. Usunął również odtwarzalne cache
  target/web-checks. Źródła i dokumentacja pozostały zachowane. Readonly mount
  VM wymagał restartu wyłącznie dunlin-checks; profil default pozostał Running.
- Prepared restore/comparison przypinają NativeProof (wartości bez Arc AppState),
  użytkownika, urządzenie i projekt. Obcy caller nie konsumuje planu; wykonanie
  sprawdza dokładną epokę i świeże urządzenie. Rejestracja derived projects odbywa
  się pod tą samą blokadą Store co kontrola source proof. Checks mają niezależny
  guardian, anulowanie przy HTTP drop i utracie grantu/urządzenia, permit/registry
  utrzymywane do rzeczywistego zakończenia pracy. Dodano siedem regresji.
- Kolejne 13 prywatnych endpointów obejmuje impact, GitHub metadata, review,
  comparison oraz artefakty. Publikacja artefaktów jest etapowana, bez nadpisania,
  z kontrolą proof przy wpisie Store; odczyt ma limit i ponowną kontrolę dostępu.
  Publiczny token pozostaje odrębną, świadomie udzieloną capability. Dodano sześć
  regresji, w tym HTTP queued read ABA i odrzucenie stale admin GitHub metadata.
- Administracyjne clone/copy używają wartościowego AdminAuthority. Wolne Git/hash
  działają poza Store, prywatny staging jest publikowany po świeżej kontroli
  admina i urządzenia w tej samej sekcji co rejestracja. Copy zachowuje tożsamość
  urządzenia rozpoczynającego; inne urządzenie/admin nie przejmują zatwierdzenia.
  Niepewny efekt publikacji daje 409 i nie jest automatycznie ponawiany. Cztery
  regresje obejmują rzeczywisty kontrolowany Git subprocess i późny błąd SQL.
- Nowy test production_browser_acceptance_runtime_restart uruchamia prawdziwy
  relay z sandbox=true, rzeczywistym bubblewrap i lokalnym fixture ACP. Zabija
  SIGKILL wyłącznie serwer, sprawdza osierocone procesy przed cleanupem, ponownie
  otwiera tę samą DB, sprawdza Unknown/Queued, boot snapshot, dedupe starego
  żądania, odrzucenie starego nonce i pojedynczy efekt świeżej jawnej sesji.
  Pierwszy rzeczywisty PASS: snapshot
  e9f8f55d4b6ea880856cc55e88f1d85438074bbffb50f42050a5b41221765d39,
  test 0,75 s, build 2m07s +28,84s, peak3098181632 bytes, oom/oom_kill=0.
  To poprzedni snapshot przed końcowymi zmianami artefaktów; nie zastępuje final run.
- Suite runtime wymaga jawnie przygotowanego obrazu browser-bwrap i nazwanej
  polityki AppArmor. Generator weryfikuje upstream profiles oraz licencję SHA256
  z Moby commit6049e40058759e8a1673ac1090ec6a7e36b9c9bd. Tylko runtime dodaje
  sześć namespace/mount syscalls i named mount/pivot_root/userns profile oraz
  systempaths=unconfined dla wewnętrznego /proc. Nie jest to privileged container;
  zachowano nonroot, cap-drop, no-new-privileges, network none i limity.
  Szczegóły/ograniczenia: runtime-checks.md. Profil ładowany tylko w dedykowanej VM.
- CI ma osobny wymagany runtime-sandbox job i pięć wejść zbiorczego gate.
  Trzy Docker buildy używają stdin bez wysyłania całego repo jako kontekstu.
  Kontrakty CI14 i actionlint PASS; nie twierdzimy, że wykonano rzeczywisty CI.
- Pełny Python discovery wykrył kolizję nazwy test_web_sandbox z produkcyjnym
  helperem po zmianach sys.path przez inne testy. Moduł testowy przemianowano na
  test_web_sandbox_launcher; aktywne wzorce zaktualizowano. Pełne discovery107,
  focused sandbox6 i CI14 PASS. Produkcyjna semantyka launchera bez zmiany.
- Końcowa weryfikacja Rust/Chromium/runtime jest w toku. E2.3 i cały E0–E10
  pozostają nieukończone; brak commitów/pusha i realnych operacji kont dostawców.
- Po odtworzeniu cache: runtime snapshot
  b864b2300536fedb11e46e4553e1cdebd5ad0e5320f0fb812f48656992e34a59
  PASS1/229filtered, 0,73s; build2m21s +42,68s. Memory peak3221225472,
  memory.max events1266, oom/oom_kill=0: osiągnięto limit pamięci z reclaim,
  bez zabicia procesu. Ten przebieg poprzedza utwardzenie oracle /proc.
- Oracle utwardzony: wyłącznie NotFound lub poprawny Z/X oznaczają zakończenie;
  inne I/O, błędny format stat/children i błędy iteracji nie dają fałszywego PASS.
- Frontend58 PASS (snapshotf9b608f5216d3ad4f4632f0efc4773975a469bf87d7b3c779725c576f0827f81),
  TypeScript PASS (f0ae42106ed280af8e22737e110ed98a348002e1dd4b8dcfc1b1bbcfa22554f4).
- Pierwszy pełny browser suite snapshot
  64d1323b652c1e4c6450a0f29ef7bbeb4ef7e6e149347aec3ccb22a09456249e:
  production web build PASS; relay221PASS/1FAILED/8ignored. Przyczyna:
  nowy active_checks scope fixture próbował utworzyć dwa projekty o tej samej
  ścieżce, co poprawnie odrzucił UNIQUE projects.path. Chromium nie wystartował,
  bo launcher zatrzymał się na błędzie testów. Fixture poprawiony na odrębny
  rzeczywisty katalog, z dodatkowymi asercjami różnych paths/IDs; nie usunięto
  żadnej asercji scope/process/permit. Pełny suite będzie ponowiony.
- Końcowy rzeczywisty runtime z utwardzonym oracle: snapshot
  64d1323b652c1e4c6450a0f29ef7bbeb4ef7e6e149347aec3ccb22a09456249e,
  PASS1/229filtered, test0,69s, build2m06s +27,57s,
  peak3202686976 bytes, max/oom/oom_kill=0. Od tego snapshotu zmieniono wyłącznie
  fixture osobnej ścieżki projektu w niezwiązanym teście checks, bez zmian runtime.
  Niezależny audyt i przegląd root potwierdziły atomowe startup recovery przed
  serve, brak automatycznego reacquire Interrupted, trwałe potwierdzenie i nonce
  invalidation. E3.6 zamknięte w literalnym zakresie no-auto-repeat obecnego
  runtime, z dowodem Linux. Prawdziwy crash cut: Running po efekcie/pendingpermission
  plus Queued; prepipe Dispatching i consumed-beforeflush pokryte jednostkowo.
  Odbiór macOS, real harnessów, DB corruption/powerloss i operator reconcile/resume
  nie są dowiedzione tym testem i pozostają otwarte. Całe E3 nie jest zamknięte.
- Drugi pełny browser suite (fixture checks poprawiony): snapshot
  e90377984182d6a0a4a5f73e6cd002dd10ff11d9f86f4e6cf5b50c9a726f23ac,
  build1m55s, relay221PASS/1FAILED/8ignored w69,13s. Checks scope test PASS.
  Wykryto rzeczywisty race normalnego PTY exit: test relay_terminal... dostał
  pusty output. Branch exit wysyłał Close przed drenażem ostatnich bajtów czytnika;
  nonblocking reader mógł jeszcze czekać na kolejną próbę. To wymaga poprawki
  produkcyjnej i regresji, nie retry testu bez zmiany. Chromium jeszcze nie ruszył.
- Po zakończeniu runtime suite usunięto wyłącznie named AppArmor profile
  dunlin-runtime-checks z kernela dedykowanej VM. Browser/Clippy używają
  domyślnych polityk kontenera; globalny profil default nie został zmieniony.
- PTY fix: normalny status procesu jest odkładany do EOF czytnika albo stałego
  2s deadline; exit.recv jest wtedy wyłączone, aby zamknięty kanał nie powodował
  pętli. Revoke/disconnect nadal przerywają drain, każde wyjście sprawdza authority.
  Dopiero po drenażu następuje hard_terminate i raport oryginalnego statusu.
  Nowa regresja trzy razy wymaga pełnych 131072 bajtów Z i końcowego markera
  wygenerowanego numerycznie (echo polecenia nie może podrobić wyniku), przed Exit.
  Niezależny przegląd obejmuje backpressure, odziedziczony slave FD i stały deadline.
- Równolegle rozpoczęto domknięcie legacy sesji/transkryptów i konfiguracji
  projektu: proof capture oraz owner/project i mutacja Store mają wspólny cut,
  settings PUT wymaga bieżącej roli administratora. To kolejny zakres w toku,
  nie wykonany odbiór. Final full suite poczeka na source freeze obu zmian.
- Linux tooling suite PASS20 (CI14+launcher6), source snapshot
  7e4037beee049bd97b455a037634af134eea464f5acd47baac617bf9dc74c4af.
  Cache target po odtworzeniu95MiB, bez hostowych artefaktów kompilacji Rust.
- Formalny licznik po odbiorze E3.6:14/68=20,6%. To licznik w pełni
  zweryfikowanych punktów, nie wycena częściowo wykonanej pracy. E0–E10
  pozostaje aktywne; zewnętrzne bramki real tailnet/IdP/keyring/provider/multi-host
  i funkcje E4–E9 nadal otwarte. Nie wykonano commitów ani push.
- Legacy API source freeze: dziewięć handlerów sessions/context/settings oraz
  owner-scoped transcript/revision/CAS/delete/takeover sprawdza proof w Storecut.
  capture_in_store nie relockuje Store; owner i project są ponownie sprawdzane
  przed operacją. Foreign/missing session nadal404, owner bez grantu403. Takeover
  bez zmiany nie udaje unknown mutation. Audyt nie znalazł odwrotnego leases→Store
  zagnieżdżenia względem nowego Store→leases; try_send nie czeka pod blokadą.
  Settings PUT weryfikuje świeżą rolę administratora przy save i odpowiedzi.
  Publiczny artefakt ma bounded permit read oraz końcową kontrolę tego samego
  tokenu/wiersza po odczycie, bez zależności od natywnego grantu uploaderowi.
- Sześć nowych regresji: revoked HTTP legacy reads/writes bez zmiany lease;
  disabled transcript storage zachowane; pinned proof ABA przed read/save/delete/
  takeover daje zero callbacks, retarget projektu404; settings ABA/cachedadmin;
  downgrade aktualnej roli pod Store; public token revoked podczas queued read
  zwraca404 bez bajtów. Pełny build/test po freeze jest w toku.
- Końcowy pełny browser suite po obu poprawkach: snapshot
  8636d8d5bc6b499ac8a2889351413e0d9244e94cd3e7ade0d1139d761d9fb8c8.
  Production web build PASS (726ms), test compile1m59s. Relay229PASS/8ignored
  (70,05s), identity1PASS, runtime47PASS (6,23s), terminal27PASS (0,02s).
  Wszystkie siedem Chromium PASS (32,15s), runtime restart celowo osobno.
  Peak3112202240 bytes, max/oom/oom_kill=0. Final regression PTY oraz sześć
  legacy/publictoken przypadków przeszły. Bez zmian kodu po tym snapshotcie.
- Clippy uruchomione na tym samym kodzie; wynik w toku. E2.3 nadal otwarte:
  remote grants/enrollment/service execution oraz pozostałe admin installation,
  probes/usage/job/run/current-role/output authority nie należą do domkniętego cut.
- Pierwszy Clippy tego samego snapshotu znalazł wyłącznie collapsible_if w
  CheckCleanup::drop api.rs98 (peak1276801024, oom0). Połączono if-let z istniejącym
  warunkiem Arc::ptr_eq jako let-chain, bez zmiany predykatu/kolejności/operacji.
  Brak allow/suppression. To jedyna zmiana kodu po zielonym pełnym browser suite;
  ponowiono Clippy all-targets, bez nieuzasadnionego powtarzania całego Chromium.
- Końcowy Clippy PASS: snapshot
  0644ee5f5b8b6e0fe9a3a098a6d6047be481a60b829a5d4b6d5aa2d07fc39358,
  relay/terminal/runtime/identity/domain/protocol all-targets offline/locked
  --no-deps -D warnings, 1m18s. Peak1261215744 bytes, max/oom/oom_kill=0.
  Różnica względem pełnego browser PASS to wyłącznie opisany równoważny let-chain.
- Odbiór tego przekroju: 229relay+1identity+47runtime+27terminal+7Chromium,
  rzeczywisty SIGKILL/relaunch z bubblewrap, TS58/TypeScript i Python107 PASS;
  CI14+launcher6 dodatkowo Linux tooling PASS; actionlint/fmt/diff/docs/vendor PASS.
  To nie jest wykonany GitHub Actions ani odbiór realnego IdP/harnessu/tailnetu.
  Nie było commitów/pusha ani modyfikacji rzeczywistych kont/poświadczeń.
- Cleanup zweryfikowany: dunlin-checks Stopped, default Running. Nazwany profil
  AppArmor usunięto wcześniej po runtime. Target95MiB (odtwarzalne zależności),
  wolne8,1GiB; Rust budował się tylko w usuwanym tmpfs kontenerów. E0–E10 nadal
  aktywne,14/68 pełnych punktów. Kolejny lokalny przekrój: admin run/jobs/probes
  i usage z aktualną authority; następnie remote host/service execution.

### Bieżąca authority administratora, kontrolowane sondy i parity AHP (2026-11-11)

- Poprzedni obrót celu: progress. E3.6 zweryfikowane rzeczywistym restartem,
  pełny relay/runtime/terminal/Chromium i Clippy PASS; formalnie14/68. Kontynuacja
  dotyczy pozostałych admin run/jobs/probes, nie zmiany pełnego celu E0–E10.
- Management source przypina statefree AdminAuthority urządzenia zlecającego.
  Update/start/job/cancel i info nie ufają wyekstrahowanej roli. Guardian100ms
  oraz invalidacje sprawdzają bieżące urządzenie/użytkownika/OIDC/hostpolicy,
  kończą grupę rzeczywistego procesu; output/status i cancel są origin-device
  scoped. Job pozostaje serwerowy po HTTP disconnect. Wynik niepewny wymaga
  inspekcji, bez automatycznego retry. Limit15min, output32KiB +boundeddiagnostic.
- Audyt root znalazł ryzyko surowego groupPID po child.wait/reap. Poprawiono
  kolejność: waitid WNOWAIT obserwuje zakończenie bez reaping, kill ownedgroup
  przed child.wait. Guardian/Drop mają serializację i sprawdzają child ownership;
  ECHILD nie daje prawa do sygnału potencjalnie ponownie użytemu PID. Regresja
  sprawdza pozostawienie lidera i zachowanie oryginalnego exitstatus.
- Usage/cache/agregacja billing wymagają aktualnego administratora. PUT własnych
  danych billingowych pozostaje dostępne użytkownikowi jego własnego urządzenia;
  fresh DeviceAuthority sprawdzane przy save, bez dodania wymagania admina.
  Worker sondy zachowuje permit do cleanup; requestdrop/deadline odwołują callback.
- Root dodał probe_all_guarded i lokalny ProbeProcess w dunlin-usage. Procesy
  mają nonblocking input/output, stały deadline, ograniczony body/line/output,
  RAII processgroup cleanup także po błędzie zapisu/parse. Security CLI3s,
  Codex6s, HTTP Claude nadal bounded15s (trwającego requestu nie można cofnąć;
  checks przed/po nie pozwalają zwrócić wyniku po revocation). Brak readerthreads
  lub nieograniczonego mpsc. WNOWAIT utrzymuje własność do groupkill/reap.
  Dodano bezpośrednią target-unix libc dependency, istniejąca wersja w lockfile.
- Focused usage suite snapshotf370d4ad0c3602d5fac2f0a376c68435d1536bf40cae8d8afafeb28f618d74bf:
 36PASS, compile25,99s/tests8,71s, peak962547712, max/oom/oom_kill=0. To pierwszy
  dowód przed dalszym utwardzeniem testów/ownership guard; pełny suite będzie
  powtórzony. Testy to wyłącznie lokalne Python fixtures, bez prawdziwych CLI,
  kont, keyringu ani model/provider calls. Launcher ma oddzielny suite usage,
  domyślny server/browser i Clippy obejmują również tę bibliotekę.
- Równolegle audyt pinned SDK wykrył rzeczywiste różnice reducerów TS: metadata
  narzędzi, warunki aktualizacji po terminal status, timestamp/flags i domyślne
  interactivity. Agent przygotowuje traces i testowe wyniki wygenerowane przez
  przypięty SDK Rust, z weryfikacją po każdym kroku. Nie modyfikujemy vendora.
  Partial queue reorder ma znaną sprzeczność SDK vs docs (pozostałe ID sortowane
  vs zachowane kolejno); nie ukrywamy jej jako parity. E3.3 nadal otwarte.
- Login guardian uzupełniony o operation-specific all-admin-relay proof. Pojawienie
  się członka po starcie hostowego logowania kończy proces i blokuje wynik/cancel
  jako409, bez zmiany ogólnego AdminAuthority clone/copy. Controlledgroup test
  potwierdza brak opóźnionego skutku po dodaniu membera. Login/keyring realny nie
  był wykonywany. Outside Runningfixture ma jawnie require_admin_only_relay=false.
- Niezależny przegląd sond wykrył dodatkowy FIFO w fallback credentials. OpenOptions
  używa O_NONBLOCK, a metadata otwartego deskryptora musi potwierdzić regular file;
  limit64KiB i authority przed/po odczycie. Test sprawdza FIFO bez writer, poprawny
  regular file, revoke oraz nadmiarowy plik. Nie zakazano zwykłych symlinków do
  regularnego pliku; nie zmieniono polityki poświadczeń desktopu.
- Actual SDK oracle wygenerowano w VM testem
  pinned_sdk_frontend_reducer_oracle_matches_checked_in_fixture. Placeholder{}
  celowo dałFAIL i wydruk referencji; po zbyt małym pierwszym limicie odbioru stdout
  ponowiono generację z pełnym limitem, nie rekonstruowano brakujących danych.
  Zaimportowano dokładny JSON25502bytes/5traces/48states z niezmienionego SDK.
  Normalny full runtime test zweryfikuje zapisany fixture. Bez patchowania vendora.
- Frontend64 PASS (bf3ef42567e7e6f17069859188cf886afefdaefe6106154870fd83b05d8337dd),
  TypeScript PASS (b232399b2513e40d986b62c26bea942213d85d8f93199190b7750d474726d4e9).
  Cztery traces porównują każdy krok, piąty jawnie sprawdza znany wyjątek partial
  queue reorder zamiast udawać parity. Full browser/backend suite w toku.
- Fmt wykrył wyłącznie porządek cfg(test) mod declarations frontend_oracle/tests
  w projection.rs. Rustfmt przestawił deklaracje bez zmiany kodu i testów; full
  suite uruchomiony przed tą zmianą porządku, Clippy/fmt będą na końcowym źródle.
- Pełny browser/backend snapshot
  e9e31fbdcf2d1ce4e6c31b6f31be7c881365f49256791198f095d074ec396d37:
  webbuild601ms +Rustcompile1m57s PASS. Relay240PASS/8ignored (73,19s), identity1,
  terminal27PASS (0,04s), usage37PASS (10,17s), pełny runtime command PASS.
  Fragment stdout runtime summary został ucięty przez zbyt mały limit odbioru;
  nie odtwarzamy liczby ani logu z pamięci. Osobny verifier SDK będzie potwierdzony.
  Wszystkie7ChromiumPASS (32,56s). Peak3221225472, memory.max events75,
  oom/oom_kill=0 (reclaim przy limicie, bez zabicia procesu).
- Kompilator wykrył nieużywany run_blocking po zastąpieniu usageguardedworker.
  Root potwierdził rg tylko jedną definicję/zero wywołań i usunął martwy helper.
  To oraz rustfmt kolejności mod frontend_oracle/tests są jedynymi zmianami kodu
  po snapshotcie zielonych testów. Brak zmian zachowania po teście; Clippy/fmt
  weryfikują final source, bez powtarzania całego Chromium dla martwego kodu.
- Final Clippy PASS na source
  81cb728dbccd9b4de375475dfe9036786cf3ceae9885c6dbb4cb6b98d79de5f6:
  relay/terminal/runtime/identity/domain/protocol/usage --all-targets offline/locked
  --no-deps -D warnings, 1m26s. Peak1327648768, max/oom/oom_kill=0.
  Fmt i diffcheck PASS. Python pełne discovery107 PASS (24,363s), docs113 i
  vendor12 PASS. Osobny normalny verifier SDK uruchomiony na tym samym source.
- Osobny verifier rzeczywistego pinned SDK PASS na tym samym final source
  81cb728dbccd9b4de375475dfe9036786cf3ceae9885c6dbb4cb6b98d79de5f6:
  dokładnie1PASS/47filtered, compile37,09s/test0,01s; peak1199206400,
  max/oom/oom_kill=0. To potwierdza48 skompilowanych testów runtime oraz zgodność
  fixture48states/5traces z aktualnym przypiętym reducerem. Pełny runtime command
  wcześniej zakończył sięPASS; osobny test nie zastępuje całego suite.
- Odbiór tego przekroju: relay240, identity1, terminal27, usage37, full runtime,
  Chromium7, frontend64, TypeScript, Python107 orazClippy/fmt/docs/vendor PASS.
  Formalny licznik nadal14/68=20,6%: E2.3 pozostaje remote host/service execution,
  E3.3/3.4 m.in. aggregate status, canonicalpartialreorder i paginacja historii.
  Następny przekrój: normalizeacceptedpartialorder do pełnej canonicalaction
  zgodnej z kolejką wykonania; następnie bounded fetchTurns i authorizedcursors.
  Zewnętrzne gate real tailnet/IdP/provider/keyring/macOS/harness acceptance otwarte.
  Bez commitów/pusha, rzeczywistych operacji kont, modeli i zmian tailnetu.
- Cleanup potwierdzony: dunlin-checks Stopped/default Running; target95MiB,
  wolne8,2GiB. Brak hostowych buildów Rust i pozostawionych kontenerów pracy.
  Cel E0–E10 pozostaje aktywny i nieukończony.

### Spójność reorder i paginacja historii AHP (2026-11-11)

- Poprzedni obrót celu: progress — bieżąca authority admina, kontrolowane procesy
  i parity SDK miały backend/Chromium/Clippy oraz oddzielny verifier PASS. Pełny
  cel E0–E10 pozostaje aktywny; formalny licznik14/68 bez zmian.
- Korekta wcześniejszego audytu: controls.rs już normalizuje częściowy reorder
  do pełnej kolejności wykonania przed projekcją pinnedSDK. Wcześniejszy TODO
  sugerujący brak tej normalizacji był nieaktualny. Nie dodano drugiej ścieżki
  produkcyjnej. Dwie nowe regresje rozszerzają dowód o kolejkę[z,a,m]/partial[m],
  SQL/liveSDK/snapshot/persistedaction, retry po konsumpcji i nowych wpisach oraz
  reopenDB/actorunavailable. Czysty SDK nadal ma osobny jawny wyjątek partialorder;
  pełna canonical action serwera omija go bez patchowania vendora.
- Root dodał migrację27: interactive_ahp_clients.views przechowuje wyłącznie
  żądany kształt subskrypcji, oddzielnie od visibility pins. AhpClient.views oraz
  ahp_set_views(device,client,&BTreeMap) mają limit32 chatURI/turncount0..64.
  Rejestracja klienta nie kasuje dotychczasowych views, jawne zmiany transportu
  je zastępują. Brak/expiry klienta nie odżywa przez setviews. Testy obejmą
  device/clientisolation, file-reopen, bounds/TTL i upgrade26→27 bez utraty
  version/boot/visibility. Rustfmt/diffcheckPASS, pełna weryfikacja w toku.
- Backend/UI wdrażają SubscribeView.turns i fetchTurns zgodnie z przypiętym SDK.
  Brak view nadal oznacza wszystkie zachowane turns, nigdy cichy tail. Cursor
  przypina connection/user/device/read-key/fixedsnapshot; pageaction dostaje
  świeżą sekwencję po canonicalcatchup i poprzedza RPCresult{}. Historia nie
  trafia do cudzych klientów ani globalnego journalu jako read-page payload.
- Requested tailview jest trwały perdevice/client. Reconnect paged resource
  wymaga świeżego tail snapshotu i remint cursor, także dla zwykłego SDK klienta;
  nie ograniczamy tego wyłącznie do naszej przeglądarki. Frozen history ma
  resourcebudget, pojedynczy turn/active payload przekraczający frame wymaga
  jawnego błędu; jego externalization pozostaje osobną otwartą częścią E3.7.
- Dopisano szósty trace SDK dla turnsLoaded/prepend/overlap/cursorclear +live.
  Wymaga rzeczywistej generacji nowego expectedfixture i ponownej weryfikacji;
  nie dopisujemy ręcznie oczekiwanego wyniku redukcji.
- Generacja actualSDK dla6traces: snapshot
  2b0d1b0224d8602f312fac50af12c000986d0b947c6f51b14901595a6296988f,
  compile32,34s; oczekiwany mismatch z poprzednim5tracefixture. Zaimportowano
  pełny nieucięty JSON31196bytes/54states i sprawdzono, że pierwsze5traces nie
  zmieniło wyników. Peak1483460608, max/oom/oom_kill=0. Normalny verifier i cały
  runtime zostaną ponowione po sourcefreeze, bez ręcznego edytowania golden.
- Frontendcheckpoint72PASS (4a378c28d109bb3237e2637e85ae729f7f1d6747d639f4b5c84e26014b3e2ae4).
  Pierwszy TypeScript wykrył wyłącznie zawężenie mutable state.chat do never
  w nowych assertions po null-check. Test używa teraz świeżego odczytu helpera
  przy granicy callbacka, bez any/suppression i bez zmiany assertions/produkcji;
  ponowienie typecheck w toku.
- Niezależny audyt E3.4 potwierdził durableboundedledger/outboxretry i obecne
  replay/visibilitypins. Pozostaje szersza luka: REST interactive create nie ma
  requestkey; utrata odpowiedzi i ponowienie może utworzyć drugi actor/session.
  Nie zamykamy E3.4 przez zawężenie go do już wspieranych dispatch actions.
  Idempotent admission będzie oddzielnym następnym przekrojem.
- Pierwszy pełny browser snapshot
  52fee88168320763fc621c54fb73d38bb4790b11589050ef63b7d00b1603d4d3:
  webbuildPASS802ms, Rust zatrzymany na E0505 nowego historybrowserfixture.
  Journal ma Drop i nadal pożyczał Store przy jego przeniesieniu do AppState.
  Po zakończeniu seed/fullsize assertions dodano jawny drop(journal). To błąd
  fixture, nie złagodzenie kontraktu historii; żaden test Rust/Chromium nie
  został zaliczony w tym przebiegu. Root połączył też równoważne nestedif
  shared/chats w let-chain przed Clippy. Następne all-targets sprawdzenie w toku.
- Pierwszy all-targets Clippy po poprawce borrow zakończył się na jednej uwadze
  cloned_ref_to_slice_refs w nowej regresji reorder (controls/tests.rs583).
  Porównanie używa teraz slice::from_ref zamiast zbędnego klonowania, z tą samą
  asercją. Nie dodano suppression. Pełny browser suite ponowiony na poprawionym
  fixture i teście; Clippy zostanie ponowione po ewentualnych wynikach runtime.
- Pełny relay drugiego przebiegu (source
  7775e28ece2920a580ed68deafc54eb52df329a3bc22a2fbb761b02f44a744df):
  Rustcompile1m58s, 244PASS/3FAILED/9ignored (78,97s). Nowe transport/shared/Store
  views testy przeszły. Przeoczenie root: starsze fixturev24/v25 przy cofaniu
  bieżącej bazy nie usuwały nowej kolumny views, więc ponowne migration27 dostało
  duplicatecolumn. Dodano DROPviews do dokładnego odtworzenia starego schematu.
  Nativegranttest oczekuje teraz wersji27; futureversion refusal test ustawia
  wersję bieżącej bazy+1 zamiast stałego27. Żadna ochrona migratora nie została
  poluzowana. Chromium/runtime dalszych pakietów nie ruszyły po relay failure;
  pełny suite ponowiono po poprawce fixture.
- Końcowy pełny browser source
  56e4469172ef07f522c14984232e1ed614e6f0e50bf80f29734642161db63682:
  webbuild698ms +Rustcompile1m57s PASS. Relay247PASS/9ignored (78,66s), identity1,
  runtime54PASS (10,96s), terminal27PASS (0,02s), usage37PASS (10,03s).
  Normalny pinned SDK verifier nowego6tracefixture równieżPASS w pełnym runtime.
  Wszystkie8ChromiumPASS (44,11s), w tym pierwszy real history UI >900KiB,
  klawiatura, strona przedRPC, utrata oryginalnej ramki i jawne odświeżenie,
  live interleaving, duplicate-free pełne21/22tury, offlinecompletion/reconnect
  nowycursor/ograniczonytail/noautofetch i wyczyszczenie wszystkich stron po revoke.
  Peak3221225472, memory.max237, oom/oom_kill=0: reclaim przy limicie bez kill.
  Kod po tym source niezmieniany; Clippy tego samego snapshotu w toku.
- Końcowy Clippy tego samego source
  56e4469172ef07f522c14984232e1ed614e6f0e50bf80f29734642161db63682 PASS:
  relay/terminal/runtime/identity/domain/protocol/usage all-targets offline/locked
  --no-deps -D warnings,1m20s, peak1319706624, max/oom/oom_kill=0.
- Cały odbiór paginacji:247relay/54runtime/27terminal/37usage/1identity +8Chromium,
 72frontend/TypeScript oraz107Python PASS. Sourcefreeze obejmuje finalne testy
  migracji/reorder, per-user read-only projection i historyfixture. Docs113 i
  vendor12 PASS; brak rzeczywistego GitHub Actions/IdP/provider/tailnet odbioru.
  Nie zmieniono vendora, nie było commitów/pusha ani realnych operacji kont.
- Formalny licznik nadal14/68=20,6%: E3.4 otwarte dla idempotent REST admission;
  E3.3 aggregate/conformance, E3.7 oversizedcontent resources i dalsze E4–E10
  pozostają otwarte. Idempotent session create to następny konkretny przekrój,
  nie powód do uznania ukończenia całego E0–E10.
- Cleanup potwierdzony: dunlin-checks Stopped, default Running. Target95MiB,
  wolne8,0GiB. Nie budowano Rusta na hoście; artefakty kontenerów były tmpfs
  i zostały usunięte po suite. Cel pozostaje aktywny i nieukończony.

### Idempotentne przyjmowanie sesji (2026-11-11)

- Poprzedni obrót celu: progress — paginacja AHP/typedSDK, migracja27, wszystkie
  8Chromium i Clippy PASS. Pełny E0–E10 pozostaje aktywny; następna potwierdzona
  luka to utrata odpowiedzi REST create i nowa sesja po retry.
- Runtime implementuje durable admission ledger PK(device,requestkey), owner/
  workspace/project, SHA256fingerprint oraz immutablepublicSession receipt.
  AtomicIMMEDIATEtransaction tworzy sesję, lease i ledger; exactretry zwraca
  session i leaseNone, nigdy nie reacquire nawetpoInterrupted/recovery. Quota
  1024global/128device, failclosed/noeviction. PublicSession nie dostał Deserialize;
  prywatny AcceptedSessionDTO nie odtwarza runtime/authority z historycznegoJSON.
- Focused admissions suite source
  318df45459f0c1edd42045ee57691d5d95097b4b2029b1b98d6d9d824bfbdc9d:
 5PASS/54filtered, compile36,32s/test0,06s, peak1485074432,
  max/oom/oom_kill=0. Dowody: real2SQLiteconnectionrace/onelease, file-reopen+
  recoveryUnknown, lateSQL/acquirerollback, currentHumanowner/device/scope/fingerprint
  i quota, immutable retry przed capacity. Server/API integracja jeszcze w toku.
- Root dodał migrację28 (ADMISSION_SCHEMA) i nowy Store integrationtest upgrade27:
  zachowane views/negotiation/legacySession, pusty nowy ledger, acceptedrecord
  i ponowny file-reopen exactretry bez drugiego lease. Wszystkie stare fixture
  odtwarzającev5/v17/v24/v25/v26 usuwają również nową tabelę przed cofnięciem
  user_version; latestassertions zaktualizowano, futureversiontest dynamiczny.
- Root wskazał rzeczywisty lock-order edge: connection_permit bierze management,
  a managementstart bierze potem Store. Nowy create nie może trzymać Store przy
  connection_permit. Integracja używa firstfreshlookup→releaseStore→permits→
  secondfreshlookup+authority/registry/commit. CapturedTokioHandle i fallible
  settings/catalog/registry preparation mają poprzedzać durable creation;
  ExitGuard i spawn następują po zwolnieniu Store/actors.
- Browserhelper tworzy UUID/immutableJSON wyłącznie przy jawnym zamiarze, ręczny
  retry zachowuje key/body/scope. Currentidentitypreflight, blockednewintent po
  unknown, responseheader/bodyreplaymatch i historyczny acceptedSession wymagają
  AHP/GET bieżącego stanu przed promptem. Produkcyjny formularz tworzenia i katalog
  zatwierdzonych modeli/trybów nadal brak; nie wymyślono fikcyjnych opcji UI.
- Pierwszy frontendcheckpoint80/TypeScriptPASS; po drobnym decoderhardening
  phasestring-only i wzmocnieniu headermismatch assertion uruchomiono finalne
  frontend/typecheck. RealChromium clienthelper/REST acceptance (nie creationUI)
  jest gotowy na utratę202 i identycznyretry/jedenactor; wykonanie w toku.
- Final helper/context header checkpoint: frontend81PASS
  4aa60c9ccad101fcd5657ce5e1e10e512c5f2d98b7c184b275168cd1f91633fe,
  TypeScriptPASS8f7930f23c58b3c9cac60fe4b692aaef86e337af05dcbda515e3d3d1e13a11ca.
  Nowa regresja obejmuje cookie change po successfulpreflight; immutable expected
  user/device headers odmawiają admission pod nową tożsamością. API testy mają
  drugiego w pełni uprawnionego użytkownika i drugie urządzenie, bez nowej sesji/
  procesu przy mismatch. Headery optionalpaired dla kompatybilności, helper zawsze
  je wysyła. RuntimeScope mismatch ownedkey daje409, bez ujawniania obcych danych.
- All-targets checkpointcompile odkrył E0277 w SQL count fixture: rusqlite nie
  dekodujeusize. Helpercount ma teraz i64 (SQLitecount nieujemny), asercje te same.
  Root usunął też duplikowaną instrumentację actor-starts z Chromiumfixture,
  ponieważ centralny fakeACP już zapisuje marker raz. Obie zmiany tylko testowe.
- Pełny browser snapshot
  480a4ad036ffa500f57e49727ad28f94bf751b76dd3b5e817462a1e328308264:
  webbuild547ms +Rustcompile1m57s PASS. Relay258PASS/10ignored (80,54s),
  identity1, runtime59PASS (10,92s), terminal27PASS (0,02s), usage37PASS (9,86s).
  Wszystkie9ChromiumPASS (47,34s), w tym realbrowser lost202 +sameUUID/bodyretry,
  oneSession/ACPprocess/initialize +freshcurrentAHP przed jednym promptem.
  Helper source injected przez test: nie twierdzimy, że istnieje produkcyjny
  createform ani że helper jest w bundle UI (brak approvedprofilecatalog).
  Peak3221225472, memory.max313, oom/oom_kill=0 (reclaim bez kill).
- Po powyższym snapshotcie rozszerzono wyłącznie ignoredactualruntime test:
  firstPOST fixedkey, poSIGKILL/relaunchsameDB exactRESTretry→historycznyreceipt,
  count1session/peer-starts1 przez400ms; AHPcurrentInterrupted/Unknown/queued.
  Freshoddzielnycreate nadalstartujejednego nowegopeer. Test realbinary+bwrap
  teraz uruchomiony; nie jest zastępowany fakekernel adapterem.
- Pierwszy actualruntime source
  58725c3d16483bc591cb088958d337417703837c57cf59266b61391714e3bb68:
  productionbinary buildPASS1m46s, rustc testbinary zabitySIGKILL przez cgroupOOM
  przed wykonaniem testu. Memory.peak3221225472, max302/oom1/oom_kill1. To niePASS
  ani błąd testowego oracle. VM3905MiB ma311MiB used i3594MiB available po cleanup;
  zwiększono wyłącznie runtimecontainer3072→3328MiB (swap nadalrówna0), inne suite
  bez zmian. Pozostawiono CPU/PID/tmpfs/network/LSM i ten sam rzeczywisty test.
  Ponowienie w toku, bez zmiany aplikacji/sourcehash.
- Rzeczywisty actualruntime po zwiększeniu budżetu, bez zmiany aplikacji:
  source58725c3d16483bc591cb088958d337417703837c57cf59266b61391714e3bb68,
  productionbuild1m38s/testbuild27,28s, dokładnie1PASS/267filtered w0,75s.
  Peak3434418176 bytes (powyżej starego3GiB, poniżej nowego3328MiB),
  max/oom/oom_kill=0. RealSIGKILL/relaunchsameDB +RESTkeyretry zwrócił ten sam
  originalSession receipt zreplayedtrue; currentAHP Interrupted/Unknown/queued,
  DB1session ipeer-starts1 przed/po400ms. Świeży oddzielnycreate→peer-starts2,
  jeden nowy efekt; oldnonce iacceptedpromptretry bez powtórzeń. Oryginalny
  orphancheck przedcleanup pozostał. To produkcyjny relay+sandboxtrue+bwrap.
- Named AppArmor profile usunięto zdedykowanegokernela po tym runtime. Clippy
  finalsource (takisamcodehash) uruchomione zdefaultDockerpolicies, wynik w toku.
- Pierwszy final Clippy source58725c3... znalazł3uwagi:2collapsible_if w lookup
  oraz await_holding_lock w teście po jawnym drop(store). W teście użyto teraz
  bloku leksykalnego zwracającego current_phase, a w Service równoważnych let-chain.
  Nie tłumiono lintów. To jedyne zmiany po greenbackend/Chromium i realruntime;
  nie zmieniają predykatów, kolejnościlookup ani aplikacyjnego zachowania.
  Clippy all-targets ponowiono na finalnym kodzie, bez powtarzania pełnych E2E
  wyłącznie dla równoważnego zapisu warunków i zakresu blokady w teście.
- Final Clippy po równoważnych poprawkach: source
  94c160e15e46da482f40cd6ea6da70f2b2bd05ebb5fb64ca173c0ba42a584dd9,
  relay/terminal/runtime/identity/domain/protocol/usage all-targets offline/locked
  --no-deps -D warnings PASS1m16s, peak1314881536, max/oom/oom_kill=0.
  Jedyna różnica wobec realruntime58725c3... to opisane let-chain i testowy blok
  scope; produkcyjne predykaty/sideeffects bez zmian. Fullbrowser480a4ad... był
  przed ignoredtest extension, sam extension miał rzeczywisty realruntimePASS.
- Niezależny audyt oraz root completion review E3.4: wsparcie obecnych runtime
  mutacji ma trwałe identity/never-evict bounded ledgers, owner+epoch stop jest
  idempotentny w skutkach, readonlypages/replay/snapshot mają spójne cięcia/limity,
  actualrestart nie odtwarza ani actora, ani niepewnego promptu. Warunki audytu
  realruntime+Clippy spełnione; E3.4 oznaczone[x]. Legacy request bez klucza to
  świadomie nowy zamiar dla kompatybilności, nie bezpieczny retry bez tożsamości.
  HTTPfs429 pozostaje ograniczeniem transportu bez nowej mutacji.
- Formalny licznik15/68=22,1%. Całe E0–E10 nadal nieukończone i aktywne. E3.3
  fullconformance/aggregate, E3.7 createUI/content resources, E5.1 approvedprofiles
  oraz E4–E10 i realne tailnet/IdP/keyring/provider/macOS gates pozostają otwarte.
  Nie wykonano commitów/pusha ani rzeczywistych operacji kont lub modeli.
- Łączna weryfikacja przekroju:258relay/59runtime/27terminal/37usage/1identity,
 9Chromium helper/REST+existingUI, 81frontend/TypeScript,107Python, docs113,
  vendor12, fmt/diff orazClippy PASS; actualbinary+bwrapSIGKILL/replay PASS.
- Cleanup potwierdzony: dunlin-checks Stopped/default Running; namedruntime
  AppArmor usunięty przedClippy. Target95MiB, wolne7,8GiB. Brak hostowych
  buildów Rust; tymczasowe artefakty byływtmpfs i usuniętoje po suite.
  Cel pełnego E0–E10 nadal aktywny. Kolejne przekroje: profile agentów i realny
  katalog uruchomienia/UI oraz pozostały conformance AHP; wielehostów i workflow
  nie zostały zastąpione uznaniem ukończenia samych lokalnych sesji.

## 2026-11-11 — profile agentów, publikacja i rzeczywisty formularz tworzenia sesji

- Po poleceniu właściciela wykonano `cargo clean`: 1286 plików, 92,3 MiB;
  bezpośrednio potem `df` pokazał 7,9 GiB wolnego miejsca. Nie wykonywano hostowej
  kompilacji Rust. Odtworzono jedynie zależności Node w task scratch; Linux wymaga
  jawnych `--os=linux --cpu=arm64 --libc=glibc` przy przygotowaniu przez npm na
  macOS. Zależności pobierano z locked manifestów, z `--ignore-scripts`; bez
  lifecycle scripts, dostawców modeli i operacji kont.
- Trzy równoległe zadania dostarczyły: domenę i testy wykonania profilu, Store/REST
  oraz edytor/formularz i produkcyjny browser acceptance. Root zintegrował runtime,
  atomowy trwały pin, migrację29 i zachowanie starego admission fingerprint.
- Definicja schema1 ma niezmienne publikacje z digestem, szkic CAS, jawny harness,
  model/mode, instrukcje i wersjonowane literalne skills. Zapis/publikacja wymagają
  aktualnego ManageProject; odczyt ViewProject. Limity:128 szkiców/projekt,
  128 wersji/agent, 64KiB definicji,8 skills i16KiB sumy instrukcji. Audyt zapisuje
  `resource_id`, zachowując istniejący FK target_id użytkownika.
- Profile sesji wskazują dokładny agent_id/version/digest. Nie można nadpisać
  modelu/instrukcji dodatkowymi polami. Jedna transakcja zapisuje sesję,lease,
  receipt i profil; test późnego błędu SQL sprawdza rollback wszystkich zapisów.
  Pin jest dostępny także przy błędzie startu przedready. Nowa publikacja nie
  zmienia poprzednich sesji; exactretry nie tworzy procesu. Instrukcje/skills są
  tekstowym kontekstem każdego promptu ACP; bez niejawnej tury startowej ani
  obietnicy roli systemowej. Legacy prompt i fingerprint pozostają bez zmian.
- Runtime dopuszcza profil tylko dla aktualnie skonfigurowanej dokładnej komendy
  katalogowego harnessu. Wire IDs są konfiguracją, weryfikowaną przez ACP przed
  pierwszym promptem, nie listą wymyślonych obsługiwanych modeli. Custom command
  nie jest zgadywana jako katalogowy identyfikator. MCP/A2A/scoped secrets i
  raport testu capabilities pozostają oddzielnymi otwartymi punktami.
- Migracja28→29 zachowuje wcześniejsze przyjęcia i audyt; dodaje tabele agentów,
  atomowe piny i nullable resource_id. Zaktualizowano fixtures migracji5/17/24/25/
  26/27 oraz dodano dedykowany test28→29. API rewaliduje urządzenie/OIDC/politykę
  pod tą samą blokadą Store co dane; dotyczy również GET diagnostyki profilu.
- Niezależny review wskazał podwójny odczyt konfiguracji harnessu i brak świeżej
  kontroli urządzenia w starszym GET diagnostyki. Oba poprawiono przed końcowym
  snapshotem browser. Brak blokującego ustalenia dla immutable resolve,
  fingerprint legacy, atomowego admission ani migracji; baza jest zaufanym
  magazynem, a publikacja zapisuje JSON i digest razem.
- Dokumentacja kontraktu i granic: [agent-profiles.md](agent-profiles.md).
  Wyniki kompletnej weryfikacji i decyzja o checklist są dopisywane poniżej;
  sam opis implementacji nie oznacza PASS ani ukończenia całego E0–E10.
- Weryfikacja przed końcowym Clippy:
  - Domena/protokół:44+6 PASS; snapshot
    `a956e319529ca5b97d32714861a7790cf8e76b3b0daa7b7ca4a9c8cb0c1239ce`;
    peak555167744B, bezOOM.
  - Frontend86 PASS (w tym codec/CAS/pinnedretry), snapshot
    `d82c6d45a4014ae8d0537f133f2affbc944299c02dc620835aaef403708cf9c5`;
    TypeScript PASS, snapshot
    `4f56ded5520db53539fb922111a2dfd90e23cfbb76174a6072e0ccd0b1be2afd`.
  - Pierwszy test frontendu ujawnił import bez rozszerzenia w nowym module
    wykonywanym bez bundlera; poprawiono na teams.ts i powtórzono suite.
    Pierwszy browser build po clean zatrzymał brak natywnego bindingu Linux;
    przyczyną przygotowania zależności był brak `--libc=glibc`, nie aplikacja.
  - Pierwszy pełny backend:265 PASS/3 FAIL/10 ignored, snapshot
    `0b29d3a4014eafb7e4c25f159597911f0ed1fa0d663b0826a6f6be73798b554f`.
    Nowe fixture’y zakładały, że grant ManageProject omija role ceiling Member;
    produkcja prawidłowo odmawiała. Dwie pozytywne ścieżki poprawiono o jawny
    awans członkostwa do Admin, dodając osobne potwierdzenie odmowy Member.
    Trzecia regresja błędnie dekodowała tekstowy Axum422 jako JSON; sprawdza teraz
    rzeczywisty status i tekst. Bez zmiany/osłabienia polityki produkcyjnej.
  - Powtórzony pełny browser/backend, snapshot
    `33ad27a354f90419281f8adf3f1150052da1a5b57cabd2bb2bc9a48f00fcbde9`:
    build produkcyjnego web PASS, Rust compile2m20s;
    268 relay/10 ignored81,89s +1 identity +60 runtime11,45s +27 terminal
    +37 usage10,08s PASS. Migracje, revocation, atomic SQLrollback, dokładna treść
    kontekstu ACP, niezmienna stara wersja po publikacji nowej, startupfailure,
    receipt replay i legacyplainprompt są rzeczywiście wykonane.
  - Wszystkie9 Chromium PASS49,24s. Nowy scenariusz używa produkcyjnych kontrolek
    Agenci→zapis→publikacja→Sesje→wybór profilu. Nie wstrzykuje helpera TS.
    Rzeczywisty HTTP202 po commicie jest tracony; nawigacja zachowuje niepewny
    zamiar, jawnyretry wysyła identycznebody/key/identity, baza i ACP mają jeden
    session/process/initialize, świeżyAHPready poprzedza jeden wykonany prompt
    zawierający rzeczywiste instrukcje i skill. Inne scenariusze observer/dispatch/
    controls/history/permissions/sharing/OIDC zachowane. Domyślna izolacja Dockera,
    peak3221225472B, max718, oom0/oom_kill0; bez poluzowań runtime sandbox.
  - W tym przekroju nie powtarzano osobnego real-binary+bwrap SIGKILL testu:
    jego wcześniejszy PASS pozostaje dowodem E3.6. Nowy profilowy przekrój ACP
    wykonuje lokalny fixture; nie jest odbiorem rzeczywistego dostawcy/harnessu.

- Końcowy Clippy all-targets (relay/runtime/terminal/identity/domain/protocol/usage,
  offline locked no-deps -Dwarnings) PASS1m22s, ten sam snapshot kodu
  `33ad27a354f90419281f8adf3f1150052da1a5b57cabd2bb2bc9a48f00fcbde9`
  co pełny browser/backend. Peak1369632768B, bezOOM; bez dodatkowych zmian kodu
  po tym zestawie weryfikacji. Format/diff-check oraz docs114 PASS.
- E5.1 oznaczono [x] na podstawie domeny, Store/REST, rzeczywistego procesu ACP
  i produkcyjnego UI. Formalny checklist:16/68 =23,5%; to liczba ukończonych
  punktów, nie estymacja ważona nakładem. E5.5 pozostaje otwarte: pełny raport
  testu definicji/capabilities i odbiór harnessu nie wynikają z publikacji.
  E3.3/E3.7, E4–E10 i bramki realnych kont/maszyn nadal wymagają pracy.
- Read-only mapa kolejnego przekroju E4 potwierdziła: obecny binding ma wyłącznie
  host local, desktop RemoteHost nie jest tożsamością workera, a uprawnienia
  workflow service principal nie stanowią credentials hosta. Pierwszy spójny
  przekrój musi objąć enrollment/approve, osobną scoped identity workera,
  uwierzytelnione WSS, heartbeat/drain/revoke, lokalnie egzekwowany lease oraz
  trwały inbox/outbox z unknown/reconcile po utracie ACK. Weryfikacja tożsamości
  Tailscale/Serve/Funnel wymaga oficjalnych źródeł i później osobnego odbioru
  dwóch rzeczywistych hostów. Mapa nie oznacza implementacji ani zamknięcia E4.
  Pełny cel E0–E10 pozostaje aktywny; bez commitów/pusha i realnych operacji kont.
- Cleanup: dedykowany dunlin-checks zatrzymany po suite; domyślny profil pozostał
  Running. Końcowy target96MiB, wolne7,6GiB; brak hostowych artefaktów kompilacji
  Rust. Snapshoty i kompilacje usunięto wraz z task scratch/tmpfs kontenerów;
  pozostają niewielkie zależności Node i logi w target/web-checks.

## 2026-11-11 — E4: rejestr hostów i oddzielna tożsamość enrollmentu

- Poprzedni przekrój zakończył E5.1. Kontynuacja przeniosła pracę na E4, zachowując
  pełny cel E0–E10. Trzy równoległe zadania dostarczyły domenę hostów, Store/REST
  oraz UI/Chromium. Root dodał migrację30, kompatybilność starych fixtures,
  inspekcję lokalnego LocalAPI i rzeczywisty worker HTTP w browser fixture.
- Rejestr jest scoped do workspace/ManageHosts. Definicje wiążą oczekiwany stable
  node ID z kanonicznym originem i polityką; nie zmieniają tożsamości wpisu.
  Jednorazowy bilet600s i credential workera mają odrębne prefiksy oraz256bit
  losowości; baza przechowuje wyłącznie hashe. Claim jest transakcyjny, epoch i
  credential revision chronią przed odzyskaniem starej authority po reenrollment.
  Raport capabilities jest ograniczonym, niezaufanym opisem. Control CAS revision
  jest oddzielona od capability revision, aby raport nie blokował operacji admina.
- Wszystkie hosty pozostają unverified/execution_ready:false. Nie ma jeszcze
  worker CLI/daemona, uwierzytelnionego WSS, zdalnych bindingów, lease guardian
  ani execution inbox/outbox. Bootstrap/report REST nie zastępuje tych elementów.
  Nie przypisano automatycznie hostów lub grantów wcześniejszym local bindings.
- Root i niezależny review wskazali dwa terminalne błędy nowego Store przed
  końcową weryfikacją: pełny zwykły audyt oraz wyczerpanie liczników mogły blokować
  odwołanie. Dodano obowiązkowy bounded latest revocation receipt w tej samej
  transakcji co czyszczenie tokenów, chroniony GET potwierdzenia, rezerwację
  ostatniej wartości licznika i stabilny exact revoked noop. Pełny zwykły audyt
  blokuje dodatni reenrollment, ale nie revoke. Testy obejmują100000 wpisów
  audytu, późny błąd SQL receipt i graniceMAX, bez pomijania aktualnych uprawnień.
- UI pokazuje prowizoryczny rejestr, statusy, politykę/metadane, drain/revoke,
  jednorazowy bilet i role. Root dodatkowo wskazał brak reakcji na utratę roli;
  dodano świeże GET po push revision, usuwanie hostów/biletu po odmowie dostępu
  i kontrolę generacji odpowiedzi. Własny create push zachowuje istniejący
  pending ticket tego samego hosta/epoch po ponownej autoryzacji; GET nigdy nie
  odzyskuje sekretu. Jawne odświeżenie go usuwa. Bez automatycznych mutacji/retry.
- Oficjalne źródła Tailscale sprawdzono ponownie: Serve identity headers nie
  identyfikują tagged devices, a DNS/certyfikat nie wykluczają Funnel.
  Dodany `host inspect-local` odczytuje wyłącznie jawnie wybrany Unix LocalAPI,
  z peer UID root/current user, limitem3s/1MiB/16KiB headers i sprzątaniem tasks.
  Wymaga dokładnego Self ID/DNS/Online oraz prywatnego background root proxy
  do numerycznego loopback. Brak wyłączenia Funnel lub przepisywania tras.
  Wynik jest historyczną lokalną obserwacją z application_authority:false;
  nie uwierzytelnia zdalnego hosta ani nie zmienia stanu jego rejestru.
  Użyto fixture Unix HTTP, bez połączeń do realnego tailscaled/tailnetu.
- Kontrakt, komendy i otwarte granice: [hosts.md](hosts.md). Migracja29→30 ma
  regresję zachowania AgentVersion oraz atomic profile/admission po reopen.
- Wczesny compile ujawnił nieobsługiwany rusqlite Row::get<u64> dla caprevision;
  poprawiono na checked i64→u64 z dozwolonym0. Pierwszy pełny browser compile
  snapshotc1077f8e1069eb8d92ab810a7dbe1f726abe35ba7a3ab72a3fb86e0f1e6fb3b6
  został zabitySIGKILL. Dmesg dedykowanej VM potwierdziłCONSTRAINT_MEMCG OOM
  rustc21:31:28, nie błąd testu ani globalnyOOM. VM miała3630MiB available,
  swap0. Budżet server/browser podniesiono3072→3328MiB, taki jak suite runtime;
  VM pozostała4GiB, bez innych zmian izolacji i bez zmian profilu default.
  Ujednolicono body type klienta HTTP, unikając podwójnej monomorfizacji.
  Wyniki końcowych suite są dopisywane poniżej po ich zakończeniu.
- Końcowe wykonania (przed Clippy):
  - Frontend94 PASS, source
    `0138be19c2a43913d034d040ac73a1dfe61da02f871aad6d34732a4fece5c9f8`;
    TypeScript PASS, source
    `b3bc65269ed0d599a77e88590020042f0b2be5589b2cb8a533b29dd3b9add280`.
    Obejmuje codec, CAS/noautoRetry, zachowanie ticket tylko przy świeżym pending
    pinie i fencing GET/mutacji. Launcher6 Python PASS po zmianie wyboru hosts
    i budżetu pamięci; docs115 PASS.
  - Pełny browser/backend i domena mają ten sam snapshot kodu
    `0c1b8d3e4c9e0d081b7ce4dea001ccdd8267ae300b0cf4acfc5c839a3114e45d`.
    Produkcyjny web build PASS, Rust compile2m10s;283 relay/11 ignored87,16s
    +1 identity +60 runtime10,72s +27 terminal +37 usage10,10s PASS.
    Domena48 +protokół6 PASS, peak558968832B bezOOM.
  - Wykonano rzeczywiste regresje równoległego claim na dwóch SQLite connections,
    token prefix/isolation, reopen, CAS/role/grant/device revocation,100000audit,
    terminalMAX, late SQLrollback i migracji29→30 z zachowanym profilem/przyjęciem.
    Pięć testów LocalAPI obejmuje rzeczywisty Unix HTTP/chunked, odmowę/redirect,
    limit odpowiedzi, zmianę Self ID, deadline, nie-socket, Funnel/overlays i
    dokładny loopback proxy. Nie są to połączenia do rzeczywistego tailscaled.
  - Wszystkie10 Chromium PASS47,23s. Nowy scenariusz używa produkcyjnych
    kontrolek Hosty; dedykowane HTTP worker/enroll i worker/report wykonuje Rust
    fixture bezCookie/Origin. Workercredential pozostaje poza browserem. Sprawdza
    pending→claimed→approve→report→drain bez zmiany CAS przez report→revoke,
    odmowę starego credential/biletu, ponowny bilet i utratę ManageHosts na
    widocznym ekranie bez kliknięcia refresh. Sprawdzono usunięcie listy/ticket,
    odrębnego viewera403 oraz brak sekretów w URL/storage. Wszystkie9 starszych
    scenariuszy zachowano. Peak3489660928B, max131, oom0/oom_kill0 przy3328MiB.
- Niezależny audyt zakresu E4.1 potwierdził rozdział oryginalnego planu: katalog,
  enrollment aplikacyjny, revocation i capabilities są tym punktem; transport,
  realne tożsamości sieciowe, leases i dwie maszyny należą do E4.3/E4.4/E4.7.
  Decyzja o checkbox następuje po końcowym Clippy, bez zamykania całego E4.

- Końcowy Clippy all-targets relay/runtime/terminal/identity/domain/protocol/usage
  offline locked no-deps -Dwarnings PASS1m25s, snapshot
  `007b6968532f72f8fac6203f66c51013ff6ed9076c87f33d265ee3cbe26ec10b`,
  peak1367433216B, bezOOM. Pierwszy Clippy wskazał wyłącznie collapsible_if i
  chunks_exact_to_as_chunks w inspektorze; zastąpiono je równoważnym let-chain
  oraz as_chunks::<2>().0 (CLI wcześniej wymaga dokładnie9 argumentów).
  Reszta kodu odpowiada wykonanemu browser/backend snapshot0c1; nie zmieniano
  zachowania po PASS funkcjonalnym ani nie powtarzano bez potrzeby całego suite.
  Format/diff-check PASS; końcowe linki/statusy dokumentacji sprawdzane ponownie.
- E4.1 oznaczono [x] w oryginalnym zakresie katalog/enrollment/revocation/
  capabilities. Formalnie17/68 =25% checklisty, bez ważenia nakładu. Całe E4
  pozostaje nieukończone: brak worker CLI/WSS, zdalnych bindingów, lease guardian,
  inbox/outbox i odbioru dwóch maszyn. Nie utożsamiamy approve z dowodem Serve,
  a samej lokalnej inspekcji ze zdalnym uwierzytelnieniem. E4.3/E4.4/E4.7 oraz
  E5–E10 i zewnętrzne bramki dalej wymagają pracy. Pełny cel pozostaje aktywny.
  Nie wykonano commitów/pusha, zmian realnego tailnetu, keyringu ani kont/modeli.
- Cleanup potwierdzony: dunlin-checks Stopped, default Running. Target100MiB,
  wolne7,5GiB. Kompilacje Rust były w ograniczonym VM tmpfs i zostały usunięte
  po suite; brak hostowych buildów Rust. Końcowe fmt/diff oraz docs115 PASS.


## 2026-11-11 — E4.3: prywatny controller–worker WSS

- Ukończono zakres E4.3: oddzielny ścisły protokół `dunlin.worker.v1`,
  dedykowane credential workera, dokładne host/workspace/enrollment/credential
  piny i reverse proof losowego nonce przypiętego do obu boot IDs i kanału.
  Dopiero potwierdzony callback otwiera trwały lease. Browser headers oraz same
  Hello/Ready nie wystarczają. Kontrakt: [worker-channel.md](worker-channel.md).
- Transport używa jawnego Unix LocalAPI, sprawdza Self/Serve i przypięty peer
  w netmap, przyjmuje wyłącznie101 ts-dial, TLS z publicznymi roots i dokładną
  nazwą. Brak DNS/LAN/env proxy/SSH fallbacku; HTTP200 Dial-Self odrzucone.
  Oficjalny bieżący handler/klient Tailscale sprawdzono w źródłach. Fixture
  używa dwóch syntetycznych LocalAPI i rzeczywistego TLS/WSS, bez realnego
  tailscaled/tailnetu; testowa CA istnieje wyłącznie przez cfg(test).
- Migracja30→31 zachowuje istniejące hosty/credentials i tworzy pusty trwały
  rejestr kanałów. Epoch wzrasta również po reopen; exact cleanup starego lease
  nie zamyka jego następcy. Credential revoke pozwala wyłącznie na ujemny cleanup.
  Heartbeat wymaga następnej sequence; exact duplicate nie przedłuża expiry.
  Guard credentials/lease/shutdown jest niezależny od zablokowanego I/O.
  Worker sprawdza monotoniczny deadline przed i po odczycie, więc opóźniony,
  już zbuforowany ACK po zatrzymaniu runtime nie przywraca lease.
- Worker CLI ma bounded stdin bez sekretu w args/env/logach, zeroizację,
  operator config jako zwykły plik bez symlinków/FIFO, bounded proof listener
  przed ekstrakcją body (8 requests/64 TCP) i niezależny network guardian.
  Niezależny review wskazał zawieszający FIFO oraz nieograniczoną agregację
  proof requests; poprawiono oba i dodano regresje realnego I/O.
- Chroniona diagnostyka GET zwraca raw zapis kanału i bieżący controller boot
  z reported_state_only:true; nie przypisuje zdrowia ani authority wykonania.
  Wszystkie hosty nadal execution_ready:false. E4.4 jest częściowo wdrożone
  wyłącznie dla kanału: brak execution dispatch/inbox/outbox/admission oraz
  lokalnego guardiana rzeczywistego zdalnego procesu. E4.2 i E4.7 pozostają otwarte.
- Weryfikacja pełnego przekroju (snapshot
  `52cad08caf05499b2691098a791a94ebb2c3666fe7bd7afab984e2cc87ab3f4e`):
  produkcyjny web build,312 relay/11 ignored,1 identity,60 runtime,27 terminal,
  37 usage oraz10 rzeczywistych Chromium PASS. Domena48/protokół10 PASS.
  Relay zawiera pięć fixture: pełny handshake/heartbeat/revoke, zmiana Funnel
  po przyjęciu, odmowa niezaufanej CA, odmowa błędnej nazwy TLS mimo zaufanej
  CA oraz odmowa lease po zmodyfikowanym nonce callbacku.
- Późniejszy snapshot
  `8cebaccafe8b962466fa3314cf655c5a3cea4842fcd42584bb545ccb7c0133cd`:
  produkcyjny build,34 testy worker_ i rzeczywisty ignored runtime restart
  z bwrap PASS. Obejmuje dodane później raw diagnostic API i trzy regresje
  deadline/control-frame streak, realny binarny CLI z FIFO i brak echa sekretu,
  a następnie SIGKILL/relaunch tej samej DB bez automatycznego retry promptu.
  Końcowy Clippy all-targets offline locked -Dwarnings potwierdzono exit0
  przez uchwyt procesu72278 po cargo clean. Wcześniejszy Clippy wskazał
  unused diagnostykę/useless borrow i duży error wymagany przez zewnętrzne
  callback ABI: dodano rzeczywisty chroniony endpoint, poprawiono format i
  wąski opisany allow wyłącznie dla tego callbacku.
- Pierwszy build CGU1 osiągnął limit3328MiB i OOM. CGU8 przy jednym jobie
  przeszedł bezOOM (pełny suite peak2658971648B, runtime2763722752B); nie
  zwiększano RAM VM ani nie używano hostowego kompilatora Rust. Jeden pełny
  wcześniejszy suite przerwano celowo podczas compile dla poprawek review;
  nie jest liczony jako PASS. Named AppArmor po runtime został usunięty.
- Formalnie18/68 =26,5% checklisty bez ważenia nakładu. Cały cel E0–E10
  pozostaje aktywny i nieukończony. E4.3 dotyczy Unix system daemon adaptera;
  realne policy/tagi, dwie maszyny, macOS GUI adapter/odbiór platform i realne
  konta pozostają osobnymi bramkami. Bez commitów/pusha i zmian realnych kont.
- Na wyraźne polecenie użytkownika wykonano cargo clean:1313 plików/101,9MiB,
  wolne2,6GiB. Usunięto również target z logami poprzednich suite; powyższy
  zapis opiera się na wcześniej odebranych wynikach oraz exit0 końcowego
  Clippy. Nie przedstawiamy usuniętych logów jako nadal dostępnych artefaktów.


## 2026-11-11 — E4.5: diagnostyka kanału w panelu hostów

- Dodano jawny odczyt chronionego GET diagnostyki do produkcyjnego panelu.
  UI pokazuje raw phase, IDs obu uruchomień, epoch/sekwencję i termin lease,
  datę odczytu oraz ostrzeżenie o innym controller boot lub terminie minionym
  według zegara browsera. Nie przypisuje zdrowia, wykonania ani aktualnej
  authority na podstawie persisted active. Brak cyklicznego REST pollingu.
- Ścisły decoder odrzuca cross-scope, niebezpieczne wartości liczbowe oraz
  dodatkowe pola authority/sekretów. Generacje/abort/deadline odrzucają stare
  odpowiedzi. Catalog/mutation invalidation usuwa odczyt, a401/403/404 usuwa
  listę hostów i jednorazowy bilet; nic nie wraca ze spóźnionego GET.
- Frontend101 PASS (w tym7 nowych testów), snapshot
  `3f14cd6a83ec6f41f1fe3eec1f392923578b924be22e83f066e8cab72f83649b`;
  typecheck PASS `a9d151c5c397aa52796028dbb3eaaa91a47d75db18e9d39f49eb08b105d0b509`;
  produkcyjny build PASS
  `65ad9b6d1891e59096547fe59340407909ef245d754032a4c9678d39c0c1a7ea`.
  Logi bieżących wykonań: target/web-checks/channel-frontend.log,
  channel-typecheck.log, channel-build-linux.log. Scenariusz Chromium hosts
  rozszerzono o rzeczywisty odczyt channel:null i usunięcie panelu po utracie
  ManageHosts; jego nowe wykonanie pozostaje do zrobienia. E4.5 nadal [ ].
- Pierwsza próba suite zakończyła się przed testami błędem niewidocznego mountu.
  Po cargo clean odświeżono wyłącznie dunlin-checks i odtworzono cache offline.
  Pierwszy build miał macOS native dependency; ponowne npm ci offline z os=linux,
  cpu=arm64, libc=glibc odtworzyło właściwy pinned pakiet i build przeszedł.
  Nie zmieniano lockfile ani produkcyjnego kodu w celu obejścia tych problemów.
- Host ma ponownie mniej niż1GiB wolnego miejsca i około10GiB swap; zatrzymano
  dedykowaną VM między testami, bez zatrzymywania default. Cache bieżących
  testów ma95MiB, brak hostowych artefaktów Rust. Weryfikacja kolejnego kodu
  wymaga monitorowania rzeczywiście dostępnych zasobów.
- Rozpoczęto worker-owned persistent export/local-grant boundary E4.2.
  Docelowy kontrakt: [remote-bindings.md](remote-bindings.md). Katalog exportów
  nie zastępuje zdalnego bindingu, execution inbox/outbox lub guardiana procesu.
  Nie podniesiono formalnego18/68 i nie zamknięto E4.2/E4.4/E4.5.


## 2026-11-11 — E4.2: lokalne exporty i granty workera

- Zaimplementowano osobny WorkerStore, z prywatnym Unix state directory i bazą
  exports.sqlite3, przypiętą do obu stabilnych node IDs, workspace/host oraz
  enrollment/credential epochs. Inny scope jest odrzucany; controller boot
  nie jest trwałą tożsamością exporta. Brak automatycznego importu OS/native
  grantów lub nadania ich z controller/browser JSON.
- Operator CLI worker export list/register/rebind/revoke/grants/grant/ungrant/check
  używa bounded regular config, bez network/credential stdin. Fizyczne ścieżki
  pochodzą wyłącznie od lokalnego operatora. Output zawiera opaque IDs, digests,
  revisions i scope, bez ścieżek. Check mówi checkout_identity_current i
  grant_current, nadal execution_ready:false. Settings digest to jawnie
  zatwierdzony hash, nie dowód zastosowania konkretnego pliku konfiguracji.
- Export i human/service principal grant mają osobne CAS/tombstones. Execute
  wymaga Read; rebind unieważnia wcześniejsze granty, a revoke nie potrzebuje
  dostępnego FS. Ordinary audit ma limit4096, latest revocation receipts są
  oddzielne i bounded; terminalMAX pozostaje zarezerwowane do odmowy.
- Opaque candidate przechodzi walidację FS poza transakcją, potem świeży SQL
  cut authority. Candidate/authorize/revalidate mają jawne read transactions;
  lokalny CLI i daemon mogą używać niezależnych SQLite connections.
- Adapter obejmuje directory, normal Git oraz linked worktree z jawnie
  zatwierdzonym git-dir/common-dir. Pinowane są root/git/common/objects identity,
  sprawdzane pointer/commondir/backlink. Nie ma automatycznego external root
  expansion ani object alternates. Nie zastępuje to przyszłego sandboxa ani
  ograniczeń hooks/filters/includes/symlink I/O. Kontrakty:
  [worker-exports.md](worker-exports.md), [remote-bindings.md](remote-bindings.md).
- Niezależny review wskazał brak pinu objects i akceptację symlink root:
  poprawiono je przed testami. Dodano checked revision reads i coherent SQL
  cuts. Pierwszy produkcyjny compile snapshot
  `a6a0459a36e29d3de87a238e4ad14a957b107ea58c89de96a7ff60b3698bfaf1`
  FAILED na u64:ToSql. Root poprawił wszystkie binds produkcyjne i testowe
  na sprawdzony sql_revision→i64, również zwykły audyt i graniceMAX.
- Runtime snapshot
  `6230c4746a2bdd52d56092c7c0ba24821c59ddbbf4b769dadc9541523313935a`:
  produkcyjny build PASS2m44s,43 testy worker_ PASS12,48s (w tym9 nowych
  exportów), rzeczywisty binarny CLI i ignored bwrap restart PASS0,79s.
  Nowe testy obejmują scope/reopen/principal kinds, grant ABA/rebind, podmianę
  katalogu, revoke bez FS przy pełnym audycie, late receipt SQL rollback,
  terminalMAX, prawdziwe git init/worktree, objects swap/symlink, prywatny
  state/FIFO i strict CLI flags. Dwa SQLite connections rzeczywiście ścigają
  CAS i cofają/nadają grant między FS validation a fresh authorize.
- Binarne acceptance wykonuje register→grant→check→usunięcie checkoutu→revoke
  →odmowa check w osobnych procesach tej samej bazy. Sprawdza brak ścieżek
  w stdout oraz execution_ready:false; następnie zachowuje wcześniejszą
  regresję SIGKILL/relaunch bez ponowienia niepewnego promptu.
  Peak2843443200B, max0/oom0/oom_kill0. Named AppArmor usunięty po suite.
  Dowód: target/web-checks/exports-runtime-sql.log; wcześniejszy błąd compile:
  exports-runtime.log. Focused Chromium i końcowy Clippy są weryfikowane dalej.
- E4.2 pozostaje otwarte: nie ma jeszcze descriptor exchange przez WSS ani
  controller RemoteBindingRef/API/UI. E4.4 nie ma execution inbox/outbox,
  admission, trwałych prób i guardiana procesu. W remote-bindings.md zapisano
  następny przekrój DescribeExport i obowiązkową ponowną authority przy
  przyszłym wykonaniu; między dwiema bazami nie ma atomowej transakcji.
  Formalnie nadal18/68. Cały cel E0–E10 aktywny; brak commitów/pusha, zmian
  realnego tailnetu, kont, providerów lub keyringu.

- Focused Chromium na tym samym snapshot6230 PASS5,75s (compile3m18s,
  produkcyjny web build PASS). Rzeczywisty panel wykonuje jawny GET kanału,
  pokazuje channel:null bez deklarowania zdrowia, usuwa wcześniejszy odczyt
  po reenrollment i cały panel/ticket po utracie ManageHosts. Nie wykonuje
  automatycznych GET diagnostyki przy invalidation. Pozostałe wcześniejsze
  scenariusze hostów zachowano. Peak2781011968B, bez max/OOM.
  Dowód: target/web-checks/exports-browser.log. To zamyka oczekujące nowe
  wykonanie Chromium z poprzedniego wpisu E4.5, lecz nie całe E4.5.
  Docs118 i diff-check PASS; końcowy Clippy w toku.

- Końcowy Clippy all-targets relay/runtime/terminal/identity/domain/protocol/usage
  offline locked no-deps -Dwarnings PASS2m06s, snapshot
  `7839ebe8b21c35072f982e9d4e55589214a8ffe1dfda538b8db415c6168f5dcf`,
  peak1467600896B, bez max/OOM. Wcześniej wskazał wyłącznie stałe chunks_exact(2)
  w CLI; zastąpiono je as_chunks::<2>().0 po istniejącym sprawdzeniu parzystej
  długości argumentów. Jest to jedyna zmiana kodu po snapshot6230 testów
  funkcjonalnych, bez zmiany zachowania; nie powtarzano ich bez potrzeby.
  Dowód: target/web-checks/exports-clippy.log; pierwotny lint:
  exports-clippy-initial.log. Format/diff i docs118 PASS.

- Cleanup potwierdzony: dunlin-checks Stopped, default Running; named runtime
  AppArmor usunięty. Target/web-checks96MiB, wolne5,0GiB. Rust kompilowany
  wyłącznie w bounded Linux VM tmpfs, bez hostowych buildów. Zostawiono mały
  cache offline i logi jako dowód; następne cargo clean również je usunie.
  E4.3 [x], formalnie18/68=26,5%; E4.2/E4.4/E4.5 i cały cel pozostają otwarte.


## 2026-11-11 — E4.2: descriptor WSSv2 i obserwowane bindingi

- Zaimplementowano strict path-free DTO DescribeExport/ExportDescribed/ExportError.
  WSS wymaga pojedynczego dunlin.worker.v2 +Hello2 dla metadata. V1 zachowano
  jako diagnostic-only handshake/heartbeat, bez registry sendera i nowych
  komunikatów. HostCapabilities.protocol_version pozostaje1 (osobny rejestr).
- Worker multiplexuje heartbeat/ACK i metadata, z8 pending requests/2 FS jobs/5s.
  Optional exports_state_dir wskazuje istniejący jawnie zatwierdzony katalog.
  Odczyt nie tworzy bazy/schematu. Każdy job robi SQL→FS→freshSQL i mapuje
  wyłącznie scope/public summary/grant, bez paths. Permit blocking FS żyje
  do rzeczywistego zakończenia również po timeout/cancellation. Nie ma dispatchu.
- ControllerService koreluje request/channel/lease, scope obu node IDs, epochs,
  export/principal/revision. Wspólne8 permits obejmuje queued i pending, również
  po odłączeniu HTTP caller. Wynik to opaque ObservedExport z monotonicznym
  terminem5s; Store ponownie sprawdza proof i bieżący kanał przy commit.
- Review znalazł realny race: opóźniona rejestracja starszego kanału mogła
  nadpisać nowszy registry slot. Dodano fresh Store lease check +rosnący epoch
  przy insert oraz guarded cleanup. Test obejmuje opóźnione register i teardown.
  Dodano także jawne deadline checks po await i końcowym SQL check: buforowana
  odpowiedź po zatrzymaniu runtime nie dostaje nowego terminu.
- Migracja31→32 tworzy bounded16 bindingów/projekt i latest revocation receipt.
  Stare local targets i worker fences są zachowane, bez automatycznego remote
  bindingu. Poprawiono wcześniejsze fixture migracyjne i latest assertions.
  Nowy controller Store ma CAS/tombstones/hidden controller node pin, dokładny
  Human/Read i current channel proof; bez caller-supplied descriptor/paths.
  Revoke nie wymaga FS/kanału, zwykły pełny audyt nie blokuje latest receipt.
- HTTP list/create/save/revoke wymaga ManageHosts+ManageProject. Przed RPC
  przypina project authority; po RPC rozpoczyna SQLite cut PRZED ponownym
  odczytem cookie/device/OIDC/host policy. Brak await w końcowym cut. Role/grant
  ABA, zewnętrzne SQL device revoke, failed commit i brak premature catalog
  notification mają dedykowane regresje. Zwykły local binding nie jest zmieniany.
- Panel w projektach zespołu pokazuje wyłącznie observed metadata. Wymaga
  jawnych host/export/revision, ma CAS baseline, fences dla starych odpowiedzi,
  manual read po unknown i offline revoke. Nie zmienia celu istniejącego
  formularza LOCAL sesji ani nie nadaje execution_ready.
- Frontend108 PASS, typecheck PASS; domena48 +protokół11 PASS, peak720359424B
  bezOOM. Logi: target/web-checks/bindings-frontend.log, bindings-typecheck.log,
  bindings-domain.log. Pierwszy pełny compile snapshot
  `b688244268eff3b982f8606242a5f2f4942ac1becac955a25c21a3481442cc26`
  FAILED na przeniesieniu scope z testowego Requests implementującego Drop.
  Zmieniono na clone; równolegle wprowadzono wyżej opisany jawny deadline test.
  Powtórny pełny suite jest w toku w bindings-full-fixed.log; jeszcze nie PASS.
- Audyt oryginalnego E4.2 wskazał dalsze braki: typed BindingRef i resolver
  exact ID/revision/no-local-fallback; explicit linked-metadata grants/sandbox
  roots w local standalone (remote adapter ich nie zastępuje); pozytywny
  linked-worktree reopen→WSS→binding oraz denial podmiany metadata. Nie
  przenosimy tych wymagań do E4.4 dla podniesienia checkboxu. Formalnie18/68;
  E4.2 i cały cel pozostają aktywne/nieukończone. Realne dwa hosty/policy w E4.7
  oraz runtime execution/admission/guardian w E4.4 nadal otwarte.

- Powtórny snapshot
  `849bd097713c39879f2d7f6ddd10c5fda22d6a2a2b26e8bd6773942a62c4dc84`
  zbudował relay i wykonał347 testów PASS; jeden wcześniejszy test guardów
  nadal błędnie uważał v2 za unsupported i FAILED,12 ignored. Wszystkie nowe
  Store/HTTP/WSS/migracja/deadline/legacy-v1 pozytywne testy przeszły.
  Zaktualizowano starą asercję do v3 unsupported i dodano jawne v1=false/v2=true.
  Suite bindings-full-final.log wykonuje końcowe wszystkie backend/browser checks.

- Końcowy pełny backend/browser snapshot
  `4c97a5b3c98aefdf0919d154fb0927b976a986d0b212abd33d5a7c295a8f91dc`:
  produkcyjny web build PASS;348 relay/12 ignored +1 identity +60 runtime
  +27 terminal +37 usage PASS. Relay105,72s; wszystkie11 Chromium PASS56,42s;
  peak2832777216B, max0/oom0/oom_kill0.
  Dowód: target/web-checks/bindings-full-final.log.
- Nowy Chromium używa produkcyjnego panelu i pełnego workera przez prywatny
  TLS/WSS fixture. Wykonuje save opaque target→obserwowany descriptor, sprawdza
  brak paths/secret storage i niezmieniony local binding, następnie lokalnie
  odwołuje worker grant i rozłącza WSS. Historyczny zapis nadal jest wyraźnie
  observed-only; offline revoke CAS działa bez ponownego Describe. Cofnięcie
  ManageHosts i osobny viewer403 usuwają metadane. Wszystkie10 wcześniejszych
  scenariuszy zachowano i ponownie wykonano. Nie jest to realny tailnet ani
  zdalny harness. Runtime/bwrap i końcowy Clippy weryfikowane osobno.

- Runtime produkcyjny build tego samego kodu4c97 PASS1m58s, lecz link binarnego
  testu zakończył się SIGBUS w rust-lld. Nie zaliczono runtime suite jako PASS.
  Dmesg dedykowanej VM nie wykazał OOM. Dodano runtime-only diagnostykę
  df/du/memory.events po build i na błędzie kompilacji testu; zachowano limity
  RAM3328MiB/tmpfs1280MiB oraz profile izolacji. Trwa kontrolowany przebieg
  bindings-runtime-diagnostic.log, aby sprawdzić quota zamiast zgadywać.
  Log pierwszej awarii: target/web-checks/bindings-runtime.log. Launcher6 PASS
  po dodaniu diagnostyki. Nie zmieniono produkcyjnego kodu dla tej awarii.

- Kontrolowany runtime z większym scratch1536MiB PASS na tym samym kodzie4c97:
  produkcyjny build2m54s, test compile58,77s,55 worker_ PASS19,58s (1 ignored),
  binarny CLI i rzeczywisty bwrap restart PASS0,76s. Peak2971910144B,
  max0/oom0/oom_kill0; RAM nadal3328MiB. Przy1280MiB linker dwukrotnie miał
  SIGBUS; diagnostyka po awarii pokazała1201884KiB użycia z1310720KiB, bezOOM.
  Zmiana wyłącznie scratch na1536MiB usunęła awarię. Chwilowego peaku zajętości
  w samym mmap linkera nie zmierzono, więc nie deklarujemy obserwowanego100%.
- Użytkownik ponownie zgłosił kończące się miejsce i zlecił kontrolę śmieci/buildów
  projektu. Zweryfikowano tree i Git: repo630MiB, z czego web/node_modules337MiB,
  target test cache96MiB, dist71MiB. Nie było dużego hostowego Rust target.
  Sprawdzono brak tracked files i aktywnych konsumentów wybranych katalogów.
  Usunięto wyłącznie ignorowane generated dirs: dist, web/dist, web/.astro,
  web/node_modules, target i Python __pycache__ w scripts/tests i scripts.
  Usunięte zależności/buildy są odtwarzalne z zachowanych manifestów/lockfiles.
- Zachowano18 małych logów (345553B) poza target:
  [dowód pełnego suite](evidence/2026-11-11-checks/bindings-full-final.log),
  [runtime PASS](evidence/2026-11-11-checks/bindings-runtime-room.log).
  Pozostałe wcześniejsze ścieżki target/web-checks/*.log z tego przekroju
  odpowiadają plikom o tej samej nazwie w implement/evidence/2026-11-11-checks/.
- Cleanup: projekt124MiB po usunięciu około506MiB generated artifacts.
  Wolne4,6GiB w chwili pomiaru. Named AppArmor usunięty, dunlin-checks Stopped,
  default Running. Żadnych tracked deletions ani zmian realnych kont/tailnetu.
  Końcowy Clippy tego przekroju JESZCZE NIE WYKONANY: priorytetem stało się
  zlecone przez użytkownika sprzątanie. Cel E0–E10 pozostaje aktywny; E4.2 [ ].
  Kolejny Clippy wymaga tylko pustego tmp-root w dedykowanej VM i registry cache,
  bez odtwarzania web/node_modules. Frontend/runtime/browser już zweryfikowane
  jak wyżej; Clippy nie może być oznaczony PASS na podstawie poprzedniego etapu.


## 2026-11-11 — wznowienie po cleanup, Clippy i jawny cel

- Po ponownym sprawdzeniu dysku/projektu odtworzono wyłącznie pusty tmp-root
  i uruchomiono dedykowaną VM; nie odtwarzano web/node_modules. Końcowy Clippy
  poprzedniego kodu snapshot4c97 PASS1m42s, peak1811546112B, bezOOM, all-targets
  offline locked no-deps -Dwarnings. Domyka oczekujące sprawdzenie sprzed
  sprzątania; [log](evidence/2026-11-11-checks/bindings-clippy.log).
- Nowy przekrój E4.2: typed BindingRef{id,revision}, ProjectTargetRef Local/Remote,
  resolver exact current target z własnym WorkerRead faktycznego użytkownika
  i fresh logical CreateSession authority. Brak dziedziczenia grantu admina
  zapisującego binding. Resolve jest obserwacją, bez process/admission mutation.
  Równolegle powstają jawne local revision guards i stable unsupported dla
  żądania Remote przed lookup receipt/local fallback. Prace i ich weryfikacja
  nie są jeszcze zakończone. E4.2 oraz cały cel pozostają otwarte.

- Źródła nowego target slice zamrożono: Domain BindingRef/ProjectTargetRef,
  Store/API resolver, jawny UI check oraz CreateRequest guards. Frontend115
  PASS, typecheck PASS, domena49/protokół11 PASS. Full backend/Chromium suite
  targets-full.log w toku; jeszcze nie oznaczono nowych zmian jako odebranych.
  Rozszerzony pełny WSS fixture ma realny git init/worktree, zapis i reopen
  WorkerStore, binding admina i HTTP resolve przez Member(View+Create bez
  Manage) z własnym Read rev3. Sprawdza revoke403, regrant200, objects swap409
  i device revoke podczas zablokowanego worker metadata RPC→final401.
  Niezależny review nie wykazał błędów scope/ABA ani deadlocku tego fixture.


## 2026-11-12 — jawny target: pełny backend i Chromium

- Snapshot `8eba9542e625705f996e81632bd51759b0e65b55d4fe596682292cd070c856a3`:
  build web i Rust PASS;360 relay/12 ignored +1 identity +60 runtime
  +27 terminal +37 usage PASS. Relay108,95s; wszystkie11 Chromium PASS84,47s.
  Peak2945806336B, max0/oom0/oom_kill0. Log targets-full.log w bieżącym
  target/web-checks (zostanie zachowany przed cleanup).
- Po raz pierwszy pozytywnie wykonano pełny linked-worktree chain: realny
  git init/worktree, jawne Git roots, zapis/reopen WorkerStore→TLS/WSS→binding
  admina→HTTP resolver Membera z View+Create bez Manage i własnym Read rev3.
  Revoke własnego grantu daje403, regrant znów200, podmiana objects daje409.
  Held Worker SQLite lock pozwala odwołać urządzenie po pierwszej autoryzacji;
  finalny cookie/device SQL cut poprawnie daje401. Saved mapping pozostaje
  niezmieniony i nie powstaje grant Execute/admission/process.
- Chromium potwierdził explicit Read check bieżącego usera, a po worker grant
  revoke przy nadal połączonym kanale odmowę tego samego ref. Historyczna
  konfiguracja nie znika z powodu samej odmowy WorkerRead; offline revoke
  oraz management/viewer fencing zachowane. Wszystkie wcześniejsze10
  scenariuszy nadal PASS. Frontend115, typecheck i domena49/protokół11 PASS.
- Po pełnym snapshot wprowadzono dwie małe delty: HTTP target validation
  przeniesiono przed experimental gate/fs_permit (Remote musi dawać501,
  invalid target400 także przy wyczerpanym FS), a test HTTP rozszerzono
  o te przypadki; UI prezentuje czas obserwacji czytelną datą zamiast raw epoch.
  Typecheck po zmianie prezentacji PASS. Runtime suite dodatkowo uruchamia
  named typed_target_http, aby wykonać nową regresję bez powtarzania całości.
  Runtime/Clippy finalnego kodu są w toku, jeszcze nie PASS. Launcher6 PASS.


### Końcowa weryfikacja jawnego targetu

- Runtime snapshot `04e9de95dfb9b9914cc995868f00de1ddaa3b6bbe0c2ec53c3e3781bbf6e6d5c`:
  produkcyjny build PASS,55 worker tests PASS/1 ignored; rozszerzony
  typed_target_http PASS0,13s, rzeczywisty binary CLI/bwrap restart PASS0,76s.
  Peak2924580864B, max/oom/oom_kill0; scratch1536MiB, po worker suite78%.
  Snapshot obejmuje końcowy HTTP guard przed fs_permit/experimental gate;
  późniejsza zmiana dotyczy wyłącznie prezentacji daty UI (typecheck PASS).
- Końcowy all-targets Clippy offline/locked/no-deps/-Dwarnings PASS2m03s,
  snapshot `a2cd3221682fe9dcc69fdfd0b57b49dba5805c27791c99e9d470e1f681012e37`,
  peak1565581312B, max/oom/oom_kill0. cargo fmt --all -- --check PASS.
  Nie twierdzimy, że pełny zestaw8eba został powtórzony po dwóch małych deltach;
  nowy guard HTTP wykonano w runtime suite, prezentację sprawdzono typecheckiem.
- Trwałe logi: [full](evidence/2026-11-12-targets/targets-full.log),
  [runtime](evidence/2026-11-12-targets/targets-runtime.log),
  [Clippy](evidence/2026-11-12-targets/targets-clippy.log),
  [frontend](evidence/2026-11-12-targets/targets-frontend.log),
  [domain](evidence/2026-11-12-targets/targets-domain.log),
  [końcowy typecheck](evidence/2026-11-12-targets/targets-typecheck-final.log).
  E4.2 pozostaje otwarte: local standalone metadata approval/native grants
  i sandbox consumers nadal nie są wdrożone. Wykonanie zdalne to E4.4.

- Po zachowaniu logów usunięto ignorowany target (96MiB) i Python __pycache__.
  Potwierdzono brak tracked files w usuwanych ścieżkach. Profil AppArmor
  dunlin-runtime-checks nie jest załadowany; dedykowana VM dunlin-checks
  zatrzymana. Profil default pozostaje Running i nie był zmieniany.


### E4.2 — wspólna tożsamość checkoutu przed lokalnym adapterem

- Wydzielono apps/server/src/checkout_identity.rs: czysty capture/revalidate,
  opaque CapturedCheckout, prywatny Record i niezależne błędy. WorkerStore,
  granty i prywatne otwieranie SQLite pozostają worker-local. Adapter błędów
  zachowuje dotychczasowe warianty i komunikaty. RepoKind/JSON/digest nie zmieniają
  formatu; brak nowych roots w sandboxie, FS, ACP czy browser API.
- Dodano stały historyczny wektor JSON/SHA-256, deny_unknown_fields na rekordzie
  i katalogu oraz sprawdzenie konwersji błędów workera. Niezależny read-only
  review nie wykazał regresji identity ani poszerzenia authority.
- cargo fmt i diff --check PASS. Pełny backend tego wydzielenia uruchomiony
  w dedykowanej VM, Rust artifacts wyłącznie w tmpfs; weryfikacja jeszcze trwa.
  Po wcześniejszym cleanup odtworzono wyłącznie pusty target/web-checks na logi,
  bez Node deps. To przygotowanie lokalnego adaptera; E4.2 nadal [ ].

- Pełny backend wydzielenia snapshot
  `443677bb651fb7e7b50b2a0d680893bf9eb035c779d0efcae390158817dffade`:
  363 relay/12 ignored +1 identity +60 runtime +27 terminal +37 usage PASS.
  Kompilacja2m22s, relay107,23s; peak3102310400B, max/oom/oom_kill0.
  Wykonano wszystkie3 nowe regresje oraz realny linked export→reopen→TLS/WSS
  →binding→HTTP resolve fixture. Clippy tego wydzielenia nadal w toku.

- Końcowy Clippy wydzielenia PASS1m58s, peak1541283840B,
  max/oom/oom_kill0; all-targets/offline/locked/no-deps/-Dwarnings.
  cargo fmt --all -- --check i dokumentacja118 plików PASS.
  Trwałe dowody: [backend](evidence/2026-11-12-targets/checkout-shared-server.log),
  [Clippy](evidence/2026-11-12-targets/checkout-shared-clippy.log).
  Po ekstrakcji nie powtarzano browser/runtime-binary suites, ponieważ nie
  zmieniono ich interfejsów ani zachowania; pełny backend obejmuje realny
  linked Git/WSS i HTTP regression. E4.2 oraz E0–E10 pozostają otwarte.

  Clippy snapshot: `443677bb651fb7e7b50b2a0d680893bf9eb035c779d0efcae390158817dffade`.

- Po końcowych sprawdzeniach ponownie zatrzymano tylko dunlin-checks i usunięto
  tymczasowy target/Python cache. Logi pozostają w implement/evidence.
  Default Running; nie uruchamiano hostowych buildów/testów Rust.


## 2026-11-12 — lokalne metadata approvals/grants i dostępne revoke

- Dodano niezależny native-project katalog approval/grant z CAS, native epoch,
  Read/Write, repo-wide write acknowledgment, ograniczonym audytem i osobnym
  ostatnim receipt odwołania. Lokalny CLI wymaga jawnej istniejącej bazy,
  odrzuca nieznane/zdublowane argumenty, nie emituje paths i nie uruchamia pracy.
- Niezależny review znalazł i naprawiono alias retarget: capture canonicalA
  nie wystarcza, jeśli project.path alias przełączono naB. Nowe revalidate_at
  ponownie wiąże oryginalną ścieżkę z zatwierdzoną tożsamością. Workerowy
  dotychczasowy canonical revalidate pozostaje zgodny.
- Naprawiono starszy ordinary local-binding revoke: pełny100000 team audit
  ani i64MAX nie blokują terminalnego odebrania authority. Receipt jest
  obowiązkowy, exact inactive CAS nie generuje zdarzeń, błąd SQL cofa całość.
- Migracja33 nie tworzy approvals/grantów dla dawnych native grants. Testy
  migracji obejmują zachowanie starego grantu i rollback wszystkich DDL/version.
  Root rozszerzył actual binary restart fixture o realny git worktree i CLI
  approve/grant/check/revoke, native/metadata/approval ABA, objects swap,
  odmowę brakującej bazy i redakcję ścieżek.
- Runtime launcher wykonuje teraz pełne testy relay/identity/runtime/terminal/
  usage przed binarnym restartem, zamiast wyłącznie filtra worker_. Launcher6
  PASS; runtime/prod build i pełne testy nowego przekroju w toku. Źródła snapshot
  `3f0aa49b99c26e5446e838dc851d5cd68db01bcd91ca7d0f25648a4fc7182428`.
  Brak nowych Node deps; buildy wyłącznie tmpfs dedykowanej VM.
- E4.2 pozostaje [ ]: nowy katalog jest konfiguracją i explicit check, a nie
  integracją nativeGit/ACP/interactive/jobs/PTY/process sandbox. Linked local
  binding pozostaje odrzucony. Nie zadeklarowano ochrony dawnych konsumentów.

- Pierwszy runtime snapshot3f0: prod build PASS,380 relay PASS/3 FAIL/12 ignored.
  Wszystkie20 nowych testów (10 model,4 CLI,4 ordinary revoke,2 migration) PASS.
  Trzy FAIL to pominięte fixture downgrade jobs25/ahp26/27: pozostawiały nowe
  tabele przed ponowną migracją. Zaktualizowano ich DROP oraz expected version33;
  kod produkcyjny bez zmian. Pełny runtime powtórzony na nowych źródłach,
  wynik jeszcze w toku. Pierwszy przebieg zatrzymał się przed binary acceptance;
  nie jest dowodem jego powodzenia. Scratch82%, brak OOM.

- Końcowy runtime snapshot
  `1cecb12a9e8432c9b8a2dbbea3aeb4194502b205667854159e446dbe7e57d89e`:
  produkcyjny build1m48s PASS, test compile38,73s;383 relay/12 ignored +1 identity
  +60 runtime +27 terminal +37 usage PASS. Relay106,51s. Wszystkie historyczne
  migracje25/26/27 naprawione i PASS; nowe migracje32→33/DDLrollback też PASS.
- Actual production binary CLI +bwrap restart PASS0,82s. Binary smoke wykonał
  realny Git linked worktree, approve/show/grant/read check, odmowę Write bez
  grantu, native grant ABA, metadata revoke/regrant, approval rebind invalidation,
  objects swap denial, offline ungrant/revoke i unknownflag/missingDB rejection.
  Summaries nie zawierają ścieżek i check ma execution_ready:false.
  Peak2936434688B, max/oom/oom_kill0; scratch82% po pełnych testach, bez zmiany
  limitów3328MiB RAM/1536MiB tmpfs. Końcowy Clippy w toku.

- Końcowy all-targets Clippy offline/locked/no-deps/-Dwarnings PASS1m20s,
  ten sam snapshot1cec; peak1553981440B, max/oom/oom_kill0. Formatowanie,
  diff-check i dokumentacja118 plików PASS. Brak zmian frontendowych;
  Chromium nie powtarzano dla tego przekroju CLI/Store.
- Trwałe dowody: [końcowy runtime i pełny backend](evidence/2026-11-12-local-metadata/local-metadata-runtime-final.log),
  [Clippy](evidence/2026-11-12-local-metadata/local-metadata-clippy.log),
  [pierwszy nieudany przebieg fixture migracji](evidence/2026-11-12-local-metadata/local-metadata-runtime.log).
  Nie oznaczono E4.2 jako ukończonego; integration consumers opisane w
  remote-bindings.md pozostają pracą do wykonania. Niczego nie commitowano/pushowano.

- Końcowe sprzątanie: named AppArmor profile usunięty, dunlin-checks Stopped,
  default Running bez zmian. Zachowano3 logi w evidence, usunięto target i
  Python cache (bez tracked files). Żadnych hostowych kompilacji Rust ani
  odtwarzania Node deps. Kolejny etap: typed native checkout authority i
  oddzielne process metadata roots z integracją wszystkich konsumentów.


## 2026-11-12 — native checkout authority i konsumenci Git/ACP (weryfikacja w toku)

- Dodano opaque/state-free NativeCheckoutAuthority: grant natywny, settings,
  checkout shape/dev/ino oraz opcjonalny approval/own metadata grant. SQL
  candidate→FS poza Store→fresh SQL finalize; zwykły checkout nie przejmuje
  później linked semantics. Batch i caller identity mają spójny SQL cut.
- NativeGitIo/Proof oddzielono od workspace NativeIo. Historia, operacje Git,
  impact/review/compare, one-shot prepared plans i check processes przypinają
  authority Read/Write; retry nie odświeża grantu. WorktreeRemove wymaga exact
  target own authority, nie grantu rodzica. Po świadomym usunięciu targetu
  pomijany jest tylko jego FS postcheck; SQL i surviving source pozostają.
- Git-init ma wyłącznie terminalny Directory→internal Git postcheck bez nowego
  reusable proof. Porównania publikują derived projects/grants w jednej
  transakcji ze źródłową authority. Cancel jest negative exact-owner operation,
  niezależną od cofniętego grantu/FS.
- Legacy ACP przypina własne checkout authority wszystkich workspace roots.
  Process sandbox dostaje oddzielne typed metadata roots (Read RO, Write RW),
  bez rozszerzenia additionalDirectories/FS/vault/attachments. Guardian ma
  fresh SQL i bounded FS worker poza Store; aktualny raw native alias nadal
  musi wskazywać zatwierdzoną canonical identity. Proces/cwd używa canonical root.
- Sandbox odrzuca metadata overlap z relay/credential/server roots i innym
  workspace leżącym wewnątrz metadata root. Mac privacy exceptions obejmują
  wyłącznie zatwierdzone metadata, a nie cały parent workspace. Rzeczywisty
  kernel test planowany dla Linux bwrap; Mac kernel coverage nie jest dowiedzione.
- ACP ProcessGroup jest rozbrajany po sygnale grupy, przed reap lidera, aby
  cleanup nie sygnalizował ponownie wykorzystanego PID. Testy actual child
  cleanup/metadata revocation są częścią nadchodzącego runtime przebiegu.
- Upload nadal używa plain workspace FS. Jego pomocniczy Git info/exclude write
  wymaga optional własnego Write proof i exact FD openat/O_NOFOLLOW regular
  bounded file, bez symlink/hardlink host-file escape i bez ogólnego path accessor.
- Bare/nested bare odrzucono przez pure capture jako nieobsługiwany adapter;
  brak .git nie może oznaczać zwykłego katalogu, jeśli Git wybierze bare metadata.
  Agent kończy scrub repository-routing env i jawny worktree override wspólnej
  biblioteki Git, zachowując desktop root discovery oraz object-only blame.
- Źródła większości konsumentów zamrożone. Format/diff-check i launcher6 PASS,
  pełny Rust/runtime/Clippy jeszcze nie zostały wykonane dla tego przekroju.
  E4.2 nadal otwarte: jobs/PTY/interactive/local linked binding i odbiór całości.

- Pierwszy immutable runtime7362: prod build PASS, test compile przerwał jeden
  stary coordination test bez nowego argumentu pustych metadata roots. Naprawiono
  wywołanie i usunięto nieużywany dawny helper upload/plainresume; SQL convenience
  wrappers używane wyłącznie w testach ograniczono cfg(test). Nie był to test PASS.
- Powtórny runtime d9dd: prod/test build PASS,397 relay PASS/7FAIL/13ignored;
  scratch84%, OOM0. FAIL: Gitinit nowego katalogu potrzebuje jawnego directory
  command builder (Git odrzuca --work-tree bez istniejącego GIT_DIR); missing
  linked approval powinno dawać403 zamiast400; stare epoch/stamp expectations
 403→409; derived linked project nadal wymaga operatorapproval/ownRead grant.
  Nie odświeżamy authority automatycznie tylko aby dawne testy przeszły.
  Kernel acceptance i workspace testy nie zostały jeszcze wykonane (cargo
  przerwało po relay). Kolejny pełny przebieg dostaje --no-fail-fast dla pakietów.
- Root dopisał wąski generic postcheck repeat-init istniejącego repo (tylko
  pierwszy Directory→Git używa terminalnego init helpera) i zabezpieczenie
  publicznej kopii Project w NativeGitIo przed zmianą ID/path względem proof.
  Nowe regresje oczekują wykonania w kolejnym snapshot. E4.2 nadal otwarte.

- Poprawki przed trzecim runtime: jawne fresh init bez --work-tree/GIT_DIR;
  repeat-init zachowuje stary Git identity; brak zgody filesystemowej mapowany
  na403, stary żywy stamp na409. Pochodny worktree nie dziedziczy approval:
  response compare/create ma git_access:requires_operator_approval, test najpierw
  dowodzi403, a dopiero po osobnym operatorapproval/ownRead dowodziodczytu.
  Nie wprowadzono automatycznej authority z grantu źródła.
- Trzeci pełny runtime obejmuje również nową regresję zmiany publicznej kopii
  Project w NativeGitIo i repeat-init, complete workspace package (6 nowych
  command tests) oraz dwa ignored runtime acceptance. Wynik jeszcze w toku.

- Trzeci runtime snapshot281e:405 relay/13ignored +1 identity +60runtime
  +27terminal +37usage PASS. Wszystkie wcześniej zgłoszone7 regresji naprawione.
  Workspace tests dotarły do import_limits_cover_the_whole_selection_and_nested_folders,
  gdzie proces dostał SIGXFSZ25: fixture robi dwa sparse set_len(512MiB+1),
  a runner miał per-file128MiB. To nie FAIL Git ani OOM; eventsOOM0, scratch84%.
  Podniesiono wyłącznie runtime RLIMIT_FSIZE do1GiB, zachowując limity RAM/tmpfs
  oraz128MiB w pozostałych suites. Nie pominięto fixture ani całego pakietu.
  Kernel acceptance nadal nie jest wykonane, bo przerwanie pakietu zatrzymało suite.

- Kontrolowana próba korekty fsize w readonly/network-none kontenerze z128MiB RAM
  i /tmp16MiB potwierdziła set_len dwóch536870913B plików przy allocated_bytes0.
  Pełny runtime ponownie uruchomiono na identycznym snapshot281e, bez zmian
  kodu/limitu RAM/tmpfs. Log native-checkout-runtime-accepted.log nadal w toku.

- Granice odbioru tego przekroju: checks mają nowy proof i guardian authority;
  nie zadeklarowano nowego kernel confinement ich komend. Dowód bwrap dotyczy
  ACP. Jobs/PTY/interactive i aktywacja linked local bindingu pozostają otwarte;
  istniejący project PTY nadal jest host shell. Git hooks/account/network policy
  poza przycięciem repo-routing env nie była zmieniana ani odebrana jako E10.

- Runtime z fsize1GiB: backend405 +1/60/27/37 ponownie PASS. Pierwszy sparse
  fixture import_limits PASS, lecz drugi rejects_cycles... tworzy sparse1GiB+1
  i dostał SIGXFSZ25. Root pominął ten drugi fixture podczas pierwszej korekty.
  Po sprawdzeniu wszystkich set_len w workspace (tylko dwa takie miejsca)
  runtime fsize ustawiono2GiB; RAM3328MiB, tmpfs1536MiB i pozostałe suites128MiB
  pozostają bez zmian. To kolejna korekta samego runnera na identycznym281e,
  bez zmian kodu i bez pomijania testów. Workspace/kernel jeszcze nie PASS.

- Pełny runtime z docelowym fsize2GiB na niezmienionym snapshot
  `281eab9bd4d819f26bbec0a1135f68d122f560c5ef714d7be60cd8823fc2930b` PASS:
  prod build1m47s, test compile43,77s;405 relay/13ignored +1identity +60runtime
  +27terminal +37usage +189workspace PASS. Relay108,20s, workspace98,29s.
  Oba sparse import preflight fixtures PASS bez pomijania. Scratch84%.
- Dwa actual runtime acceptance PASS5,00s: binary CLI/bwrap crash-restart
  oraz realny ACP metadata bwrap. Kernel potwierdził RO/RW metadata i odmowę
  source workspace; grant revoke, objects inode swap i alias retarget zatrzymują
  actual bridge i ciche dziecko (ticker nie rośnie po zamknięciu).
  Peer/PID namespace nie jest atrapą, a processgroup jest sygnalizowana przed reap.
  Nie wykonano dodatkowego testu metadata lingering, ani kernel macOS.
- Peak2963456000B, max/oom/oom_kill0, RAM3328MiB i scratch1536MiB bez zmian.
  Named profile wyładowano po runtime. Końcowy all-targets Clippy (również
  dunlin-workspace) w toku; jeszcze nie PASS dla tego snapshot.

- Pierwszy all-targets Clippy281e odrzucił linux_harness_command za8 argumentów.
  Dodano wyłącznie adnotację too_many_arguments z uzasadnieniem rozdzielenia
  workspace/metadata/broker roots, zgodną z sąsiednimi command constructors.
  Format PASS; brak zmiany zachowania po pełnym runtime. Końcowy Clippy nowego
  snapshot w toku. Nie powtarzamy runtime dla samej adnotacji lintowej.

- Końcowy Clippy PASS1m22s, all-targets/offline/locked/no-deps/-Dwarnings,
  równieżworkspace. Snapshot
  `49c53c073778d4c6b039886736e98370f124c5cbcb9bedc8491b07f547b2985d` różni się
  od przetestowanego281e wyłącznie adnotacją clippy i jej komentarzem.
  Peak1569689600B, max/oom/oom_kill0. Format/diff-check, launcher6 i docs118 PASS.
- Dowody: [pełny runtime + workspace + kernel](evidence/2026-11-12-native-checkout/native-checkout-runtime-complete.log),
  [końcowy Clippy](evidence/2026-11-12-native-checkout/native-checkout-clippy-final.log).
  Zachowano też początkowy compile failure, regresje/status fixes, SIGXFSZ128MiB
  i1GiB oraz pierwszy lint failure; żadnego nie oznaczono jako PASS.
- Niczego nie commitowano ani pushowano. E4.2 i cały E0–E10 pozostają otwarte;
  kolejny realny przekrój obejmuje jobs/PTY/interactive/local linked binding,
  z niezależnymi source/derived authority i ich unieważnianiem.

- Sprzątanie po odbiorze: zachowano7 logów352020B w evidence, named AppArmor
  wyładowany, dunlin-checks zatrzymana, default pozostaje Running. Usunięto
  ignorowany target i Python cache (brak tracked files), bez odtwarzania Node
  dependencies i bez hostowych kompilacji Rust. Nie zmieniano realnych IdP,
  tailnetu, providerów ani keyringu użytkownika.

## 2026-11-12 — E4.2: jobs, PTY, interactive i lokalny linked binding — weryfikacja w toku

- Jobs zapisują osobny source Read stamp i derived Write stamp; stare zadania
  bez proof wymagają nowego zatwierdzenia. Guardian i publikacja wyniku
  sprawdzają aktualne urządzenie, granty, ustawienia i tożsamość checkoutu.
  Linear status delivery otrzymał niezależny świeży SQL/FS/SQL fence.
- Project PTY oraz interactive actor zachowują własną authority i kończą pracę
  po zmianie grantu/checkoutu. PTY pozostaje host shell; brak nowej deklaracji
  kernel confinement terminala. Interactive przekazuje metadata tylko typed
  sandbox adapterem; linked checkout z wyłączonym sandboxem jest odrzucany.
- Lokalny binding ma jawny optional metadata approval revision, CAS i własny
  grant Read; browser pokazuje stan konfiguracji dla bieżącego użytkownika.
  Przygotowanie FS odbywa się poza Store, a publikacja ma świeży SQL cut.
  Migracja34 dodaje nullable job stamp i binding approval revision, bez
  automatycznego nadawania uprawnień istniejącym rekordom.
- Frontend121 oraz standardowy TypeScript PASS. Pierwszy Rust compile wykrył
  !Send siginfo_t utrzymane przez await; poprawiono zakres zmiennej.
- Kolejny pełny runtime snapshot
  `50a4bce9abae6ad58766d152693d3f28fe4adb843a5c19a604d9f4fc6a5bd1da`:
  430 relay PASS,2 FAIL,14 ignored; identity1/runtime61/terminal27/usage37/
  workspace189 PASS. Scratch84%, OOM0. Dwa FAIL: fixture odzyskiwania migracji
  usuwał drugi raz nieistniejące kolumny; linked actor fixture miał sandbox:false
  i poprawnie nie uruchomił procesu. Pierwszy fixture poprawiono, drugi
  przenoszony jest do rzeczywistej Linux bwrap suite. Nie osłabiono produkcyjnej
  kontroli sandboxa. Nowy pełny runtime/kernel/browser/Clippy nadal oczekują.
- E4.2 i pełny E0–E10 pozostają otwarte. Nie wykonano commit/push ani zmian
  realnych IdP, tailnetu, providerów lub keyringu.

- Domena49/protokół11 PASS; docs118 i kontrakt CI14 PASS. Zwykły job app-web
  pomija cały runtime prefix, a nie tylko restart, aby rzeczywiste bwrap testy
  wykonywał wyłącznie osobny job z dedykowaną polityką kernela.
- Pierwszy actual Chromium local_bindings PASS5,50s (peak2976059392B, OOM0):
  ordinary/linked explicit pin, CAS, reapproval/rebind, własny Read revoke,
  viewer read-only i revoked identity. Po tym snapshot dopisano przyczynową
  synchronizację E2E na katalog+GET zamiast wyścigu między save a remount;
  cała suite przeglądarkowa wymaga jeszcze przebiegu na końcowych źródłach.
- Read-only audyt ujawnił source defect w legacy ACP linger: bezpośredni
  permission response write do nieczytającego procesu mógł zatrzymać guardian
  w jednej gałęzi select. Trwa poprawka i rzeczywisty bwrap test disconnectu
  z otwartym metadata FD. Osobno korygowana jest kolejność terminate/disarm
  grupy interaktywnego actora przed reap lidera. Bieżący runtime snapshot346c
  jest wcześniejszy od tych poprawek; nie może stanowić ich dowodu.

- Runtime snapshot `346cd429e26e1eb6710861829f8d331c3c5579e440e74500b5866eec80dd5b5a`
  PASS:433 relay/15ignored, identity1, runtime61, terminal27, usage37,
  workspace189. Trzy actual Linux bwrap acceptance PASS5,69s: metadata kernel,
  binary crash/restart i nowy linked interactive ownWrite/revoke/regrant.
  Peak3013984256B, max/OOM0, scratch84%. Migracja34 rollback/preservation PASS.
  Dowód: [runtime przed poprawką linger](evidence/2026-11-12-checkout-consumers/checkout-consumers-runtime-accepted.log).
  Snapshot nie zawiera późniejszej poprawki linger ani interactive Group ordering.
  Te źródła są zamrożone; następne kontrole to all-targets Clippy i pełny runtime
  z czwartym rzeczywistym acceptance metadata_lingering.

- Pierwszy końcowy Clippy wykrył tylko let_and_return w targets.rs legacy
  fingerprint fixture; uproszczono return bez zmiany zachowania. Log zachowany.
- Runtime snapshot a6a0b3928555627b3f40c9d2776cb142bcf60e0b830fb4cb84a8c1f3521dfa39
  zatrzymano celowo (SIGTERM/143) podczas relay tests po wdrożeniu mocniejszej
  bariery testu: FIONREAD == F_GETPIPE_SZ potwierdza faktycznie pełny pipe przed
  revoke, zamiast zakładać blocked writer po100ms. Nie oznaczamy tego biegu PASS.
- Końcowy pełny runtime działa na snapshot
  `8585e3870159b8b34cac7a4958e3a4e5312fbe3960fa542fabfda447b16c5c40`;
  log target/web-checks/checkout-consumers-runtime-kernel-final.log. Obejmuje
  cztery actual bwrap tests, również nowy lingering held-metadata-FD/blocked-pipe.
  Wynik jeszcze oczekuje. Po nim wymagane końcowy Clippy oraz pełny Chromium,
  w tym zmieniona przyczynowa synchronizacja local-binding E2E.
- Dodano regularny cfg(macos) kernel test metadata_macos.rs do istniejącego
  cargo test na macOS: production sandbox-exec, własne Read/Write, rzeczywista
  odmowa odczytu/zapisu source/other project/fixture DB z EPERM/EACCES.
  Mac test pozostaje NIEWYKONANY lokalnie; brak hostowej kompilacji Rust.
  E4.2 nie jest zamknięte bez jego wyniku. Docs119, format i diff-check PASS.
- Zapisano proponowany (niewdrożony) następny przekrój E4.4 w
  [worker-execution-plan.md](worker-execution-plan.md): jedna trwała próba ACP,
  faktyczna polityka ustawień workera, inbox/outbox, execution lease, guardian
  i recovery Unknown. Sam kanał/binding nadal nie uruchamia zdalnej pracy.

- Końcowy Linux runtime snapshot8585 PASS:433 relay/16ignored, identity1,
  runtime61, terminal27, usage37, workspace189; cztery actual bwrap acceptance
  PASS7,41s. Nowy metadata_lingering potwierdza active prompt disconnect,
  otwarty writable metadata FD, rzeczywiście pełny kernel pipe przed revoke
  oraz zakończenie lease i brak dalszych zapisów po grant ABA. Peak3079434240B,
  max/OOM0. [Pełny log](evidence/2026-11-12-checkout-consumers/checkout-consumers-runtime-kernel-final.log).
  E4.2 pozostaje otwarte z niewykonanym nowym macOS kernel testem.
- Równolegle rozpoczęto E4.4: WorkerStore schema2/policy snapshot i osobne
  bounded execution DTO v3 są zaimplementowane i oczekują weryfikacji.
  Nowy policy plik ma raw-byte SHA256, pin export revision, atomowy rebind
  unieważniający granty i wymagany jawny ack shared metadata Write dla linked.
  Stare v1 peer reads nie migrują katalogu; stare metadata-only exports nie
  otrzymują polityki wykonania. Protokół ma Human Execute, initial receipt,
  lease renewal, cancel, reconcile bez znanego epoch i terminal outcome ACK.
  Hello/transport pozostają v1/v2; Remote nadal501, brak executora.
  Nowy policy/protocol kod powstał po snapshot8585, więc powyższy runtime nie
  jest dowodem jego testów. Końcowe Clippy, domena i backend/Chromium sprawdzą
  dalsze źródła. Test-only stare convenience wrappers ograniczono cfg(test).

- Weryfikacja rozszerzonego E4.4 source: Clippy zatrzymał się na E0277
  LowerHex dla nowego sha2 Output w policy.rs. Poprawiono parser i test na
  istniejące hex::encode. To nie był Clippy PASS; log zachowany w evidence.
- Pełny backend + wszystkie Chromium scenariusze działa na snapshot
  `3c7fd207e4f8d3211e9f299ea96f10601b4e9bc502772a6805fa98a2773a0e81`, log
  target/web-checks/checkout-consumers-browser-all.log. Obejmuje nowe7 policy
  tests oraz przyczynową synchronizację local-binding E2E. Wynik oczekuje;
  nowe6 protocol tests wymagają osobnej domain suite i końcowego Clippy.
  Host Rust target nie jest budowany; po kontroli zachowamy logi i usuniemy
  własny ignorowany scratch/Node cache. Dedicated VM nadal wykonuje kontrole.

## 2026-11-12 — E4.4: pasywne ledgery, macOS profile kernel proof i bootstrap

- Pełny backend/Chromium snapshot3c7f PASS:440 relay (nowe7 policy tests),
  identity1/runtime61/terminal27/usage37, wszystkie12 Chromium59,03s;
  peak3028811776B, OOM0. Domena49/protokół17 PASS (6 nowych DTO v3 tests).
  [Browser/backend](evidence/2026-11-12-checkout-consumers/checkout-consumers-browser-all.log),
  [domena/protokół](evidence/2026-11-12-checkout-consumers/checkout-consumers-domain-final.log).
  Ten snapshot poprzedza migracje35/3, nowsze transporty i Runtime.
- Podłączono pasywne ledgery do głównej Store schema35 i WorkerStore schema3
  oraz wspólne fingerprinty. Migracje nie uruchamiają procesu. Nowe rejestry
  zachowują immutable identity/receipt, exact pins, monotoniczny live lease,
  durable outcome/outbox i quarantine niezależną od terminalnego outcome/ACK.
  Nie odtwarzamy wykonywalnego handle z DB. Przy opóźnionym COMMIT stary
  wygasły lease nie może być przedłużony. Pending cancel nie tworzy worker epoch;
  guardian cancel i ACK są osobnymi negative producer cuts po user revoke.
- Pierwszy Mac driver zatrzymał się przed Rust (parent snapshot w /private/tmp
  nie jest mountem dedykowanej VM); kolejne wywołania mają --tmp-root
  target/web-checks. Drugi wykrył compile findings: sha2 LowerHex w generatorze,
  checked u64/i64 SQLite conversions i brak typu fixture descriptor. Naprawiono;
  dodano malformed persisted counter regression. Oba FAIL zachowano.
- Rzeczywisty profile proof PASS: źródła
  `b8c1786c376603b97a0b81d8c25a0e5d169f42a1e68563eca399edf877d21bdc`.
  Linux Rust tworzy dokładny pełny produkcyjny profil Mac z rzeczywistej
  authority fixture/own Read i Write. Host wykonuje niezmienione profile przez
  sandbox-exec/Python na rzeczywistym kernelu macOS, po unsandboxed control.
  Metadata RO/RW oraz EPERM/EACCES source/other project/real fixture SQLite PASS;
  protected files unchanged. Brak hostowego buildu Rust. Zakres to profil kernela,
  NIE pełne binarium macOS ani portable FD authority. Native Mac Rust test nadal
  niewykonany. [Receipt](evidence/2026-11-12-macos-profile/accepted/receipt.json),
  [generator i testy](evidence/2026-11-12-macos-profile/accepted/linux-export.log).
- Ten sam przebieg wykonał36 wybranych tests execution PASS3,27s (ledgery,
  migracje35/3, fingerprints i istniejące regresje) +1 profile export PASS.
  Wystąpiło6 ostrzeżeń o jeszcze nieużywanych adapterach Runtime. Nie ukryto
  ich allow/cfg; końcowy Clippy pozostaje otwarty do rzeczywistej integracji.
- Root dodał później guardian-only finish_stopped_execution: exact scope/fence,
  mandatory lokalna stop attestation, atomowy Unknown+outbox+reservation release,
  zachowanie już utrwalonego Completed. Dwa nowe tests SQL nie są jeszcze wykonane.
  Outbox filtruje current triple-fence PRZED paginacją oraz pomija niedostarczalne
  private/unconfirmed rows bez wysyłania treści. Historical/denied rows nie mogą
  zagłodzić aktualnego readable principal; named reconcile nadal zwraca odmowę.
  Nowa fairness regression również oczekuje VM. Nie ma kasowania historycznych
  identity ani resetu lease przy retry/ACK.
- WIP transport v3 ma domyślnie wyłączony worker_controller.execution_enabled
  i wymaga żywego producenta; worker wymaga jawnego lokalnego [execution] enabled
  i poprawnego Runtime przed Hello3. Brak downgrade przy błędzie opt-in.
  HTTP Remote nadal501. Kod transportu i procesowego Runtime powstał po b8c1;
  cały aktualny tree nie ma jeszcze końcowego build/test/Clippy. Runtime module
  i hidden local guardian CLI wymagają końcowego połączenia przez roota;
  ControllerExecutionService/AHP bridge nadal nie są podłączone.
- Tiny Linux bootstrap probe wykazał FAIL: PDEATHSIG outer + bwrap die-with-parent
  nie kończy namespace-init podczas startupu (payload jeszcze nie ruszył).
  [Zachowany failure](evidence/2026-11-12-worker-bootstrap/outer-death-failure/result.json).
  Wdrażana jest niezależna guardian-owned bootstrap, init pidfd handoff przed
  userprogram ACK oraz subreaper cleanup po śmierci helpera. Dwa małe128MiB
  Python kernel probes PASS dla trzech pojedynczych SIGKILL etapów; preferowany
  drugi wymaga authoritative waitid(P_ALL/WNOWAIT)->ECHILD, nie pustego /proc.
  To hipoteza kernela, NIE wykonana integracja Rust. Jednoczesna śmierć Main
  i guardian nadal wymaga containment/deployment gate, bez twierdzenia o pełnej
  odporności na dowolny SIGKILL. [Notatka i źródła](evidence/2026-11-12-worker-subreaper/design-note.md).
- Cały E0–E10, E4.2 i E4.4 pozostają otwarte. Niczego nie commitowano/pushowano;
  nie zmieniano realnego tailnetu, IdP, usług modeli ani credential stores.

## 2026-11-12 — E4.4: rzeczywisty worker, guardian i transport

- Zachowano cztery kolejne logi poza target, w
  [worker-runtime](evidence/2026-11-12-worker-runtime/). Każdy nagłówek
  zawiera hash dokładnego snapshotu źródeł; wynik nie obejmuje późniejszych zmian.
- Snapshot `bb827a4d57c65adb760c396d15192ebf997a43bffe4a22004ac614696dac54e7`:
  44 wybrane testy relay oraz 1 session-runtime PASS. Obejmuje nowe regresje
  finish_stopped_execution i outbox fairness oraz migracje35/3. Test rzeczywistego
  transportu pozostawał ignored w tej suite.
- Snapshot `ab298c5733bc9b456a7b8374199cfc6c8601058826ae9bd5f2b0e88aa5fd7cd0`:
  zwykłe testy 481 relay, identity1, runtime61, terminal27, usage37 i workspace189
  PASS. Etap kernela: 4 PASS, 1 FAIL (heartbeat fixture transportu). Fixture
  jawnie włączała execution, lecz raportowała sandbox None. Ustawiono w niej
  LinuxBubblewrap; domyślne metadata-only fixtures pozostają bez zmian.
  Cały przebieg był FAIL, nie pełny PASS.
- Kolejny focused snapshot `056194ffcc10c9befc21b0577b4ebb7921cec75042bfabd13a603fe9b8e651e1`
  zatrzymał się na kompilacji testu: prywatny moduł policy. Opaque PolicySnapshot
  udostępniono crate-public; wewnętrzny parser/polityka pozostają prywatne.
- Snapshot `9c648bd631929fb59c9fdf589d232dbc1a30574e62fd7b9ddea78396bb0df5c2`:
  focused runtime **5 PASS**, 0 FAIL, 5,69s. Rzeczywiste Linux bubblewrap/pidfd:
  execution, preflight, revocation, sześć faz pojedynczego SIGKILL Main/guardian
  oraz TLS/WSS transport/reconcile/heartbeat. Peak3313971200B, OOM0. To dowód
  wykonania worker runtime i transportu, nie pełnego kontroler→ACP→ACK pionu.
- Main ma już moduły Runtime i ControllerExecutionService oraz prywatne lokalne
  wejście execution-guard przed Tokio. Controller service startuje tylko przy
  jawnym execution_enabled i kończy się przed zamknięciem kanałów. Ta nowsza
  integracja oczekuje kompilacji i rzeczywistego testu pionu.
- Wprowadzono odpowiedź OutcomeAcknowledged po trwałym SQL ACK workera, osobną
  korelację ACK i ograniczone opaque potwierdzenie przy czyszczeniu ack_pending.
  Sam zapis do socketu nie wystarcza. Nowszy kontrakt oczekuje testu całego pionu.
- Trwa dodatkowa regresja faktycznie zablokowanego parent ACP stdin writer
  (częściowy zapis większej ramki do ograniczonego pipe), niezależnego guardian
  stop i braku późnego efektu. Wcześniejsze5 PASS nie pokrywa tej nowszej zmiany.
- Publiczny Remote/AHP nadal odmawia501: jeden wewnętrzny turn nie dowodzi
  obsługi zdalnej sesji wieloturowej. E4.4, końcowy Clippy i pełny aktualny tree
  pozostają nieodebrane. Jednoczesny SIGKILL Main+guardian wymaga zewnętrznego
  containment gate. Brak zmian w realnym tailnecie/IdP/credentials; brak commit/push.

- Następny snapshot `c543728aef8cec9b9423cd86380e78c37f0ca2447433ccfe7e2dee0ececd2a4c`:
  main-wired ControllerExecutionService kompiluje się; wszystkie5 testów lifecycle/
  policy PASS0,11s, peak3173675008B i OOM0. Nadal16 ostrzeżeń o API bez produktowego
  konsumenta. [Log](evidence/2026-11-12-worker-runtime/controller-service-compile.log).
  Regresja korelacji spontanicznego terminal Accepted z równoległym Renew/Cancel
  oraz nowy test całego TLS→ACP→durable ACK powstały później i nie są objęte
  tym wynikiem. Kontrola120 dokumentów i12 unit tests launchera PASS.

## 2026-11-12 — E4.4: pierwszy pełny wewnętrzny pion PASS

- Rzeczywisty ignored kernel test
  `worker_fixture::execution_vertical::production_browser_acceptance_runtime_remote_execution_vertical`
  **PASS0,66s**, snapshot
  `e3d3429c16c6f8da577b4c74fa932f7f484e353d286df290a49f94480963ce01`.
  [Dokładny log](evidence/2026-11-12-worker-runtime/remote-execution-vertical.log).
  Produkcyjne binarium i wszystkie testy kompilują się w Linux VM;
  memory peak3360079872B, OOM0. Normalnych unit tests ten focused przebieg
  nie wykonuje. Pozostają25 production/3 test warnings; Clippy nie jest odebrany.
- Test przechodzi przez rzeczywiste prywatne TLS/WSS, ControllerExecutionService,
  v3 Describe i binding, opublikowany profil, WorkerStore, runtime bubblewrap/ACP
  i canonical wynik Completed. Kontrolowana LocalAPI fixture zastępuje zewnętrzny
  tailnet, nie transport/proces/ledger. ACP jest kontrolowanym wykonywalnym
  programem fixture, nie rzeczywistym płatnym harness/providerem.
- Przypięte model/mode i dokładne literal instructions+skill docierają do ACP.
  Po identycznym retry zachowane identity, jeden start procesu i jeden efekt.
  SQL oracles: worker inbox1/outbox0/reservation0 oraz brak controller ack_pending.
  Potwierdza rzeczywisty trwały ACK, nie sam zapis ramki do socketu.
- Przegląd znalazł kolizję terminal Accepted/Outcome z równoległym Renew/Cancel/
  Reconcile. Rozdzielono spontaniczne events od RPC bez usuwania cudzej korelacji;
  malformed Execute acceptance nadal odmawiana. Dwie nowe regresje kompilują
  się w tym snapshot, ale nie zostały jeszcze wykonane.
- Root naprawił detached startup recovery: registration i otwarty receiver
  pozostają własnością rzeczywistego blocking SQL job, także po timeout/cancel
  wywołania. Nowa regresja causally polluje start z zablokowanym Store i wymaga
  odmowy kolejnego producenta do zakończenia recovery. Test kompiluje się,
  jego wykonanie jeszcze otwarte.
- Nowe blocked-parent ACP writer i version hang/flood kernel tests kompilują się,
  ale wymagają wykonania. Dalszy przegląd wykrył cleanup gap błędu waitid podczas
  version probe; poprawka ownership/cleanup powstaje po tym snapshot i nie jest
  objęta PASS pionu. Nie twierdzimy, że cały najnowszy tree jest już odebrany.
- Pełny Remote/AHP adapter pozostaje osobnym następnym krokiem opisanym w
  worker-execution-plan.md: atomic turn→attempt, journal przed ACK, Unknown
  blokujący kolejkę i rzeczywista semantyka rozmowy. Nie włączono HTTP Remote501
  ani nie oznaczono całego E4.4/E0–E10 jako ukończonego.

## 2026-11-12 — następny wspólny przebieg i atomowe primitives AHP

- Naprawiono własność version probe po waitid/read/reap errors: RAII StdChild
  i zweryfikowany pidfd, bounded EINTR retry, cleanup potomków na każdej ścieżce.
  Niepotwierdzone zatrzymanie blokuje startup w pojedynczej kwarantannie; brak
  Hello3 i brak fikcyjnego StopProof. Rzeczywiste hang/flood/EINTR fixtures mają
  potomka setsid, trzymany stdout i ticking FD; wymagają ECHILD przed wynikiem
  helpera i zatrzymania efektu po nim. Wykonanie oczekuje wspólnej suite runtime.
- Journal::claim_next_in_transaction utrwala claim/events/projekcję we wspólnym
  z callerem SQL cut i nie commit/notify. Dotychczasowy wrapper zachowuje własną
  transakcję; notify_committed_claim odmawia przed COMMIT. Trzy regresje obejmują
  autocommit/lease/authority denial, późny CHECK failure i rollback kolejki,
  eventów, projekcji i sequence bez notification, a także jawny postcommit.
- Store admit_remote_execution_in_transaction oraz record_remote_execution_in_transaction
  zachowują mandatory auth i odmawiają autocommit. Istniejące wrappery delegują
  do nich w dotychczasowej IMMEDIATE transakcji. Dwie regresje sprawdzają późny
  sibling SQL failure: rollback real command albo outcome/ack_pending, potem
  poprawny commit i canonical retry. Testy symulują błąd późniejszego konsumenta,
  NIE dowodzą jeszcze istniejącego pełnego turn→attempt adaptera.
- Wszystkie powyższe zmiany są zamrożone i objęte uruchomioną default runtime
  suite (normal tests + rzeczywiste kernel acceptance). Wynik i dokładny
  snapshot zostaną zapisane po terminalnym zakończeniu, nie na podstawie zamiaru.
- [Projekt trwałej sesji](remote-session-plan.md) rozdziela sesję/proces od tury
  i wymaga additive v4; stary v3 terminal nadal oznacza potwierdzone stop.
  Następny niewłączony model v4 nie jest automatycznym rozszerzeniem Hello3
  ani aktywacją Remote501. Dotychczasowe bramki E4.4/E3/E10 pozostają otwarte.

- Przebieg `1e2a8f28d47a7dc298dff86a21995e9896b147cf6d5ad68d6a37e031b03c44c7`
  zakończył się błędem kompilacji E0282 nowej root regression startup: kanał
  release miał wyłącznie Drop bez send, więc brakowało typu. Dodano channel::<()>.
  Żadne testy tego przebiegu nie są zaliczone. Zachowany
  [FAIL](evidence/2026-11-12-worker-runtime/full-runtime-startup-test-type-failure.log).
  Ponowiono pełną suite dla immutable snapshot
  `65529b3c942d1570045813e1d645a31efa15564feb203ee25214127cc38125e5`;
  wynik nadal oczekuje terminalnego zakończenia. Nowszy niewłączony model v4
  jest tworzony osobno po tym snapshot i nie będzie objęty tym wynikiem.

## 2026-11-12 — wspólna regresja worker/controller/Journal PASS

- Pełny runtime snapshot
  `65529b3c942d1570045813e1d645a31efa15564feb203ee25214127cc38125e5`
  **PASS**: identity1, relay497 (25 ignored w zwykłej suite), Journal64,
  terminal27, usage37, workspace189. Oddzielna rzeczywista kernel acceptance
  **11 PASS18,61s**, w tym dotychczasowe runtime/restart/linked checks, worker
  preflight/execution/SIGKILL/revocation, nowy parent ACP blocked writer,
  version hang/flood/EINTR z setsid potomkiem/ticking FD i ECHILD przed wynikiem,
  TLS/WSS transport oraz pełny controller→ACP→durable ACK pion.
  Peak3371327488B, OOM0. [Pełny log](evidence/2026-11-12-worker-runtime/full-runtime-corrected.log).
- Wykonano nowe regresje: startup cancelled waiter trzyma exclusive producer
  do rzeczywistego recovery, spontaniczny terminal nie konsumuje Renew/Cancel/
  Reconcile waitera, malformed Execute receipt nadal odmówiony, Journal claim
  rollback/notify i Store admit/outcome wspólne SQL cut. Wszystkie PASS.
  Nie jest to jeszcze dowód gotowego produktowego Remote/AHP consumer.
- Model v4 powstał później, osobno od snapshotu powyżej. Ma oddzielne session/turn
  identities, TurnCompleted bez stop semantics, SessionStopped, domain-separated
  wspólne hashe, typed hash-chain stream, bounded budgets i current-channel
  history replay/ACK zachowujące oryginalne events/hash/fences. Nie negocjuje
  Hello4 i nie uruchamia runtime. Osobna domain/protocol suite jest w toku dla
  snapshotu `b9a97f16f4394a90db33fed4e1e54c58c76ed26287f44265b4adc8120f864b4b`.
- Następna implementacja dotyczy odrębnego worker session ledger schema4 i
  rzeczywistego persistent ACP actora; bez fikcyjnego v3 attempt dla rozmowy.
  Migration/runtime/Hello4/Remote activation pozostają otwarte. Końcowy Clippy,
  native Mac binary, real harness/provider, dwa hosty i cały E0–E10 nieodebrane.

## 2026-11-12 — niewłączony v4 wire model i replay PASS

- Oddzielna domain/protocol suite snapshot
  `6c08d5120f1af5d78d03a0ff40991f77280170215a18e4006306ef0a3baa6167`
  **49 domain +32 protocol PASS** (15 nowych session/replay regressions),
  peak518774784B, OOM0. [Log](evidence/2026-11-12-worker-runtime/session-protocol-domain-corrected.log).
- Publiczny worker::session module jest additive i niewłączony. Hello nadal
  obsługuje wyłącznie istniejące wersje; stare dekodery odmawiają nowych ramek.
  Typy rozdzielają session-open/session fence od turn/receipt i SessionStopped
  od TurnCompleted; terminal turn nie powtarza pełnego tekstu po chunkach.
  Wspólne domain-separated SHA256 i dokładny field order mają niezależny
  Python-hashlib golden vector. Budżety/frame/input/chunk/safe integer,
  cross-session/boot i chain/digest/correlation mają regresje.
- Review znalazł brak historycznych delt po wymianie kanału. Dodano jawne
  ReplayStream/StreamReplayed z aktualnym transport channel, oryginalnymi
  hashed events/fence, dokładnym anchor/limit, contiguous predecessor, fixed
  through_sequence oraz stroną<=8/64KiB. Pusta strona z brakującą historią
  odmawia; brak cichego rewrite hash ani odtworzenia positive lease.
- Pierwsza suite snapshotb9a97f16: domain49 PASS/protocol31 PASS+1 FAIL.
  Fixture testowała legalne escapowanie wyłącznie TABami, choć walidator
  słusznie odmawia whitespace-only prompt. Dodano jeden niebiały znak przy
  zachowaniu limitu bajtów; nie osłabiono walidatora.
  [Zachowany FAIL](evidence/2026-11-12-worker-runtime/session-protocol-whitespace-fixture-failure.log).
- Dodane sha2/hex korzystają z już zablokowanych wersji0.11.1/0.4.3; bez
  aktualizacji pakietów. Codec nie dowodzi aktualnej authority ani StopProof.
  Brak v4 ledger/runtime/transport/Hello4/AHP activation; ich implementacja
  trwa dalej. Cały E0–E10, odbiór operatorski i wskazane bramki pozostają otwarte.

## 2026-11-12 — S2 trwałe sesje: implementacja ledger/ACP/guardian

- WIP worker_session ledger ma osobne tabele schema4 oraz opaque session/turn
  leases, admission/dedupe, stream/tombstones/ACK i negative stop API. Root
  podłączył migrację tylko do operator open/create: peer open_existing nadal
  nie migruje i czyta starsze1/2/3. Nowszy runtime/Hello4 nie jest aktywowany.
  Root dodał dwie rzeczywiste migration regressions: v3 canonical receipt/epoch/
  reservation zachowane przy upgrade, peer3 nie tworzy4, końcowy table collision
  rollback i naprawa. Starsze downgrade fixtures zrzucają też tabele4.
- Prywatny guardian manifest ma zamknięty owner Execution albo Session, z dokładną
  tożsamością i fence. StopProof nie przechodzi między wariantami/epoką/boot;
  dwie unit regressions oczekują wykonania. OS spawn/cleanup jest wspólny;
  v3 wrapper zachowuje lease i bootstrap. Nowy spawn_session mapuje monotonic
  clock próbkując CLOCK_MONOTONIC PRZED odczytem remaining/budget, z kontrolą
  deadline/budget przed fork. Własność po fork jest zawsze w Owned. Fallback
  po rzeczywistym stop wywołuje właściwy exact-fence ledger.
- Wąski SandboxMetadataRoots::from_worker_session przyjmuje tylko opaque lease,
  tak jak istniejący from_worker_execution. Brak ogólnego konstruktora z peer
  paths i brak rozszerzenia browser/native FS authority.
- Równolegle powstaje persistent ACP engine: jedna inicjalizacja/session-new,
  rosnące RPC IDs, kolejne prompty w tym samym procesie. To warstwa pipes,
  NIE launcher ani StopProof. Ciągły bounded reader i runtime guard mają
  rozdzielać idle/active także podczas awaited SQL hooks. ACP ordered-producer
  assumption pozostaje jawna (notyfikacje ACP nie mają turn IDs).
- Aktualne zmiany powstały po ostatnich PASS snapshotach i wymagają testów VM.
  Nie deklarujemy gotowego v4 runtime/transport/AHP ani sesji wieloturowej.

## 2026-11-12 — wznowienie projektu do E10 i odbiór fundamentów sesji

- Utrwalono wcześniej zakończony, lecz nieopisany w dzienniku przebieg
  `36dc804ffb1e89beaffd49a9c47e913cda87e7e8b553919ad632d97f66d18124`:
  relay525 (25 ignored), Journal64, identity1, terminal27, usage37, workspace189
  PASS; osobna kernel acceptance11 PASS18,85s. Peak3421372416B, OOM0.
  [Pełny log](evidence/2026-11-12-worker-runtime/remote-session-foundation-runtime.log).
  Wykonano12 regresji session ledger,12 samodzielnego ACP engine, migracje
  v3→v4 i rozdzielenie guardian owner/proof. Jest to odbiór fundamentów, nie
  wieloturowego sandboxa ani produktowego Remote.
- Podłączono `runtime::sessions` do kompilacji i życia Runtime. WIP actor
  łączy opaque session/turn leases, istniejący launcher/guardian i persistent
  engine. Admission i rejestracja actora należą do blokującego joba również
  po utracie waitera. Intent poprzedza spawn/prompt; tekst jest utrwalany
  przed końcem tury. TurnCompleted nie zwalnia slotu; cleanup wymaga StopProof.
  Osobny watchdog bez SQL obejmuje idle/active; disconnect/shutdown zatrzymuje
  również sesję. Otwieranie starego operator state nie migruje schema3.
- Nowy ignored kernel test `production_browser_acceptance_runtime_worker_session`
  wymaga rzeczywistego bwrap i produkcyjnego relay. Sprawdza pamięć pomiędzy
  dwoma promptami, jeden initialize/session-new/proces, retry bez dodatkowego
  efektu, trwałą rezerwację między turami oraz idle disconnect/revoke/expiry.
  Setsid potomek z trzymanym writable FD musi przestać pisać po stop proof.
  Test jest napisany; jego wynik nadal oczekuje terminalnego przebiegu.
- Pierwsza kompilacja integracji snapshotbd8857cc FAIL: metody poza impl oraz
  odczyt prywatnego pola Store. Naprawiono zakres i przeniesiono rozpoznanie
  starego schematu do API ledgeru. [Log](evidence/2026-11-12-worker-runtime/session-integration-first-compile-failure.log).
  Kolejny snapshot94f86201 FAIL przed testami: brak typu wyniku nieskończonej
  pętli actora (E0282). Dodano jawne Result<(),Code>.
  [Log](evidence/2026-11-12-worker-runtime/session-integration-second-compile-failure.log).
- Następny snapshot obejmuje dodatkowo nieblokujące rozpoznanie immutable
  session fence/root i spawn w blocking job oraz dokładne RPC correlation
  helpery/testy v4. Jego testy są w toku. Nie aktywowano Hello4/Remote501,
  nie zaliczono końcowego Clippy ani etapów E4.4–E10.

## 2026-11-12 — pierwszy rzeczywisty worker session sandbox PASS

- Focused runtime snapshot
  `275bdf4b33d357c83473aa00561ba2e7effb162e91f7476d66cac0eb85c348fb`
  kompiluje produkcyjny relay i wszystkie relay tests. Nowy kernel test
  `production_browser_acceptance_runtime_worker_session` PASS1,73s,
  peak3388706816B/OOM0. [Log](evidence/2026-11-12-worker-runtime/session-sandbox-first-pass.log).
  Cztery rzeczywiste scenariusze obejmują dwuturowy kontekst w jednym ACP
  procesie/session-new, retry Open/Submit bez dodatkowego efektu, rezerwację
  między turami, Close oraz idle disconnect/revoke/expiry. Setsid potomek
  z trzymanym writable FD jest zatrzymany przed trwałym stop/release.
  Focused suite nie wykonuje zwykłych testów ani nowego transportu v4.
- Snapshot `be430e9bb285835384d6bc6c0d47c222594c6a7f288b246213441f5c132a1184`:
  domain49/protocol33 PASS. [Log](evidence/2026-11-12-worker-runtime/session-correlation-domain.log).
  Nowa macierz10 RPC sprawdza dokładne receipt/ref/sequence/budget/fence,
  odmowę obcej operacji/zmienionej korelacji oraz to, że Event i SessionReady
  nie są potwierdzeniem oczekującego RPC. Nadal bez aktywacji Hello4.
- Nowszy WIP dodaje current-channel bounded session outbox z aktualnym Read
  oraz osobny opaque SessionTransport/producer/correlation. Pełna runtime
  regresja snapshotce7d3d0f jest w toku; nie obejmuje późniejszej integracji
  sieciowej ani dopisanych sześciu session SIGKILL scenarios.
  Publiczny Remote nadal501. Cały projekt do E10 pozostaje aktywnym celem.

## 2026-11-12 — session runtime/transport, Journal transactions i pełna regresja PASS

- Snapshot `14312f29757e0f52296729d2805066c5e2361a55bf0435628d8a8f3d62e1d3a0`
  **PASS**: relay532 (28 ignored w zwykłej suite), Journal67, identity1,
  terminal27, usage37, workspace189; rzeczywiste kernel acceptance14 PASS23,54s.
  Peak3376599040B/OOM0. [Pełny log](evidence/2026-11-12-worker-runtime/session-runtime-transport-full-pass.log).
  Nowe kernel scenarios: dwie tury jednego sandbox ACP, sześć single SIGKILL
  Main/guardian podczas bootstrap/handoff/running i prywatny TLS/WSS v4
  Describe→Open→Ready→dwie tury→stream→worker SQL ACK→Close/Stop.
  Zachowany context, dokładne retry bez dodatkowych efektów, slot między
  turami, source hash chain oraz zatrzymanie setsid potomka z held writable FD.
- Hello4 wymaga controller execution_enabled+persistent_sessions i osobnego
  żywego session producenta; worker wymaga enabled+persistent_sessions,
  rzeczywistego Runtime i operator-upgraded schema4. Brak downgrade.
  V1/v2/v3 zachowują odrębne ścieżki. Produkcyjny v4 producent nie jest jeszcze
  uruchamiany, publiczny Remote nadal501; fixture sama konsumuje stream i ACK,
  nie dowodzi jeszcze istniejącego controller Journal/AHP consumer.
- Journal ma caller-owned transition_timed/agent_update/harness_ready/interrupt
  oraz jawne notify po COMMIT. Trzy nowe regresje PASS: autocommit/current
  authority/lease denial, późny sibling CHECK failure z rollback transkryptu,
  projekcji i clocks oraz negative interruption. Stare wrappery zachowane.
- Pasywne controller session admission (bez produkcyjnej migracji) ma3 SQL
  tests PASS: atomowe journal/profile/admission/open rollback, retry bez nowego
  lease/komendy oraz odmowa autocommit/metadata-only/denied v4 producer.
  Nowsze turn mapping/stream→Journal/ACK methods powstały po tym snapshot;
  ich dwie dodatkowe regresje wymagają wykonania.
- Poprzedni snapshotce7d3d0f zatrzymał się przed testami na u64 ToSql w nowym
  outbox test. Naprawiono checked SQL conversion.
  [FAIL](evidence/2026-11-12-worker-runtime/session-outbox-test-sql-type-failure.log).
  Snapshotf71692c6 zbudował production binary, lecz test rustc został zabity
  przy cgroup OOM1. [OOM](evidence/2026-11-12-worker-runtime/session-transport-test-compile-oom.log).
  Runtime codegen8→16 pozwoliło zbudować testy: snapshot6aa61323 miał528 relay
  PASS+1 FAIL historycznej asercji, która nadal odmawiała v4; pozostałe suites
  PASS, kernel phase nie ruszył. [FAIL](evidence/2026-11-12-worker-runtime/session-full-legacy-header-expectation-failure.log).
  Poprawiono test: jawna pozytywna v4 parser assertion, nieobsługiwane v5 i
  oddzielne v4 opt-in denial tests. Po zmierzonym max193 przy limicie RAM
  oraz rozbudowie testów runtime używa32 codegen units; RAM3328MiB,
  scratch1536MiB i wszystkie polityki izolacji pozostają zachowane.
- Launcher12 unit tests, check_docs121 i tracked diff whitespace PASS.
  Końcowy Clippy, nowszy tree, produktowy Remote/AHP, Mac worker i E5–E10
  pozostają otwarte. Brak zmian realnego tailnetu/IdP/providerów/credentials.

## 2026-11-12 — controller stream/Journal/schema36 pełna regresja PASS

- Snapshot `c2fbd11cf9edca06dd95a8e8dccc362cec4dace21741d63c79a2cfec77f39f39`
  **PASS**: relay536 (28 ignored), Journal67, identity1/terminal27/usage37/
  workspace189 oraz14 rzeczywistych kernel checks PASS23,52s.
  Peak3370479616B/OOM0.
  [Pełny log](evidence/2026-11-12-worker-runtime/session-controller-stream-full-pass.log).
- Nowe controller schema36 tabele session/turn/stream mają atomowe admission,
  Open/Submit pre-send cuts, świeży Describe i mandatory v4/caller callbacks,
  trwały turn mapping oraz hash-chain inbox→Journal/SDK projection→ACK watermark.
  Testy obejmują late sibling SQL rollback, terminal przy brakującej delcie,
  duplikat bez drugiej projekcji, text przed RPC receipt, późny receipt po
  terminalu i ACK evidence denial. Unknown blokuje kolejny claim i zachowuje
  reservation do source-attested SessionStopped. Brak jeszcze produkcyjnego
  consumera tych primitives, więc nie twierdzimy, że browser Remote działa.
- Migracja36 zachowuje v3 command bytes i binding/authority, nie tworzy sesji;
  końcowa kolizja DDL cofa oba wcześniejsze tables i wersję. Starsze downgrade
  fixtures zrzucają też nowe tabele; wszystkie objęte tym snapshot tests PASS.
  Rezerwacje v3 i v4 liczą się we wspólnym controller host capacity cut.
- Odrębny domain/protocol snapshot
  `898b2c10a85972b4320b5266fc699da84587bd4c51c14a2a6ce5e03bba4c738a`:
  domain49/protocol33 PASS, peak523132928B/OOM0.
  [Log](evidence/2026-11-12-worker-runtime/session-negotiation-domain-final.log).
  Pierwsze ponowienie wykryło jeszcze jedną historyczną v3 test assertion
  oczekującą odmowy Hello4. Zamieniono nieobsługiwaną wersję na5; stare
  execution decoders nadal odmawiają v4 payload, a limity1/2/3 pozostają120/
  120/1024. V4 ma jawny limit4096.
  [Zachowany FAIL](evidence/2026-11-12-worker-runtime/session-negotiation-legacy-hello-fixture-failure.log).
- Nowszy tree dodaje oddzielną migrację37 zamiast zmiany odebranej schema36:
  pre-send dispatched marker, canonical stop_record, monotoniczne renew
  intents i negative recovery/reconcile. Stare interrupted rows konserwatywnie
  zachowują quarantine; tylko pending bez receipt jest proven never-dispatched.
  Recovery nie zwraca Open/Submit, nie odtwarza lease ani nie ACKuje brakującego
  outputu. Nowe3 regression tests i refactor współdzielonego Context wymagają
  wykonania. Produktowy producent/Remote501, final Clippy i E4.4–E10 otwarte.

## 2026-11-12 — recovery/schema37 odebrane, producent sesji w integracji

- Snapshot `4ee7ae754114ada7930aa4b12130ecdce69ff9652d2dbd14c17ce22dfe811082`
  **PASS**: relay539 (28 ignored), Journal67, identity1/terminal27/usage37/
  workspace189 i14 rzeczywistych kernel checks23,41s. Peak3349958656B,
  OOM0. [Pełny log](evidence/2026-11-12-worker-runtime/session-controller-recovery-full-pass.log).
  Obejmuje additive migrację37, negatywne recovery/reconcile oraz wspólny
  Context kontrolera. Nie obejmuje później dopisanego producenta sesji.
- Nowy `remote_sessions` integruje pojedynczego producenta v4, bounded admission,
  actor niezależny od HTTP/browsera, osobny guard authority, monotoniczne lease
  deadline, pre-send Open/Submit, serial claim i renew session/turn. Brak odpowiedzi
  nigdy nie powoduje ponownego Submit. Hash-chain stream jest projektowany do
  Journalu przed ACK; osobne sprzątanie replay/ACK/reconcile obsługuje również
  historyczny kanał bez przywracania positive authority. Te zmiany wymagają
  wykonania testów i nie są jeszcze odebrane.
- Dodano opt-in startup/shutdown i Remote/Profile REST z wymaganym kluczem,
  wspólne AHP queue/control gates, limit remote prompt16KiB i jawną odmowę remote
  tool approvals. Wyłączona konfiguracja zachowuje dotychczasowe Remote501.
  Nowy rzeczywisty REST→AHP WebSocket→WSS→sandbox→Journal test ma sprawdzić dwie
  tury w jednym procesie, idle renew, retry admission, projekcję i source stop/ACK.
  Odbiór tego pionu, UI wyboru Remote, Clippy i E4.4–E10 pozostają otwarte.

- Następny snapshot `de20fff51357fd94049c2ad9d05a0206a4366bb3a1b8fdb5a65f42e20fce6820`
  **PASS**: relay540 (28 ignored), Journal67, identity1/terminal27/usage37/
  workspace189 i14 kernel checks23,56s; peak3371016192B/OOM0.
  [Log](evidence/2026-11-12-worker-runtime/session-controller-actor-compile-full-pass.log).
  Obejmuje kompilację nowego modułu producenta oraz test cancellation-after-claim,
  lecz nie uruchamia jeszcze jego produktowego pionu. Późniejsza aktywacja w
  startup/REST/AHP i nowy rzeczywisty test mają odrębny przebieg w toku.
- Nowszy UI wybiera Local lub konkretny aktywny RemoteBinding, gdy serwer ogłasza
  gotowość kontrolera sesji. Zamiar przypina id/revision i identyczne bytes retry;
  katalog profili/metadata exportu nie stanowi dowodu dostępu do wykonania.
  Serwer nadal wykonuje finalny Describe i transactional authority cuts.
  Frontend, typecheck/build i browser acceptance wymagają uruchomienia.

- Pierwszy focused pion snapshot
  `925020854bb8f47e9b80b9e2f3c552a8d0bff2419061b7090c8aee6ab1606f03`
  skompilował się, ale **FAIL**: AHP404 mimo Remote REST202. Przyczyna: AHP
  upgrade oraz session capabilities sprawdzały wyłącznie lokalny experimental
  runtime. [Zachowany log](evidence/2026-11-12-worker-runtime/session-journal-ahp-activation-failure.log).
  Dodano opt-in zdalnego kontrolera do obu gates, zachowując wymóg transkryptów;
  start zdalnego producenta przy transcript opt-out jest teraz odmawiany.
  Nowy jednostkowy regression oraz ponowienie realnego pionu oczekują wyniku.
- Frontend122 testów **PASS**; TypeScript i produkcyjny build45 modules **PASS**
  (snapshot `66121a90e0c233e95c76c51c2b8e28254cffb5429a6bb5c35cf35f7d6c2a853a`).
  Build używa istniejącego readonly cache `target/web-checks/node-deps/node_modules`.
  Nie oznacza to odbioru nowego formularza w Chromium.
- Nowszy worker ma dodatkowy kernel flock na prywatnym state_dir przed boot
  recovery. Zapobiega uruchomieniu recovery przez drugi proces workera nad
  nadal żywym katalogiem pierwszego. Lock inode pozostaje po shutdown; FD ma
  CLOEXEC, wymagany owner/0600/regular/nlink1, symlink/FIFO/hardlink odmawiane.
  Dwa nowe tests (w tym rzeczywisty konkurencyjny proces) wymagają wykonania.
  Nie jest to gwarancja dla ręcznie sklonowanych katalogów/credentials; restore
  generation oraz pozostałe bramki E10 pozostają otwarte.
- Drugie focused wykonanie skompilowało produktowy backend, lecz **FAIL** na
  niepoprawnym kliencie testowym: `dispatchAction` wysłano jako RPC z id,
  zamiast notyfikacji wymaganej przez AHP. Serwer poprawnie zwrócił -32600.
  [Log](evidence/2026-11-12-worker-runtime/session-journal-notification-fixture-failure.log).
  Test teraz wysyła notyfikację i wymaga trwałego echo o dokładnym origin i akcji,
  bez rejectionReason. Ponowienie w toku; nie zaliczono produktu samą kompilacją.
- Drugie wykonanie miało peak3469910016B (bez OOM), zaledwie około17MiB poniżej
  limitu3328MiB. Dla rosnącego runtime/test binary ustawiono64 codegen units przy
  niezmienionych RAM/CPU/scratch/politykach. Launcher12 tests PASS; pomiar nowego
  peaku oczekuje wyniku. Nie podniesiono uprawnień ani zasobów kontenera.

## 2026-11-12 — pierwszy rzeczywisty produktowy pion Remote PASS

- Focused snapshot `914803775f16a48457f495654695c58d0f340cc73ec183fd846bd509d9f1135f`
  **PASS1**,6,61s. [Log](evidence/2026-11-12-worker-runtime/session-journal-vertical-first-pass.log).
  Rzeczywisty Remote/Profile REST202 z kluczem, AHP1.0 notyfikacja z trwałym echo,
  dwie tury przez ten sam proces ACP w bubblewrap, rozłączenie browser WebSocket,
  idle renewal, pamięć poprzedniej tury, projekcja dwóch odpowiedzi Journalu,
  identyczny retry bez drugiego actora oraz HTTP stop i controller ACK przeszły.
  Nie użyto prawdziwego modelu, kont ani tailnetu. To test protokołu, nie Chromium.
- Ten sam snapshot zawiera kernel FileLease przed recovery (jego dwa jednostkowe
  testy nie były uruchamiane przez focused filter). Codegen64: build2m13s,
  test compile44s; peak3489660928B, memory.max185, OOM0. Limit3328MiB pozostał
  aktywny; peak dotknął limitu i nie jest dowodem rezerwy RAM. Pełna regresja
  sprawdzi również dodatkowe pakiety i te jednostkowe testy.
- Nowsze zmiany przed pełną regresją: guard sam atomowo zatrzaskuje stop przy
  błędzie, pre-wire sprawdza globalny shutdown oraz stricte test final sequence5
  i worker SQL ack5/reservation0. UI jasno komunikuje, że remote cancel kończy
  całą sesję; remote tool approvals pozostają unsupported. Te nowsze zmiany
  wymagają osobnego wyniku. Cały E4.4 i projekt do E10 nadal są otwarte.

- Przygotowano opcjonalny `--runtime-browser` i dodatkową gałąź istniejącego
  rzeczywistego testu. Driver Chromium używa produkcyjnego UI/REST/AHP i tego
  samego rzeczywistego WSS/sandbox workera. Ma sprawdzić Remote binding pin,
  utratę HTTP202+identyczny retry, dwa prompty z zachowaniem kontekstu po reload,
  stop i końcowe SQL `(sessions2,turns4,reservation0,ack10)` oraz martwego potomka.
  Test nie został jeszcze uruchomiony. Domyślny runtime nadal weryfikuje wyłącznie
  pion protokołu; wcześniejszego PASS nie przypisuje się Chromium.
- Launcher14 jednostkowych tests PASS, także brak możliwości wybrania obcego
  suite/filtra przy GUI opt-in, readonly cache mounts i zachowanie containment.
  Jedyną dodatkową rezerwacją są PID256 dla Chromium (zwykły runtime96); RAM,
  CPU, tmpfs i polityki pozostają jak wcześniej. Pełna regresja wcześniejszego
  snapshotu nadal trwa. Nowsze zmiany obejmują driver/fixture/launcher, bez
  zastępowania produkcyjnego wykonania mockiem.
- W trwającej pełnej regresji przeszły już3 tests lifecycle producenta (obie
  flagi, exclusive producer/drop bez cyklu Arc, transcript opt-out) oraz2 testy
  kernel FileLease (oddzielny proces i odrzucanie niebezpiecznych entries).
  Wynik całego suite i nowego końcowego stop/ACK testu będzie zapisany dopiero
  po zakończeniu. GUI gałąź nadal czeka na oddzielne wykonanie.

## 2026-11-12 — pełny produktowy Remote/Journal/runtime PASS

- Snapshot `a4061d4f8b4e1bc6d85a063e928e70e1f5706ce4ec5e0b26d7989c4b7eb995c9`
  **PASS**: relay545 (29 ignored), Journal67, identity1/terminal27/usage37/
  workspace189 oraz15 rzeczywistych kernel checks31,11s. Peak3457466368B,
  memory.max0/OOM0. [Pełny log](evidence/2026-11-12-worker-runtime/session-remote-product-full-pass.log).
  Obejmuje produkcyjne startup/shutdown, REST/AHP gates, transcript opt-out,
  private FileLease, atomowy stop guard i pre-wire shutdown, odbiór dwóch tur
  po rozłączeniu browser WebSocket oraz końcowe controller sequence5/ACK5,
  worker `(session1,turns2,reservation0,ack5)` i martwego potomka po stop.
  Nie obejmuje późniejszego GUI drivera/launcher opt-in.
- Po zmianie komunikatu remote cancel: frontend122 **PASS** i TypeScript+
  Vite45modules **PASS**.
  [Frontend](evidence/2026-11-12-worker-runtime/session-remote-frontend-final-pass.log),
  [build/typecheck](evidence/2026-11-12-worker-runtime/session-remote-web-build-final-pass.log).
- Oddzielny runtime/Chromium opt-in został uruchomiony na nowszym source. Ma
  sprawdzić dodatkową drugą sesję z formularza i prawdziwy utracony HTTP202,
  jednak wynik jest nadal w toku. Dotychczasowy PASS protokołu nie jest PASS
  GUI. Końcowy Clippy, pozostałe E4.4/E5–E10 i realne integracje pozostają otwarte.

## 2026-11-12 — produkcyjny formularz Remote→Chromium→worker PASS

- Snapshot `8c61eef4421eb8a99c4f5c2dc1bca02c73799f02e69fe5c766497a142a66f096`
  focused runtime z `--runtime-browser`: **PASS1**,12,99s, potwierdzona dodatkowa
  gałąź `remote-session production Chromium branch: PASS`.
  [Pełny log](evidence/2026-11-12-worker-runtime/session-remote-browser-first-pass.log).
- Oprócz poprzedniej sesji protokołu produkcyjny formularz wybrał dokładny
  RemoteBinding/profile pin. Prawdziwy POST202 dotarł do serwera, a utracono tylko
  jego odpowiedź; jawny retry miał identyczne body i key oraz replay header true.
  Po resecie browser/SDK dwie tury tego samego procesu ACP zachowały nonce.
  Transcript UI pokazał obie odpowiedzi; nie wystawiono remote tool approvals.
- HTTP stop wyłączył composer; osobna weryfikacja Rust wymagała canonical stop,
  controller ACK5, worker SQL `(sessions2,turns4,reservation0,ack10)` oraz martwego
  potomka. Sama reakcja UI ani samo end_turn nie są dowodem zatrzymania.
- Peak3489660928B/memory.max309/OOM0: container dotknął limitu3328MiB, bez jego
  zwiększania. GUI miało jawny PID256 przy zachowanych CPU1, tmpfs1536MiB,
  readonly mounts, network none, AppArmor/seccomp i dropped capabilities.
- Nie uruchamiano prawdziwych providerów, kont, IdP ani tailnetu. Nadal otwarte:
  końcowy Clippy, dalsze E4.4 acceptance (m.in. controller restart/lag w tym pionie),
  worker macOS/realny harness, inne etapy E5–E10 i ich bramki operatorskie.

## 2026-11-12 — porządkowanie Clippy i przygotowanie fault acceptance

- Pierwszy Clippy **FAIL42**: nieużywane wewnętrzne v3 API, pomocnicze pola,
  style, duże enum variants i złożony SQL tuple. [Log](evidence/2026-11-12-worker-runtime/session-product-first-clippy-failure.log).
  Zmniejszono Reply jobs przez Box, nazwano SQL row type, poprawiono style,
  usunięto nieużywane gettery i zachowywanie wersji bwrap po jej walidacji.
  Wąskie `expect(dead_code, reason=...)` opisuje świadomie pozostawione v3
  compatibility API i audit-only metadane hooka ACP; active v4 nie ma takiego
  wyciszenia. To nie deklaruje publicznego endpointu v3.
- Kolejny build testów wykrył usunięcie pól PromptIntent używanych przez engine
  audit fixture (E0609). Pola przywrócono wraz z konstrukcją dokładnych danych;
  testów nie usuwano ani nie zmniejszano zakresu. [Log](evidence/2026-11-12-worker-runtime/session-product-clippy-prompt-hook-compile-failure.log).
  Następny Clippy doszedł do jednego collapsible_if w session test; poprawiono.
  [Log](evidence/2026-11-12-worker-runtime/session-product-clippy-test-style-failure.log).
  Nowszy Clippy jest w toku, brak końcowego PASS.
- Przygotowano test-only packet fault hook po rzeczywistych worker SQL cuts.
  Pierwsza sesja gubi tekst event1, RPC TurnAccepted i odpowiedź trwałego ACK;
  prawdziwy terminal jest przesyłany przed tekstem, aż kontroler zapisze Gap,
  a brakująca treść musi przyjść w rzeczywistym ReplayStream. Hook nie ma serde,
  konfiguracji produkcyjnej, alternatywnego ACP ani zmyślonego receipt/hash.
  Liczy rzeczywiste Open/Submit na wejściu workera: wymagane Open1, Submit2,
  każdy command tylko raz. Druga sesja GUI pozostaje bez fault injection.
- Active v4 Submit po utracie odpowiedzi może teraz odczytać wyłącznie exact
  receipt wcześniej atomowo zapisany z oryginalnego streamu. Wymaga nadal
  aktualnego caller/source/Journal lease i niewygasłego lokalnego deadline;
  brak receipt pozostaje niepewny. Odczyt nie wysyła polecenia, nie zmienia
  fazy/timera ani projekcji. Dodano osobny test negative authority/lease/identity
  i niezmienności zegara. Te nowsze zmiany czekają na wykonanie.
- Następny Clippy przeszedł server i jego testy; zatrzymał się na pojedynczym
  `items_after_test_module` w workspace Git helperze. Przeniesiono istniejącą
  funkcję scrub nad test module bez zmiany jej działania.
  [Log](evidence/2026-11-12-worker-runtime/session-product-clippy-workspace-order-failure.log).
  Bieżący bardziej wymagający runtime/Chromium fault test jest już uruchomiony;
  final Clippy i pełna regresja nowszych zmian nadal wymagają wyniku.

## 2026-11-12 — realne utraty/reorder WSS + Chromium PASS

- Snapshot `0fa2d97afaa3db331956ed99c87968fadf7c2d95cb4f13ec2352af9537ce618a`
  focused runtime/Chromium **PASS1**,16,02s; peak3469328384B,
  memory.max0/OOM0. [Log](evidence/2026-11-12-worker-runtime/session-remote-fault-browser-pass.log).
- Fault assertions potwierdziły rzeczywisty controller Gap, brakujący tekst
  doręczony przez ReplayStream, utraconą odpowiedź TurnAccepted i utraconą
  odpowiedź worker SQL ACK. Worker otrzymał Open1 i Submit2; każdy command
  dokładnie raz. Dwie tury zachowały jeden proces/kontekst, Journal miał dokładnie
  dwie odpowiedzi, a końcowy ACK5/reservation0 potwierdzono po obu stronach.
- Po fault scenariuszu druga sesja z produkcyjnego formularza przeszła nadal
  utratę HTTP202/retry, reload, dwa prompty i stop/ACK. W sumie worker miał
  sessions2/turns4/reservation0/ack10 oraz martwego potomka po zatrzymaniu.
- Snapshot obejmuje poprawki stylu/alokacji, zachowane audit metadane promptu,
  weryfikację Journal workspace/project w source callback i resolution utraconego
  Submit reply wyłącznie z wcześniej zapisanego receipt. Nowy jednostkowy test
  read-only resolution skompilował się, lecz focused suite go nie uruchamiał.
- Ostatni Clippy oraz pełny runtime nowszego kodu nadal wymagają wykonania.
  Controller process crash/relaunch, dalsze cancellation/revocation/backpressure
  acceptance i cały projekt E5–E10 pozostają otwarte. Nie zaliczono realnego
  tailnetu, modelu, kont ani platform samym testem fixture.

## 2026-11-12 — Clippy PASS i rozszerzenie active cancellation

- Snapshot `0fa2d97afaa3db331956ed99c87968fadf7c2d95cb4f13ec2352af9537ce618a`
  **Clippy all-targets PASS**, `-D warnings`,2m42s, peak1920299008B/OOM0.
  [Log](evidence/2026-11-12-worker-runtime/session-product-clippy-first-pass.log).
  Jest to ten sam source co poprzedni fault/Chromium PASS16,02s; późniejsze
  cancellation changes wymagają odrębnego wykonania, nie dziedziczą tego PASS.
- Nowy kod sygnalizuje accepted, durable AHP cancellation bez oczekiwania na
  główną pętlę actora. Oryginalny receipt cache służy wyłącznie do negative Close,
  żeby zatrzymanie znanego procesu nie oczekiwało na SQL/FS authority. Każdy
  positive observation/renew cut odmawia requested/unconfirmed cancellation.
- Kanoniczny Unknown/Cancelled terminal może domknąć stream po lokalnym
  Interrupted bez ponownego caller grantu. Sprawdza dokładny dispatch/receipt,
  nie projektuje tekstu, nie tworzy lease i nie zwalnia reservation; dopiero
  kolejny SessionStopped/StopProof zwalnia slot. Dodatkowy test sprawdza odmowę
  złego receipt, zakaz renew, brak drugiej projekcji oraz tę granicę stop.
- Read revocation pozostawia workerowy transcript replay i live reconcile
  zablokowane. Exact stopped metadata (bez tekstu/active turn) pozostaje jednak
  dostępne po aktualnym uwierzytelnionym kanale jako negative capacity evidence.
  Scope/identity i digest nadal są sprawdzane; dodano regression privacy/fencing.
- UI rozróżnia confirmed remote stop od Unknown wyniku przerwanej tury.
  Nowa gałąź Chromium ma dwie ukończone tury po reload, trzecią rzeczywiście
  zawieszoną, queued edits/reorder i AHP cancel całej sesji. Rust wymaga worker
  sessions2/turns5/reservation0/ack12, controller final7/ACK7, dwóch initialization
  i pięciu promptów, martwego potomka oraz dwóch nieuruchomionych queued turnów.
  Ten rozszerzony przebieg został uruchomiony; jego wynik jest w toku.

## 2026-11-12 — active remote cancellation i kolejka: Chromium PASS

- Snapshot `cf34650234df5a64021fbd3ecafbbbeaae02e8ae6179988a7f9efb49035dcd48`
  focused runtime/Chromium **PASS1**,17,43s; peak3478843392B,
  memory.max0/OOM0. [Log](evidence/2026-11-12-worker-runtime/session-active-cancel-browser-pass.log).
  Obejmuje wcześniejsze rzeczywiste packet loss/reorder oraz trzeci aktywny
  prompt, dwie pozycje kolejki, edycję i reorder, AHP cancellation i confirmed
  remote stop przy nadal Unknown wyniku tury. Worker sessions2/turns5,
  reservation0/ACK12; potomny proces nie pisze, obie queued pozycje nie ruszyły.
- Nowsze frontend122 i produkcyjny build/typecheck **PASS** obejmują finalną
  informację UI o confirmed stop/unknown outcome.
  [Frontend](evidence/2026-11-12-worker-runtime/session-active-cancel-frontend-pass.log),
  [build](evidence/2026-11-12-worker-runtime/session-active-cancel-build-pass.log).
- Wydzielono wspólny startup/shutdown obu remote service do `remote_services`;
  main zachowuje dotychczasową kolejność recovery i startup. Pełny runtime
  tego startup i cancellation jest w toku. Przygotowywany osobny test zabija
  SIGKILL rzeczywiste procesy obu service i uruchamia je na tych samych bazach;
  nie ma jeszcze wyniku tego nowego testu ani końcowego Clippy.

## 2026-11-12 — przygotowanie E5.2: oficjalny kontrakt MCP

- Potwierdzono wydany kontrakt 2026-07-28 w oficjalnych źródłach. Obecny
  `crates/mcp-client` zaczyna od legacy initialize i nie jest implementacją nowego
  modelu. Wersja modern wymaga per-request metadata i server/discover; adapter
  2025 musi pozostać osobną ścieżką. To rozpoznanie, nie implementacja ani PASS E5.2.
  [Versioning](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning),
  [discovery](https://modelcontextprotocol.io/specification/2026-07-28/server/discover).
- Przy dalszej implementacji obowiązują zgodne nagłówki/body, brak modern session
  ID i brak automatycznego odtwarzania niepewnej mutacji po zerwanym streamie.
  MRTR oraz subscriptions wymagają odrębnej obsługi; nie można oznaczyć wsparcia
  2026 samą zmianą numeru wersji w initialize.
  [Transport](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http),
  [changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog).

## 2026-11-12 — pełna regresja active cancellation i wspólnego startup PASS

- Snapshot `b528fbb45f5603c5d65ca6e6da589638d77832dca0797cdba8e21a1f8fc9f113`
  pełny suite runtime **PASS**: relay548 (29 ignored), identity1, Journal67,
  terminal27, usage37, workspace189 oraz15 rzeczywistych Linux kernel acceptance
  (32,44s). [Log](evidence/2026-11-12-worker-runtime/session-active-cancel-full-pass.log).
  Peak3489660928B, memory.max67, OOM0/kill0; limit RAM nie był zwiększany.
- Zwykły suite wykonał nowe regression tests lost-reply resolution, cancellation
  fences/negative terminal po Interrupted oraz Read revoke/stopped metadata.
  Startup/shutdown obu remote service jest współdzielony z main i skompilowany
  w tym przebiegu. Nowy test actual dual process-relaunch powstał później:
  jego osobny focused przebieg jest uruchomiony i nie dziedziczy tego PASS.

## 2026-11-12 — process acceptance: pierwsza próba fixture niezaliczona

- Snapshot `80ae3cde2895a31779635ade800dcea78a6a15dd4a11ccf1f36af687691da9c1`
  skompilował nowy test, lecz zakończył się timeoutem odbioru readiness po20s;
  nie doszedł do SIGKILL ani odbioru recovery. Peak3446276096B/OOM0.
  [Log](evidence/2026-11-12-worker-runtime/session-process-bootstrap-fixture-failure.log).
- Parser oczekiwał samodzielnego JSON w stdout, a domyślny libtest może poprzedzić
  pierwszy wpis nazwą wykonywanego testu. Child runner przełączono na terse;
  parser toleruje dodatkowo wyłącznie dokładny znany prefiks helpera. Nie zmieniono
  produkcyjnego recovery/transportu. Następny przebieg obejmie także przygotowany
  SIGSTOP/CONT i nadal wymaga wyniku; poprzednia próba nie stanowi PASS.

## 2026-11-12 — rzeczywisty restart obu remote service i SIGSTOP/CONT PASS

- Snapshot `8f8a9dcb29949c81e06fda4bbef85c6239837390951bdb84ea834e1014534264`
  focused process acceptance **PASS1**,20,96s. Peak3489660928B,
  memory.max473, OOM0/kill0. [Log](evidence/2026-11-12-worker-runtime/session-process-restart-stall-first-pass.log).
  To osobne rzeczywiste procesy z produkcyjnymi service/runtime za kontrolowanym
  prywatnym TLS/WSS; bez hostowych kont, prawdziwego tailnetu i dostawcy modeli.
- SIGKILL workera i kontrolera w aktywnym sandbox prompt, otwarcie obu tych samych
  baz i nowe controller/worker boots: Journal Interrupted/Unknown/Queued,
  historyczny REST replay zwraca tę samą sesję i nie tworzy actora. Worker
  sessions1/turn1/reservation0/ACK3 dopiero po source stop; efekt dokładnie jeden,
  potomek z held FD martwy. Dopiero jawna nowa sesja wykonuje kolejny prompt.
- Trzecia sesja otrzymuje aktywny prompt i queued pozycję. Cały kontroler jest
  zatrzymany SIGSTOP przez minimum11s, ponad oba lease10s; worker sam kończy
  prawdziwą ścieżkę disconnect/guardian. Po SIGCONT i nowym worker boot stara tura
  jest Unknown, queued nie startuje, a końcowy worker ledger ma sessions3/turns3,
  reservation0/ACK9 i dokładnie trzy efekty. SIGSTOP/CONT adresuje wyłącznie
  własny proces przez pidfd. Nie wykonano żadnego globalnego kill/restartu usług.
- Po tym snapshot przeniesiono testowy bind listener przed recovery, zgodnie z
  produkcyjnym main i kontraktem wyłącznego coordinatora. Nowszy Clippy jest
  uruchomiony; ten drobny wariant startu wymaga jeszcze focused runtime wykonania.
  Dalsze SQL/journal/ACK crash cuts i pełne platformowe/tailnet acceptance pozostają
  otwarte. Nie oznaczono całego E4.4 ani E10 jako zakończonych.

## 2026-11-12 — domknięcie dokładnych granic Journal/ACK: przygotowanie

- Clippy nowszego procesu znalazł pojedynczy `needless_borrows_for_generic_args`
  w wyborze trwałej testowej bazy. Usunięto zbędne `&`, bez zmiany zachowania;
  nie wyłączono lintu. [Log](evidence/2026-11-12-worker-runtime/session-process-clippy-borrow-failure.log).
- Nowy wariant process acceptance wykonuje ten sam rzeczywisty restart/lag dla
  dwóch dokładnych cutów: po controller Journal commit przed ACK wire (worker
  ACK0) oraz po worker SQL ACK1, ale przed dostarczeniem odpowiedzi i controller
  confirmed ACK (0). Test-only pauza emituje tylko marker; parent sprawdza oba
  prawdziwe SQL stany przed SIGKILL. Nie fabrykuje receipt ani terminalu i nie ma
  produkcyjnego ustawienia tych pauz. Oba warianty wymagają wykonania.

## 2026-11-12 — dokładne SQL/Journal/ACK crash cuts PASS

- Snapshot `d3f249337b92821afa9eb18782e13a0b4739b1acba0f1cafe40ec9438f798c37`
  focused actual process acceptance **PASS1**,42,14s. Peak3489660928B,
  memory.max107, OOM0/kill0.
  [Log](evidence/2026-11-12-worker-runtime/session-process-journal-ack-cuts-pass.log).
- Scenariusz wykonuje się dwukrotnie na osobnych bazach, za prawdziwym TLS/WSS,
  z ACP peer w rzeczywistym bwrap. Parent czeka na marker pauzy i potwierdza:
  controller sequence1/ACK-watermark1/confirmed0, worker ACK0 przed wire albo
  worker ACK1 po źródłowym SQL commit bez doręczonej odpowiedzi. Następnie SIGKILL
  obu własnych procesów. Żaden receipt/hash/event nie jest fabrykowany.
- Oba warianty przeszły restart tych samych DB, nowe boots, historyczny create
  bez actora/replay, Interrupted/Unknown/Queued, source stop/ACK3 i reservation0,
  jawny nowy create z nowym kontekstem oraz dodatkowy SIGSTOP/CONT kontrolera
  przez minimum11s. W każdym wariancie końcowy worker ledger3 sessions/3 turns,
  reservation0/ACK9, trzy efekty i martwy potomek; obie stare queued pozycje nie
  zostały wysłane. Test obejmuje już listener exclusivity przed recovery.
- Produkcyjny build nie zawiera ACK pause hooks ani flag tych pauz. Prywatny
  child runner testowy wybiera jedną host-scoped pauzę; pozostałe testy nie są
  objęte tym trybem. Wszystkie wykonywalne guards i oba protokoły są produkcyjne.
- Nowszy all-targets Clippy jest uruchomiony. Domknięcie E4.4 czeka na ten wynik;
  realny tailnet, macOS worker i credential/provider provisioning nadal są
  osobnymi otwartymi odbiorami, nie wynikiem tych fixtures.

## 2026-11-12 — odbiór E4.4 i final Clippy PASS

- Snapshot `d3f249337b92821afa9eb18782e13a0b4739b1acba0f1cafe40ec9438f798c37`
  **all-targets Clippy PASS**, `-D warnings`,1m53s; peak1978896384B,
  memory.max0/OOM0. [Log](evidence/2026-11-12-worker-runtime/session-process-journal-ack-clippy-pass.log).
  Dokładnie ten sam source ma focused process/Journal/ACK/lag PASS42,14s.
- Przyjęto **E4.4**: v1/v2/v3 compatibility i jawne opt-in v4, shared host
  reservations, serial queue/admission/fencing, idle/active guardians i kernel
  ownership, durable stream/projection/ACK z rollback, rzeczywiste packet loss,
  reorder/duplicate, lost reply, bounded spool, GUI context/reconnect/queue/cancel,
  SIGKILL/bootstrap/handoff/running oraz oba dokładne Journal/ACK crash cuts,
  realne relaunch obu service i SIGSTOP/CONT bez ożywienia starych promptów.
- Macierz dowodów obejmuje domain/protocol49+33, pełny runtime b528fbb4
  (548 relay/67 Journal/189 workspace/15 kernel), Chromium cf346502
  (sessions2/turns5/ACK12), frontend122/build/typecheck oraz powyższy final d3f24933.
  Zakres każdego logu pozostaje jawny; nie twierdzimy, że focused suite wykonał
  wszystkie pozostałe testy ani że wszystkie snapshoty były identyczne.
- Wspierany worker persistent runtime to Linux/bwrap; default off, remote human
  tool approvals unsupported, cancellation kończy kontekst z Unknown wynikiem.
  Native macOS Rust/platform acceptance, dwie rzeczywiste maszyny/tailnet,
  keyring/provider credentials i odbiór rzeczywistego harnessu pozostają otwarte
  w E4.2/E4.7/E10. Sam etap E4 nadal nie jest zamknięty.
- Checklista:19/68 punktów odebranych, około28%. Następny przekrój to E4.5:
  rzeczywista obserwacja kanału i diagnostyka runtime w panelu hostów, bez
  utożsamiania historycznego rekordu SQL z żywym połączeniem lub authority.

## 2026-11-12 — E4.5: bieżąca obserwacja WSS i runtime counts, w realizacji

- Dodano osobny, nietrwały registry żywych zaakceptowanych kanałów v1–v4.
  Powstaje dopiero po Accepted na zweryfikowanym WSS; RAII usuwa dokładną
  rejestrację, bez usuwania nowszego kanału tego samego hosta. Sam rekord SQL
  nigdy nie tworzy tej obserwacji i nie zmieniono execution authority.
- Zegar obserwacji heartbeat jest monotoniczny. Nowa sekwencja zapisuje moment
  odebrania, nie późniejszego zakończenia SQL job; duplicate nie przedłuża zegara.
  Getter wymaga też aktualnego Store identity/channel/boot/expiry w caller TX.
- Nowy manager-scoped GET `/v1/web/workspaces/{id}/hosts/{host}/health` zwraca
  obserwację kanału i zagregowane reservations/uncertain/pending ACK, bez promptów,
  transkryptów lub cudzych session IDs. Stary `/channel` zachowuje kontrakt.
- Rozszerzono rzeczywisty v1 TLS/WSS fixture o żywą obserwację i zniknięcie po
  revoke; HTTP regression odmawia viewerowi i nie uznaje starego rekordu za live.
  Pełny runtime tego backendu jest uruchomiony: wynik jeszcze nieznany.
- UI/push WebSocket i final health diagnostics acceptance nadal są w realizacji.
  Punkt E4.5 pozostaje otwarty; nie dziedziczy PASS E4.4 sprzed tych zmian.

## 2026-11-12 — E4.5: compile check i przygotowany push

- Pierwszy backend snapshot ab77b097 zakończył się E0277: rusqlite nie dekoduje
  SQL count bezpośrednio do u64. Dodano odczyt i64 i sprawdzoną konwersję;
  brak castów ignorujących ujemne/nieprawidłowe wartości. Testy tego przebiegu
  nie ruszyły. [Log](evidence/2026-11-12-worker-runtime/host-health-count-type-compile-failure.log).
- Dodano osobny bounded WS `/health/live`, subprotocol `dunlin.host-health.v1`:
  pełne snapshoty bez historii, wspólny limit64 browser connections, frame/input
  limits, heartbeat, send deadline i świeży manager/device/host cut także bez
  zmian payloadu. Endpoint nie przyjmuje komend wykonania ani sekretów w URL.
- Przygotowany regression otwiera rzeczywisty WS i odbiera scope-bound snapshot,
  następnie downgrade managera bez zmiany hosta musi dać unavailable/close.
  Ten nowszy kod i test wymagają wykonania. UI nadal w realizacji.

## 2026-11-12 — E4.5: klient/UI podglądu i pierwsze checks PASS

- Produkcyjny Host Registry ma osobny podgląd live: transport i heartbeat,
  rezerwacje, aktywne sesje/tury, uncertain oraz pending ACK. Uruchomienie jest
  jawne, dane idą przez WebSocket; klient nie używa REST pollingu ani storage.
  Strict decoder sprawdza zakres, znane pola, liczniki, stream/sequence i TTL.
  Server keepalive nie przedłuża worker lease; utrata dostępu, wygaśnięcie lub
  błąd strumienia usuwają live payload. Historyczny /channel pozostaje osobny.
- Frontend **PASS128**, produkcyjny Vite build **PASS**.
  [Frontend](evidence/2026-11-12-worker-runtime/host-health-frontend-first-pass.log),
  [build](evidence/2026-11-12-worker-runtime/host-health-build-first-pass.log).
- Osobny tsc początkowo znalazł narrowing TypeScript dla wspólnego wariantu
  heartbeat/unavailable. Rozdzielono discriminated union, bez zmiany wire/runtime
  i bez `any`/wyłączenia strict. Następny **tsc --noEmit PASS** exit0, source
  `4a7703a9ddf06a2e8fafabc5ac5afe154784487fb4625272744dbabbc0361236`.
  [Pierwszy błąd](evidence/2026-11-12-worker-runtime/host-health-type-narrowing-failure.log),
  [PASS](evidence/2026-11-12-worker-runtime/host-health-typecheck-first-pass.log).
- Doprecyzowanie wcześniejszych wpisów: suite build uruchamia Vite, sam nie
  wykonuje tsc. Poprzednie sformułowania przypisujące logom build również
  typecheck były zbyt szerokie. Powyższy osobny tsc obejmuje całe src i tests,
  w tym dotychczasowy kod E4.4 oraz nowy podgląd; log build sam nie jest takim
  dowodem. Nie zmieniono launchera ani zakresu kompilatora.
- Przygotowano dodatkowe registry unit tests: duplicate i late SQL completion
  nie odświeżają monotonicznego liveness; stary Drop/heartbeat nie zmienia nowego
  kanału. Wymagają jeszcze wykonania. Rozszerzony Chromium driver ma odwiedzić
  realny host v4 po zdalnej sesji, odebrać następny source heartbeat i potwierdzić
  zero REST health reads. Nowsze unit/browser checks oraz final Clippy są otwarte.

## 2026-11-12 — E4.5: WS authority PASS, poprawka oczekiwania wersji fixture

- Snapshot ba6bcae5 wykonał normalne relay checks:548 PASS,1 FAIL,31 ignored.
  Nowy rzeczywisty WS downgrade-manager bez zmiany host payload **PASS**.
  Jedyny FAIL pochodził z nowego assertion fixture: oczekiwał v1, a domyślny
  worker prawidłowo negocjuje v2 i getter zwrócił2. Nie zmieniano serwera ani
  negocjacji; poprawiono test na2. Pełny przebieg nie jest PASS i po zakończeniu
  pozostałych pakietów jego log zostanie zachowany. Kernel checks po tym błędzie
  nie są zaliczane.
- Osobny istniejący manual_v1 fixture dostał sprawdzenie prawdziwego v1 live
  observation po jego heartbeat ACK. Wcześniejszy opis podstawowego fixture jako
  v1 był niedokładny: teraz testowane są oddzielnie oba rzeczywiste transporty.
- Następny full runtime/Chromium ma obejmować także nowe monotonic/registration
  unit tests, poprawkę oczekiwania v2 oraz UI obserwujący rzeczywisty v4 heartbeat.

## 2026-11-12 — izolacja workspace i odtworzenie pojedynczego przypadku

- Cały runtime ba6bcae5 ostatecznie przekroczył wall900s podczas workspace
  comparison test; exit124. Nie doszedł do kernel acceptance. Oprócz wcześniej
  opisanego błędnego v1 assertion nie zaliczono tego przebiegu.
  [Log](evidence/2026-11-12-worker-runtime/host-health-version-expectation-workspace-timeout.log).
- Dodano suite workspace z tym samym offline/readonly/UID/cap-drop/network-none
  sandboxem;1536MiB RAM, scratch768MiB, PID96, file2GiB dla istniejących sparse
  preflight cases. Brak hostowego buildu i zmiany polityk runtime. Launcher15
  regression tests PASS; bounded rust identifier dotyczy także tej ścieżki.
- Focused workspace case na świeżym sandboxie **PASS1**,5,82s (build1m08s),
  peak676986880B/OOM0; test nie został pominięty ani złagodzony.
  [Log](evidence/2026-11-12-worker-runtime/host-health-workspace-focused-first-pass.log).
  Nie odtworzono hang w izolacji. Dodatkowy powtórzony przebieg jest w toku;
  nie przypisujemy przyczyny błędu health code bez dowodu. Następna pełna
  regresja obejmie poprawione oczekiwanie wersji oraz nowsze health unit/UI cases.


## 2026-11-12 — E4.5: pełna regresja runtime i Chromium PASS

- Snapshot `a0e744b99e31a5bd9094c5abcd19063fb2476dacc3cffff53d6f61b9533441d3`: relay551 PASS (31 ignored), identity1, Journal67, terminal27, usage37, workspace189 oraz osobny kernel acceptance16 PASS. Chromium rzeczywiście obserwuje host v4 i kolejny heartbeat bez REST health pollingu.
  [Pełny przebieg](evidence/2026-11-12-worker-runtime/host-health-full-browser-first-pass.log).
- All-targets Clippy `-D warnings` na tym samym snapshot: PASS1m43s, peak2015555584B, OOM0.
  [Clippy](evidence/2026-11-12-worker-runtime/host-health-clippy-final-pass.log).
- Powtórzony focused workspace case również PASS5,23s; pełne189 workspace checks powyżej także przeszły. Wcześniejszy timeout nie został odtworzony; kod workspace nie wymagał zmiany.
  [Powtórzenie](evidence/2026-11-12-worker-runtime/host-health-workspace-focused-repeat-pass.log).
- Osobny legacy Hosts browser nie dotarł do Chromium: linker przekroczył limit128MiB pojedynczego pliku (SIGXFSZ25), bez OOM. Launcher przyznaje teraz browser/server ten sam limit2GiB co runtime/workspace, ponieważ budują ten sam duży ELF. RAM, PID, readonly i polityki izolacji pozostają ograniczone jak poprzednio.
  [Pierwszy błąd](evidence/2026-11-12-worker-runtime/host-health-registry-browser-fsize-failure.log).
- Launcher17 tests PASS, kontrola121 Markdown PASS, diff whitespace PASS. Powtórzenie legacy Hosts browser jest w toku; E4.5 pozostaje otwarty do wyniku tego testu.


## 2026-11-12 — legacy Hosts browser: poprawka oczekiwania opisu

- Powtórzenie po zmianie file-size limitu zbudowało relay i uruchomiło Chromium. Test zatrzymał się na dawnym tekście „Wyłącznie raportowany stan”; panel ma już opis „Zapis diagnostyczny kanału. Bieżącą obserwację pokazuje podgląd na żywo.”. Zaktualizowano tę asercję do rzeczywistego kontraktu UI; pozostałe sprawdzenia uprawnień, tickets, CAS/drain/revoke i braku persistence pozostają.
  [Nieudany przebieg](evidence/2026-11-12-worker-runtime/host-health-registry-browser-caption-failure.log).
- To nie jest PASS całego browser acceptance. E4.5 pozostaje otwarty do ponownego wykonania.

## 2026-11-12 — E5.2: metadane i projekcja nagłówków

- Dodano `crates/mcp-client/src/metadata.rs`: metadane żądania wersji2026, nazwa/method headers i kompilowane dokładne ścieżki `x-mcp-header`, walidacja nazw/typów/unikalności, kodowanie UTF-8 i sentinel literals, ograniczenie safe integers, rozmiaru i pracy nad schematem. Przykładowe dane w schema examples nie są traktowane jak adnotacje.
- Moduł nie wysyła żądań ani nie przyznaje uprawnień. Legacy Client pozostaje przy dotychczasowym handshake2025; nowy transport/discovery, adapter wyboru wersji, MRTR, subscriptions, OAuth i walidacja JSON Schema pozostają do implementacji. Nie deklarujemy jeszcze obsługi całego2026 ani odbioru E5.2.
- Podstawa: [oficjalny Streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http) i [kontrakt narzędzi](https://modelcontextprotocol.io/specification/2026-07-28/server/tools). Suite `mcp` uruchamia pełny pakiet klienta w tym samym offline/readonly sandboxie z RAM1536MiB, scratch768MiB, PID96 i file128MiB; pierwszy przebieg jest w toku.

- Pierwszy suite mcp: **18 tests PASS**, w tym5 nowych metadata cases; build/offline/locked PASS, peak678322176B/OOM0. Snapshot `050bdfede32421b6419372faa10f25d3453f2a036b35eeda6ddd235711755c97`.
  [Testy](evidence/2026-11-12-worker-runtime/mcp-metadata-first-pass.log).
- Rozszerzono ten mały suite o następujący po udanych testach all-targets Clippy `-D warnings` dla samego pakietu MCP. Pierwszy powyższy log obejmuje testy, nie Clippy; nowy pełny przebieg musi zostać wykonany osobno.


## 2026-11-12 — odbiór E4.5: panel kondycji hostów

- Registry Chromium **PASS1**,7,89s, build2m41s. Snapshot `431e4d8de4b572e36fac3809529fa6805b48b0f241021a8e4093bd82f763f46a`, peak3489660928B/OOM0. Test wykonał create/enrollment, rzeczywisty dedicated worker HTTP claim/report, admin CAS/drain/revoke, odrzucenie dawnych credentials, jawny raw diagnostic i jego invalidation, odebranie ManageHosts, viewer403 i brak secret persistence.
  [Końcowy browser](evidence/2026-11-12-worker-runtime/host-health-registry-browser-final-pass.log).
- W połączeniu z wcześniej zapisanym pełnym runtime/Chromium v4, jednostkowymi monotonic/fencing checks, HTTP/WS scope downgrade, frontend128, tsc/Vite i health Clippy jest to dowód odbioru panelu E4.5. Różne logi mają jawne snapshot hashes; nowszy browser zawiera nowy niepodłączony moduł metadanych MCP i poprawkę oczekiwanego opisu UI, bez zmiany funkcjonalnego kodu health względem a0e744b9.
- Checklist: **20/68 (29,4%)**. To udział odebranych kryteriów, nie godzin. Zakończenie panelu nie jest zamknięciem całego E4: ProjectBinding/platform acceptance, opcjonalny onboarding i realne tagi/granty/tailnet pozostają oddzielnie otwarte. Całe E5–E10 pozostaje w celu projektu.
- E5.2 jest w realizacji: moduł metadanych ma18 tests PASS; końcowy suite z osobnym pakietowym Clippy jest w toku. Nie zaznaczamy E5.2 jako ukończonego.

- Końcowy suite MCP **PASS18 + all-targets Clippy -D warnings PASS**, exit0.
  [Testy i Clippy](evidence/2026-11-12-worker-runtime/mcp-metadata-final-pass.log).
  Moduł metadanych jest zweryfikowany; następna praca E5.2 to połączenie go z nowym stateless transportem, discovery i kontrolowanym adapterem2025.


## 2026-11-12 — E5.2: modern-first transport i jawny adapter2025

- Dodano `negotiated::Client`: read-only server/discover z metadanymi2026, stateless POST JSON/SSE, nagłówki narzędzi i brak initialize/session/DELETE w nowej wersji. Publiczny dotychczasowy Client pozostaje jawnym adapterem2025; istniejące integracje nie zostały jeszcze przełączone.
- Nierozpoznane HTTP400 probe wybiera adapter2025. Rozpoznane błędy modern,401/403, inne statusy i błędy transportowe nie powodują downgrade. Wspólny90s budget obejmuje probe i legacy handshake; request20s, response2MiB, catalog512/16pages i cursor4KiB pozostają ograniczone. Brak env proxy, redirects i automatycznych retry operacji.
- Modern catalog wyklucza narzędzia z błędnymi adnotacjami nagłówków, atomowo publikuje komplet stron i usuwa dawny snapshot przy błędzie refresh. Wywołanie wymaga zgodności z pobranym schematem i projekcją; to spójność protokołu, nie allowlista/uprawnienie E5.3.
- **25 tests PASS + all-targets Clippy -D warnings PASS**, exit0. Rzeczywiste lokalne HTTP fixtures sprawdzają stateless/SSE i kodowane nagłówki, legacy session/DELETE, brak downgrade przy błędach2026/auth, wykluczenie złego schema, failed refresh, brak retry przy błędzie narzędzia/InputRequired oraz deadline przed siecią.
  [Dowód](evidence/2026-11-12-worker-runtime/mcp-negotiated-first-pass.log).
- Pierwszy build znalazł mismatch Response kontra Box<Response> we wspólnym ramieniu status error; poprawiono rozpakowanie wrappera HTTP.
  [Błąd kompilacji](evidence/2026-11-12-worker-runtime/mcp-negotiated-response-type-failure.log).
- E5.2 pozostaje otwarty: MRTR obecnie zwraca Unsupported bez kontynuacji, brak jeszcze subscriptions/cache/OAuth i pełnego JSON Schema validatora oraz podłączenia nowego klienta do integracji. Nie deklarujemy pełnej zgodności2026.
- Przy wyjaśnianiu komunikacji server–runner poprawiono przestarzały wstęp worker-channel.md: opis v1/v2 nie oznacza już braku wykonania v3/v4. Linki do aktualnych kontraktów i granice odbioru są jawne. Dokumentacja121 Markdown PASS; diff whitespace PASS.


## 2026-11-12 — E5.2: jawne kontynuacje MRTR

- Dodano Outcome::Complete/InputRequired i single-use Pending dla tools/call, prompts/get i resources/read. Wygodne call pozostaje zgodne z dotychczasowym Result<Value>; przy InputRequired zwraca Unsupported i nie kontynuuje automatycznie.
- Pending jest związany z konkretną instancją klienta/auth oraz oryginalnym zleceniem. Nie ma Clone/Serialize/Debug. Opaque requestState jest sprawdzany tylko pod względem typu/rozmiaru, dokładnie echoowany i zastępowany co rundę. Świeży JSON-RPC ID i parametry pierwotnej operacji nie przenoszą stanu na inne wywołania.
- URL elicitation ma jawny opt-in capability i decyzje accept/decline/cancel bez content/secrets. Transport nie otwiera URL ani nie zmienia bearer. Integracja UI zgody pozostaje do wykonania. Form elicitation czeka na walidator/UI; roots i sampling nie są reklamowane, zgodnie z ich deprecated statusem2026.
- Budżety8 zajętych miejsc przed siecią,8 rund,60s/operację w ramach90s klienta,16 inputs i16KiB opaque state. Slot przechodzi przez kontynuacje także przy pełnym obłożeniu; drop/error/complete go zwalnia. Network timeout uwzględnia wspólny deadline.
- **34 tests PASS + all-targets Clippy -D warnings PASS**, exit0, peak711495680B/OOM0. Nowe9 cases obejmują rzeczywiste lokalne HTTP rundy, URL decisions, rozdzielenie użytkowników, prompts/resources, wymianę stanu, limity i admission bez dodatkowego requestu. Oddzielny thread test sprawdza współbieżną rezerwację i zwalnianie8 slots.
  [Dowód](evidence/2026-11-12-worker-runtime/mcp-mrtr-first-pass.log).
- Początkowo32 tests PASS, lecz Clippy odrzucił deprecated fetch_update. Następny wariant try_update nie był zgodny z MSRV1.92. Final używa compare_exchange_weak bez wyciszania lintów.
  [Deprecated API](evidence/2026-11-12-worker-runtime/mcp-mrtr-atomic-deprecation-failure.log),
  [MSRV](evidence/2026-11-12-worker-runtime/mcp-mrtr-msrv-failure.log).
- Dodano aktualny [kontrakt klienta](mcp-client.md); kontrola122 Markdown i whitespace PASS. E5.2 nadal otwarty: subscriptions/cache, OAuth, pełny JSON Schema/form elicitation i podłączenie klienta do wykonania/UI. Formalny odbiór pozostaje20/68.


## 2026-11-12 — E5.2: subskrypcje POST SSE i unieważnianie obserwacji

- Dodano modern subscriptions/listen, niezależnie posiadany Subscription reader, jawny filtr4 rodzajów zmian i obsługę podzbioru potwierdzonego przez serwer. Pierwszy komunikat musi być ACK; subscriptionId, metoda i dokładny URI są sprawdzane dla kolejnych ramek. Graceful completion jest odróżniony od abrupt EOF. Brak GET/Last-Event-ID/auto reconnect oraz fallbacku legacy dla tej funkcji.
- Odczyt notifications/tools/list_changed unieważnia katalog tylko swojej instancji klienta/auth. Otwarcie strumienia, jego drop/error/EOF/completion pokrywają lukę obserwacji. Katalog pobierany w chwili zmiany nie może zostać opublikowany; client.call i MRTR continuation ponownie sprawdzają aktualny epoch. Te obserwacje nadal nie są grantami E5.3.
- Limity4 uchwyty przed siecią,64KiB/frame,2MiB/stream,1024 JSON messages i64 URI do4KiB. Komentarze SSE liczą się do bajtów. HTTP zachowuje20s i90s budżetu klienta. Integracja napędza reader jawnie, np. własnym wątkiem. Close/drop klienta sygnalizuje anulowanie; już zablokowany read pozostaje ograniczony deadline.
- Pierwszy suite **42 tests + Clippy PASS**. Dodatkowa regresja z dwoma użytkownikami i sterowanym momentem zdarzenia podczas page2 dała **44 tests + all-targets Clippy -D warnings PASS**, exit0. Snapshot `6abce678d02b7d190d1b5f19f3133ad00e3ab835b1faf8f6be69c0c4da04799a`, peak712536064B/OOM0.
  [Pierwszy przebieg](evidence/2026-11-12-worker-runtime/mcp-subscriptions-first-pass.log),
  [Końcowy przebieg](evidence/2026-11-12-worker-runtime/mcp-subscriptions-final-pass.log).
- Oficjalny kontrakt odczytano przez GitHub connector: docs/specification/2026-07-28/basic/patterns/subscriptions.mdx, blob0349e28be419632e1498a3924a8ed0ba2b85d363. Dokumentacja HTTP strony subscriptions była niedostępna; nie zgadywano pól ACK lub notification filter.
- [Kontrakt klienta](mcp-client.md#subskrypcje-zmian) opisuje obsługę i ograniczenia. Kontrola122 Markdown oraz diff whitespace PASS.
- E5.2 nadal otwarty: cache TTL, OAuth, pełne JSON Schema/form elicitation i podłączenie wykonania/UI; E5.3 ma osobno broker policy/allowlist/scoped secrets/kill switch. Formalna checklist nadal20/68.


## 2026-11-12 — E5.2: prywatny cache TTL odczytów MCP

- Dodano memory-only cache dla dozwolonych metod odczytu, kluczowany metodą i parametrami. Każda instancja należy do jednego endpoint/bearer/input-capabilities context; także public responses nie są współdzielone. Tools/call, prompts/get, InputRequired i wszystkie kontynuacje MRTR są wyłączone z cache.
- Monotonic TTL przy dostępie, brak/zero/ujemny TTL bez cache, dodatni max300000ms;64 entries/8MiB zakodowanych payloadów i kluczy/16KiB key. Strony katalogu mają niezależne TTL/cursor keys; sprzeczne cache scopes powodują odrzucenie i usunięcie stron. Jawne refresh_tools omija cache.
- Subscriptions epoch unieważnia wpisy, także resources/read; cache hit ponownie sprawdza epoch po skopiowaniu JSON. Zmiana podczas lookup nie udostępnia starego hitu. Cache respektuje deadline i zamknięcie klienta. Zaobserwowane401/403 także na listen usuwa dane i blokuje dalsze użycie klienta. Błąd schematu tools/call usuwa pin/cache bez automatycznego retry.
- Pierwszy suite **56 tests + Clippy PASS**; dodatkowe cases401 z listen oraz HeaderMismatch dały **58 tests + Clippy PASS**. Końcowy snapshot z ponownym sprawdzeniem epoch po lookup: **58 tests + all-targets Clippy -D warnings PASS**, exit0, peak712208384B/OOM0.
  [Pierwszy](evidence/2026-11-12-worker-runtime/mcp-cache-first-pass.log),
  [Auth/schema](evidence/2026-11-12-worker-runtime/mcp-cache-authentication-pass.log),
  [Końcowy](evidence/2026-11-12-worker-runtime/mcp-cache-final-pass.log).
- Testy obejmują realne HTTP hit/miss bez pollingu, per-page TTL, różne URI/użytkowników (public i private), MRTR exclusion, brak cache mutacji, notifications, scopes mismatch, wymuszony refresh, TTL expiry bez sleeps, limity i auth/deadline fencing. Nie jest to dowód OAuth ani brokera uprawnień.
- [Kontrakt](mcp-client.md#cache-odczytów-i-ttl) i oficjalne reguły cache są podlinkowane. Kontrola122 Markdown oraz whitespace PASS. E5.2 nadal otwarty: OAuth, JSON Schema/form, integracja wykonania/UI; E5.3 pozostaje osobną warstwą. Formalnie20/68.


## 2026-11-12 — E5.2: OAuth discovery i odczyt rzeczywistych challenges

- Dodano oauth::Discovery: Protected Resource Metadata, parser wszystkich WWW-Authenticate i read-only nieuwierzytelniony MCP probe. Zachowywany jest oryginalny challenge wraz ze scopes; scope spoza scopes_supported nie jest odrzucany ani automatycznie zatwierdzany.
- Discovery wybiera jawny resource_metadata albo kolejność well-known resource path/root, a następnie OAuth/OIDC path insertion/appending. Tylko404/405 prowadzi do kolejnego URL; błędny dokument200, issuer mismatch lub błąd auth nie uruchamia fallbacku.
- Dokładny string issuer, dopasowanie resource do origin/path boundary endpointu, deklarowane code/S256 i operator origin allowlist są weryfikowane. Wybrany indeks issuer jest jawny; metadane nie rozszerzają listy originów. URL HTTPS/loopback, bez credentials/fragmentów; brak redirects/env proxy, brak tokenów/cookies podczas odczytu metadanych.64KiB/doc,10s/request,30s całość.
- Pierwszy suite64 tests/Clippy PASS; rzeczywisty probe z powtarzanymi WWW-Authenticate i warianty root/path dały66 tests/Clippy PASS. Następnie rozszerzono suite mcp o dunlin-http, ponieważ wspólny wrapper udostępnia teraz header_values z zachowaniem wszystkich pól i błędów tekstu.
- Końcowy przebieg **66 MCP +6 HTTP tests PASS oraz all-targets Clippy -D warnings dla obu pakietów PASS**, exit0. HTTP test sprawdza kolejność challenges i zgłoszenie nieprawidłowego tekstu zamiast pomijania pola. Launcher17 tests i dokumentacja122 Markdown PASS.
  [Pierwszy](evidence/2026-11-12-worker-runtime/mcp-oauth-discovery-first-pass.log),
  [Probe](evidence/2026-11-12-worker-runtime/mcp-oauth-probe-first-pass.log),
  [Oba pakiety](evidence/2026-11-12-worker-runtime/mcp-oauth-discovery-final-pass.log).
- To discovery, nie kompletny OAuth flow. PKCE/state/callback z RFC9207, wymiana/odświeżanie/odwołanie tokenów, issuer-bound registrations i persistence/UI pozostają do wykonania. Nie podłączono realnego konta ani nie rozpoczęto logowania.
- Aktualny [kontrakt](mcp-client.md#oauth-odkrywanie-metadanych) zawiera oficjalne źródła. E5.2 i cały projekt E0–E10 pozostają otwarte; formalnie20/68.


## 2026-11-12 — E5.2: PKCE, callback i jednorazowa wymiana kodu OAuth

- Dodano AuthorizationFlow z32 rekordami/300s,256-bit state i verifier PKCE/S256, przypiętym issuer/resource/redirect oraz AuthContext user/workspace/connection. Context musi pochodzić od uwierzytelnionej aplikacji; DTO sam nie jest dowodem tożsamości. State jest przechowywany pod hashem, verifier w Zeroizing.
- Callback sprawdza origin/path, duplikaty, context, expiry i RFC9207 iss przed code/error. Obcy context nie zużywa cudzego rekordu; prawidłowy callback atomowo usuwa rekord przed token HTTP. Mismatch, error, replay i błąd token request nie powodują automatycznego retry.
- Pre-registration jest związana z dokładnym issuer; obsługiwane public none i confidential Basic/Post zgodnie z metadata AS. Zarezerwowane OAuth query params endpointów są odrzucane. Authorization i token request mają ten sam resource/redirect; token POST zawiera verifier, a URL do UI tylko challenge.
- TokenSet i Registration nie mają Debug/Serialize. Własne bufory tokenów, verifiera, sekretu klienta i token request są Zeroizing. TokenSet sprawdza context/endpoint/local expiry; Bearer i downscope są obsługiwane, scope escalation odrzucane. Ważność lokalna od początku requestu: max3600s, bez expires_in60s. Refresh token jest prywatnie zachowywany dla kolejnego etapu.
- Pierwszy przebieg6 HTTP +73 MCP tests PASS, lecz Clippy wykrył needless borrow. Poprawka używa as_slice bez kopiowania entropy array. Następny przebieg miał PASS; końcowa wersja dodatkowo usuwa kopię tymczasowego form-encoded sekretu w Basic Auth.
  [Pierwszy lint](evidence/2026-11-12-worker-runtime/mcp-oauth-code-borrow-lint-failure.log),
  [Pierwszy PASS](evidence/2026-11-12-worker-runtime/mcp-oauth-code-initial-pass.log),
  [Końcowy PASS](evidence/2026-11-12-worker-runtime/mcp-oauth-code-final-pass.log).
- **Końcowo73 MCP +6 HTTP tests oraz all-targets Clippy -D warnings obu pakietów PASS**, exit0. Rzeczywisty lokalny token endpoint sprawdza resource, PKCE, redirect i metody auth. Pozostałe cases: replay, obcy user/workspace/connection, issuer missing/mismatch/error, expiry/admission, reserved query injection, brak retry400 i over-scoped token.
- [Kontrakt](mcp-client.md#oauth-pkce-callback-i-wymiana-kodu) opisuje granice biblioteki. Kontrola122 Markdown i whitespace PASS. Nadal otwarte: refresh/revoke/persistence/CIMD, HTTP callback/bind browser cookie/UI, schematy/form i wykonanie. To nie jest odbiór całego E5.2 lub realnego providera. Formalnie20/68.


## 2026-11-12 — E5.2: refresh, revocation i blokowanie niepewnych poświadczeń

- Dodano explicit refresh i RFC7009 revoke, wspólny token parser/POST auth oraz pin client ID/resource. AuthContext ma też connection_revision; callback/token access/refresh/revoke wymagają zgodnego świeżego context. Nie jest to dowód pochodzenia DTO; aplikacja musi pobierać context z własnego auth/store.
- Refresh przechodzi do RefreshUncertain przed HTTP, blokując dodatnie użycie po utraconej odpowiedzi lub błędzie. Sukces atomowo w pamięci wymienia access/refresh; brak nowego refresh zachowuje stary. Scopes są ograniczone do dotychczasowych, resource i metoda klienta przypięte. Brak automatycznego retry starego refresh.
- Revoke wyłącza lokalnie przed siecią. Weryfikuje origin/query/auth metadata endpointu, wysyła oddzielne znane refresh/access tokens i akceptuje200 także dla wartości już nieważnej. Niepewny wynik zachowuje lokalną blokadę i umożliwia jawne ujemne sprzątanie. Sukces usuwa sekrety i ustawia Revoked; nie jest obietnicą usunięcia wszystkich tokenów grantu AS.
- Pierwszy suite **80 MCP +6 HTTP tests i Clippy PASS**. Końcowy ze sprawdzeniem metadata revocation/default Basic oraz callback connection revision: **81 MCP +6 HTTP tests i all-targets Clippy -D warnings obu pakietów PASS**, exit0, peak737492992B/OOM0.
  [Pierwszy](evidence/2026-11-12-worker-runtime/mcp-oauth-lifecycle-first-pass.log),
  [Końcowy](evidence/2026-11-12-worker-runtime/mcp-oauth-lifecycle-final-pass.log).
- Testy rzeczywistego lokalnego HTTP obejmują rotację i non-rotation dla public/Basic/Post, utratę odpowiedzi po odebraniu refresh request, brak dodatniego retry, jawne revoke,503 i powtórzenie ujemnego sprzątania, cross-user/workspace/connection/revision/client odmowę, brak endpointu, access-only oraz scope escalation.
- [Kontrakt](mcp-client.md#oauth-refresh-revoke-i-status-poświadczeń) opisuje ograniczenie in-memory i wymaganie durable refresh intent przed siecią. Persistence/CIMD, callback HTTP/browser cookie/UI, schema/form i wykonanie pozostają otwarte. Dokumentacja122 Markdown i diff whitespace PASS; E5.2 nadal niezaznaczony, formalnie20/68.


## 2026-11-12 — E5.2: trwały dziennik OAuth, generacje i CAS

- Dodano CredentialJournal oraz implementację w głównym Store serwera, migracja37→38. SQLite zawiera wyłącznie context/binding/generację/stan/zamiar/referencje/digest. IMMEDIATE transakcje sprawdzają bieżące członkostwo, właściciela i rewizję; admin nie dziedziczy osobistego połączenia innej osoby.
- install_durable/refresh_durable zapisują zamiar przed operacją i nowy prywatny rekord pod unikalną referencją przed CAS aktywującym generację. Rezerwacja refresh jest zatwierdzona przed HTTP; niezakończony zamiar po restarcie nie przywraca starego tokenu. disable zwiększa rewizję i odrzuca spóźniony commit. access_token_durable sprawdza dziennik ponownie po pobraniu sekretu.
- Prywatny envelope jest ograniczony do64KiB i Zeroizing, związany z context, generacją oraz fingerprintem endpointów/issuer/resource/registration. Restore sprawdza SHA256, binding i lokalną ważność; cofnięcie zegara względem zapisu blokuje restore. Wszystkie referencje pozostają w ewidencji, także po wygranym disable i późnym zapisie starego procesu. Nie ma jeszcze GC/trwałego remote-revoke; limity64 połączenia i256 referencji ograniczają wzrost.
- Pierwszy przebieg klienta: **86 MCP +6 HTTP tests i all-targets Clippy -D warnings PASS**; [log](evidence/2026-11-12-worker-runtime/mcp-oauth-durable-first-pass.log). Obejmuje kolejność rezerwacji/zapisu, utratę HTTP response, awarie vault/commit, brak retry oraz odrzucanie podmienionych/przeterminowanych rekordów. Późniejsza poprawka rezerwuje pełny bufor envelope przed serializacją, aby uniknąć realloc sekretów; końcowy przebieg poniżej obejmuje tę wersję.
- Pierwszy server check wykrył niezgodne u64 w rusqlite i niedostępne UUIDv4; dostosowano do i64/positive_revision i UUIDv7 używanych w repo. [Log](evidence/2026-11-12-worker-runtime/server-oauth-durable-compile-failure.log). Następny compile zakończył się potwierdzonym cgroup OOM przy codegen8, peak3489660928B/oom_kill1; [log](evidence/2026-11-12-worker-runtime/server-oauth-durable-codegen8-oom.log). Launcher używa teraz64 codegen units i scratch1536MiB dla zestawów kompilujących pełny relay test binary, zachowując RAM3328MiB, jeden build job i izolację.17 testów launchera PASS.
- Po zmianie codegen64 pełny relay przebieg zakończył się bez OOM (peak3456512000B, oom_kill0): **555 PASS,3 FAIL,31 ignored**. Wszystkie7 nowych przypadków OAuth PASS; trzy błędy dotyczyły historycznych fixtures migracji (jedna pominięta tabela w jobs i dwa powtórne DROP w naprawie kolizji). Fixtures poprawiono; [zachowany log](evidence/2026-11-12-worker-runtime/server-oauth-durable-migration-fixtures-failure.log).
- Końcowy all-targets Clippy -D warnings serwera i powiązanych pakietów PASS, peak1965404160B/OOM0; [log](evidence/2026-11-12-worker-runtime/server-oauth-durable-clippy-pass.log). Końcowy pełny server suite PASS: **558 relay +1 identity +67 session-runtime +27 terminal +37 usage**,31 jawnie pominiętych testów wymagających osobnych suites, peak3397627904B/OOM0; [log](evidence/2026-11-12-worker-runtime/server-oauth-durable-final-pass.log). Rust source SHA256 dc7ec5a142094fb7e75e6440b099f4a2fe7537ce227acc676a1fbfe06e563189 jest wspólny z końcowym Clippy. Końcowy MCP suite również PASS: **86 MCP +6 HTTP tests i all-targets Clippy -D warnings obu pakietów**, ten sam Rust source SHA256, peak738738176B/OOM0; [log](evidence/2026-11-12-worker-runtime/mcp-oauth-durable-final-pass.log). [17 testów launchera](evidence/2026-11-12-worker-runtime/oauth-durable-launcher-pass.log),122 Markdown/local links/anchors oraz diff whitespace PASS. Produkcyjne podłączenie magazynu sekretów, callback HTTP/browser cookie/UI, trwałe revoke/GC i CIMD pozostają otwarte; E5.2 nadal niezaznaczony. Formalnie20/68.


## 2026-11-13 — E5.2: adapter SecretStore, krótkie blokady i browser disconnect

- CredentialJournal obsługuje Mutex<J>, blokując wyłącznie pojedynczą operację dziennika. Poisoned Mutex odrzuca odczyty i mutacje; nie odzyskuje automatycznie dostępu. OAuthCredentials w crates/vault podłącza install/refresh/access_token/disable do istniejącego SecretStore. Konstruktor relay wybiera odseparowaną przestrzeń keyringu; nie wykonuje I/O przy konstrukcji. Nie uruchomiono rzeczywistego keyringu ani konta dostawcy.
- Adapter przyjmuje tylko referencje mcp-oauth:<UUID>, ogranicza rekord do64KiB i odrzuca surowe błędy backendu. Rekordy nie są rejestrowane jako narzędzia lub polityki vault MCP. Nie ma fallback do plików/SQLite i nie ma przedwczesnego usuwania starych referencji.
- Suite mcp obejmuje teraz także dunlin-vault i jego all-targets Clippy. Pierwszy przebieg **86 MCP +6 HTTP +31 vault tests i Clippy PASS**, peak874913792B/OOM0; [log](evidence/2026-11-12-worker-runtime/mcp-oauth-vault-first-pass.log).17 testów launchera PASS. Cargo.lock zmienia wyłącznie własną listę zależności dunlin-vault.
- Dodano właścicielskie GET status i POST disconnect pod /v1/web/workspaces/{id}/mcp-connections/{connection}. Aktualne cookie/device/IdP/host policy są weryfikowane pod blokadą Store; user nie pochodzi z body. Status nie zwraca referencji, digestu, bindingu ani tokenu. Disconnect wymaga same-origin i expected_revision, zwiększa rewizję, zapisuje audyt w tej samej transakcji i publikuje invalidation dopiero po commit. Wyraźnie zwraca provider_revocation=not_confirmed; nie czyta keyringu i nie wysyła provider HTTP.
- Istniejący test realnego lokalnego OAuth/SQLite używa teraz produkcyjnego OAuthCredentials i interfejsu SecretStore z fikcyjnym backendem. Sprawdza brak Store lock podczas keyring get/set i HTTP, restart po zamknięciu wszystkich starych Store handles, utratę odpowiedzi, błąd zapisu przed/po zapisie wartości, odłączenie w trakcie HTTP oraz po zapisie sekretu i brak retry starego tokenu. Dodatkowe testy obejmują poisoned journal, atomic audit failure i owner/CSRF/current browser authority na endpointach.
- Pierwszy Clippy serwera wykrył wyłącznie zbędny import trait w teście; usunięto. [Log](evidence/2026-11-12-worker-runtime/server-oauth-vault-clippy-unused-import.log). Pełny server suite PASS: **562 relay +1 identity +67 session-runtime +27 terminal +37 usage**,31 testów wymagających osobnych suites jawnie pominiętych, peak3406938112B/OOM0; [log](evidence/2026-11-13-mcp-oauth/server-oauth-vault-pass.log). Rust source SHA2563998293258339a067ef58cfeda7832129ce0cd43a41d3b1b1d8a8a57ed5e5302. Końcowy all-targets Clippy -D warnings serwera i powiązanych bibliotek PASS, peak1971052544B/OOM0; [log](evidence/2026-11-13-mcp-oauth/server-oauth-vault-clippy-pass.log), ten sam snapshot. Wspólny końcowy suite MCP PASS: **86 MCP +6 HTTP +31 vault tests oraz all-targets Clippy -D warnings wszystkich trzech pakietów**, ten sam snapshot; [log](evidence/2026-11-13-mcp-oauth/mcp-oauth-vault-pass.log). [17 testów launchera](evidence/2026-11-13-mcp-oauth/oauth-vault-launcher-pass.log),122 Markdown/linki/anchors i whitespace PASS. Start/callback HTTP z browser binding, odświeżanie w brokerze, UI, trwałe revoke/GC i CIMD pozostają otwarte. E5.2 nadal niezaznaczony; formalnie20/68.


## 2026-11-13 — E5.2: katalog providerów operatora, start i browser callback OAuth

- Dodano walidowaną tablicę mcp_oauth w relay.toml: do16 providerów z przypiętym endpoint/issuer/client_id/redirect/origins/scopes i public/basic/post. Przeglądarka wybiera wyłącznie ID; rejestracja confidential używa referencji mcp-oauth-client: w prywatnym SecretStore, bez sekretu w konfiguracji lub API. Provider ID istniejącego połączenia nie zmienia się przy ponownej autoryzacji. To nie jest jeszcze CIMD/dynamic registration.
- GET provider catalog, POST authorize i GET callback są podłączone do AppState/routera. Bootstrap robi nieuwierzytelnione discovery i sprawdza przypiętego issuera. PKCE/state/issuer oraz wymiana kodu używają istniejącej biblioteki; rezultat trafia wyłącznie do OAuthCredentials. Nie wykonano operacji na rzeczywistym keyringu, koncie ani tailnecie.
- Migracja38→39 dodaje authorizing i opcjonalne provider_id, kopiując rodzica i wszystkie referencje w jednej transakcji. Niepewny refresh pozostaje zablokowany. Stan authorizing/fingerprint jest trwały przed zwróceniem authorization_url. Disconnect zwiększa rewizję i blokuje późny callback; jawne wznowienie niedokończonej autoryzacji również podnosi rewizję. Właściciel ma GET listy połączeń do odzyskania kontekstu po utracie odpowiedzi start.
- Główne cookie pozostaje Strict. Dedykowane HttpOnly/Lax flow cookie ma TTL300s, ścieżkę callbacku, hash w pending i Secure dla HTTPS. Pending wiąże hash pierwotnej sesji, device/user/context, state, provider/config i flow; globalnie32 pending/starting,4 na użytkownika, jedna oczekująca operacja na sesję i2 równoległe operacje blokujące. Callback nie potrzebuje przesłania głównego Strict cookie, lecz odczytuje aktualność pierwotnej sesji przez zachowany hash. Złe cookie/state nie zużywa prawidłowej próby ani nie czyści jej cookie.
- Wydzielono checked journal. Sprawdzenie sesji/device/IdP/host policy i zapis generacji mają wspólną transakcję SQLite. Test z drugą bazodanową Connection potwierdza blokadę konkurencyjnej mutacji podczas authority check; HTTP/keyring pozostają poza Store lock. Wylogowanie przed token POST, podczas niego lub po zapisie sekretu blokuje aktywację. Callback jest jednorazowy; komunikaty nie odzwierciedlają AS error/code/token; sukces kieruje wyłącznie do / z no-store/no-referrer.
- Pierwszy Clippy wykrył widoczność stałej migracji, kolejny lexical lifetime MutexGuard w teście; obie kwestie poprawiono bez suppressions. [Widoczność](evidence/2026-11-13-mcp-oauth/server-oauth-flow-visibility-failure.log), [testowy lock](evidence/2026-11-13-mcp-oauth/server-oauth-flow-test-lock-lint.log).
- Pierwszy pełny suite **571 relay +1 identity +67 session-runtime +27 terminal +37 usage PASS**,31 jawnie pominiętych osobnych acceptance, peak3390468096B/OOM0; [log](evidence/2026-11-13-mcp-oauth/server-oauth-flow-first-pass.log). Przypadki obejmują przepływ code bez main cookie w callbacku, obce state/cookie, replay, logout/disconnect/config/issuer, wylogowanie po AS/keyring, restart usługi, TTL/capacity i niepuste migracje/FK/collision rollback. Po poprawce Secure dla TLS loopback i zaostrzeniu hooków na try_lock pełny suite przeszedł ponownie: **572 relay +1 identity +67 session-runtime +27 terminal +37 usage**,31 jawnie pominiętych acceptance, peak3401617408B/OOM0; [log](evidence/2026-11-13-mcp-oauth/server-oauth-flow-full-pass.log).
- Dalsze uszczelnienie ogranicza jawne connection_id do istniejącego własnego połączenia (nowe ID nadaje tylko serwer) oraz dodaje no-store/no-referrer na granicy trasy, także dla wczesnych błędów cookie/state. [Pierwszy scoped PASS21](evidence/2026-11-13-mcp-oauth/server-oauth-flow-owner-guard-pass.log). **Końcowy scoped PASS21 testów OAuth** (browser/API/config/journal/migracja), peak3394531328B/OOM0; [log](evidence/2026-11-13-mcp-oauth/server-oauth-flow-component-final-pass.log). All-targets Clippy -D warnings serwera/powiązanych pakietów PASS, peak2004541440B/OOM0; [log](evidence/2026-11-13-mcp-oauth/server-oauth-flow-headers-clippy-pass.log). Te dwa końcowe przebiegi mają wspólny snapshot e25be79aead0c1aea0ebd5e1ccf52f240d144361fc07a343ec9a906e81f24c72; pełny572 dotyczy poprzedzającej wersji c686031e8f630b7f660405029b78fb4b1bfd5d9db2a4aea6bb6b5172c539bfdc. Po zmianie tylko trasy start/callback powtórzono komponent zamiast niezwiązanych suites.122 Markdown i whitespace PASS.
- [Kontrakt i przykład konfiguracji](mcp-client.md#rozpoczęcie-i-callback-oauth-w-serwerze) zawierają źródła RFC9700 i cookie SameSite. Odbiór w Chromium, panel UI, broker/refresh, trwałe revoke/GC, CIMD i schema/form pozostają otwarte; REST fixture nie jest odbiorem realnego providera ani całego E5.2. Formalnie20/68.


## 2026-11-13 — E5.2: nazwane prywatne połączenia MCP i odbiór UI OAuth

- Dodano widok Połączenia MCP: wybór zespołu/providerów operatora, prywatna nazwa konta, logowanie na stronie AS, własne statusy, anulowanie niedokończonego logowania, lokalne disconnect i jawna ponowna autoryzacja. Migracja39→40 dodaje opcjonalne display_name bez modyfikacji poświadczeń/referencji; nazwa ma1–120 znaków bez controls i jest zachowywana przy reautoryzacji. Wspólna SCHEMA_VERSION zastępuje powtarzane numery aktualnej wersji w assertions migracji.
- Żądania MCP UI przypinają X-Dunlin-Expected-User. Serwer sprawdza go po aktualnym cookie/device/IdP/host policy; mismatch/duplikat daje401. Nie jest to nowe poświadczenie. Dodano HTTP test starej tożsamości na katalogu, start, status i disconnect oraz zachowania niezmienionej rewizji/nazwy. Testy Store obejmują bounded/private name i migrację40 na niepustej bazie z collision rollback.
- Frontend waliduje ograniczony JSON, ID/revision i URL OAuth (same-origin callback, code/S256, state/challenge, brak secret-shaped params). Mutacje nie mają automatycznego retry; niepotwierdzony wynik blokuje kolejne do jawnego odświeżenia. Abort/unmount odrzuca spóźniony start bez nawigacji; zmiana członkostwa ponownie sprawdza zespoły. Jednorazowy return marker w sessionStorage zawiera wyłącznie user/workspace; brak credential/state/code/URL i brak localStorage.
- Pierwszy Clippy wykrył brak nowego pola w initializerze projekcji SQL, a pierwszy typecheck brak definite assignment; poprawiono. Zachowano [lint](evidence/2026-11-13-mcp-oauth/mcp-ui-clippy-first.log) i [typecheck](evidence/2026-11-13-mcp-oauth/mcp-ui-typecheck-first.log). Dwa pierwsze przebiegi Chromium wykazały błąd native Window.fetch wywołanego z this klienta. Diagnostyka potwierdziła brak requestów MCP i komunikat błędu w panelu; wywołanie jest teraz niezwiązane z obiektem klienta. Zachowano [pierwszy](evidence/2026-11-13-mcp-oauth/mcp-ui-browser-first.log) i [diagnostyczny](evidence/2026-11-13-mcp-oauth/mcp-ui-browser-second.log) przebieg oraz dodano regresję calling convention do testów Node.
- **135 frontend tests PASS**, w tym7 testów klienta MCP; [log](evidence/2026-11-13-mcp-oauth/mcp-ui-frontend-final.log). **17 testów launchera PASS**; [log](evidence/2026-11-13-mcp-oauth/mcp-ui-launcher.log).
- **Produkcyjne UI/Chromium OAuth PASS**, exit0, peak3412701184B/OOM0; [log](evidence/2026-11-13-mcp-oauth/mcp-ui-browser-pass.log). Relay127.0.0.1 i strona zgody127.0.0.2 są różnymi sites. Rzeczywista nawigacja potwierdza brak głównego Strict cookie i obecność dedykowanego Lax cookie w obu callbackach, jednorazowe code i PKCE/resource na token endpoint, powrót do nazwanej karty oraz brak tokenów w UI/storage. Route interception symuluje utracone odpowiedzi po server commit, bez automatycznych retry, oraz opóźniony start po opuszczeniu panelu. Rust sprawdza dokładnie4 starty,3 disconnect,2 AS authorizations/exchanges/private writes i jedno connected połączenie w SQL. Zrzuty [desktop](evidence/2026-11-13-mcp-oauth/mcp-ui-desktop.png) i [mobile](evidence/2026-11-13-mcp-oauth/mcp-ui-mobile.png) obejrzano; brak poziomego overflow przy390px. Base64 obrazów wydzielono z tekstowego logu do PNG.
- Pełny server suite **575 relay +1 identity +67 session-runtime +27 terminal +37 usage PASS**, exit0,32 jawnie pominięte acceptance uruchamiane osobnymi suites, peak3463192576B/OOM0; [log](evidence/2026-11-13-mcp-oauth/mcp-ui-server-pass.log). Obejmuje także nowe testy stale identity/name/migration40 oraz wcześniejsze fixtures migracji.
- Końcowy **all-targets Clippy -D warnings serwera i powiązanych bibliotek PASS**, exit0, peak2030514176B/OOM0; [log](evidence/2026-11-13-mcp-oauth/mcp-ui-clippy-pass.log). Wspólny Rust source SHA256 z pełnym server suite: 0a92482080ce44831ace55b06a024930f532dae9cdb8fe6c2c0e420ab914b6de. Browser suite obejmuje też produkcyjny tsc/Vite build. Kontrola122 Markdown/linków/anchors i whitespace PASS.
- Ten odbiór używa lokalnych fixtures HTTP i MemoryStore, bez rzeczywistego konta, keyringu lub tailnetu. Broker/refresh podczas wykonania, trwałe revoke/GC, CIMD, schema/form i pozostałe wymagania E5.2/E5.3 pozostają otwarte. E5.2 nadal niezaznaczony; formalnie20/68.


## 2026-11-13 — E5.2: broker katalogu, odświeżanie przed dispatch i aktualna authority

- Dodano POST /v1/web/workspaces/{workspace}/mcp-connections/{connection}/inspect z expected_revision oraz przycisk Sprawdź narzędzia. Provider pochodzi wyłącznie z własnego połączenia i katalogu operatora. Odpowiedź zawiera ID/revision, potwierdzoną wersję MCP i ograniczone nazwy narzędzi; nie zawiera tokenów ani referencji. Katalog jest obserwacją, nie grantem tools/call. Runtime narzędzi pozostaje osobnym wymaganiem E5.2/E5.3.
- Broker używa produkcyjnego negotiated Client i OAuthCredentials. Przypina browser/device/IdP/host policy, rewizję członkostwa, połączenia i dokładną generację. Checked journal sprawdza aktualną authority w tej samej transakcji SQL co odczyt/zapis generacji. Keyring/HTTP działają poza Store lock i w dwóch ograniczonych slotach; testy hooków wymagają try_lock podczas I/O.
- DispatchGuard sprawdza aplikacyjną authority przed/po HTTP i odczycie cache; odmowa jest terminalna. Obejmuje modern, legacy initialize/notification/DELETE i odczyt subskrypcji. Broker odrzuca także końcowy katalog po zmianie generacji. Odebranie/regrant członkostwa nie odtwarza trwającego scope. Testy obejmują zero HTTP przy initial deny, cached catalog po revoke/regrant, odrzucenie odpowiedzi podczas HTTP i brak dalszego legacy initialization.
- access_token_or_refresh_durable rozpoznaje wyłącznie lokalnie wygasły, prawidłowy envelope. Refresh CAS przypina dokładnie odczytaną generację; nie obraca nowszego tokenu po konkurencyjnej wymianie. Zwrócony bearer ma przypisany record używany przez dispatch guard. Brak automatycznego retry MCP401 i niepewnego refresh. Testy obejmują valid/no rotation, expiry/one rotation, utratę HTTP response i utratę CAS przed token POST.
- Pierwszy MCP compile wykrył użycie prywatnego pola fixture Reply; zastąpiono je publicznym konstruktorem. [Log](evidence/2026-11-13-mcp-oauth/mcp-broker-client-first.log). Następny zestaw91 MCP +6 HTTP +31 vault i Clippy PASS; [log](evidence/2026-11-13-mcp-oauth/mcp-broker-client-second.log). Pierwszy Clippy serwera wykrył tylko needless borrow w nowym teście; poprawiono bez suppressions. [Log](evidence/2026-11-13-mcp-oauth/mcp-broker-clippy-first.log).
- Końcowy **92 MCP +6 HTTP +31 vault tests i all-targets Clippy trzech pakietów PASS**, peak881053696B/OOM0; [log](evidence/2026-11-13-mcp-oauth/mcp-broker-client-pass.log). **136 frontend tests PASS**, w tym walidacja katalogu i jawnego POST/CAS; [log](evidence/2026-11-13-mcp-oauth/mcp-broker-frontend.log).
- Pełny **578 relay +1 identity +67 session-runtime +27 terminal +37 usage PASS**,32 jawnie pominięte osobne acceptance, peak3388776448B/OOM0; [log](evidence/2026-11-13-mcp-oauth/mcp-broker-server-pass.log). Nowe HTTP cases potwierdzają ownership/CAS/CSRF, rzeczywisty catalog POST bez tools/call, refresh przed katalogiem, trwałe zablokowanie utraconego refresh oraz brak wyniku po logout podczas secret read, disconnect podczas katalogu i membership revoke/regrant.
- Końcowy **all-targets Clippy -D warnings serwera/powiązanych pakietów PASS**, peak2016800768B/OOM0; [log](evidence/2026-11-13-mcp-oauth/mcp-broker-clippy-pass.log). Clippy i końcowy MCP suite mają source SHA256 a28711f73ddc966fe435421ada233625751a0ec61d0f678dbb156d5ef9e29058. Pełny server suite ma11a1ea70bf7ddc2e91032adb81bd114765b5d3779f421851cf03fcbfe63353c8: późniejsze zmiany dotyczą wyłącznie grupowania przycisków w JSX i apps/web/README.md. Odwrócenie dokładnie tych dwóch zmian odtwarza hash całego wcześniejszego snapshotu; [dowód różnicy](evidence/2026-11-13-mcp-oauth/mcp-broker-snapshot-difference.json). Kod Rust jest wspólny.
- **Produkcyjne UI/Chromium PASS** na końcowym snapshot a28711f73ddc966fe435421ada233625751a0ec61d0f678dbb156d5ef9e29058, exit0, peak3489660928B, oom_kill0; [log](evidence/2026-11-13-mcp-oauth/mcp-broker-browser-pass.log). Tsc/Vite build i cały wcześniejszy OAuth/lost responses/unmount scenariusz nadal PASS. Nowy fragment wymaga pokazanego MCP2026-07-28 i lookup, dokładnie jednego catalog read u providera, braku tools/call, braku tokenów w DOM/storage i końcowego connected w SQL. Zrzuty [desktop](evidence/2026-11-13-mcp-oauth/mcp-broker-desktop.png) i [mobile](evidence/2026-11-13-mcp-oauth/mcp-broker-mobile.png) obejrzano; przyciski i katalog mieszczą się przy390px bez poziomego overflow.122 Markdown/linki/anchors i whitespace PASS.
- Nie użyto rzeczywistych kont, keyringu ani tailnetu. Do zakończenia E5.2/E5.3 pozostają wywołania narzędzi w runtime z allowlist/scoped secrets/kill switch i zgodami, trwałe revoke/GC, CIMD oraz schema/form. E5.2 pozostaje niezaznaczony; formalnie20/68.


## 2026-11-13 — E5.2: walidacja input/output JSON Schema i typowane formularze MRTR

- Klient kompiluje inputSchema i outputSchema przez przypięte jsonschema0.46.0. Oba transporty wymagają definicji z pobranego katalogu; pełny pin obejmuje też outputSchema. Błędne argumenty są odrzucane przed tools/call, bez konwersji i dopisywania default. Udany wynik z outputSchema wymaga pasującego structuredContent; błędny wynik daje Protocol bez retry. Adapter2025 nie raportuje nieobsługiwanego input_required jako sukcesu.
- Dodano profil referencji ograniczony do fragmentów jednego dokumentu, lokalnych kotwic i JSON Pointer. Nie ma odczytu HTTP/plików, zagnieżdżonych resource IDs ani custom metaschemas. Domyślny draft2020-12 i jawne4/6/7/2019-09/2020-12 korzystają z walidatora; kompilacja odrzuca nieobsługiwane regex zamiast pomijać ograniczenie. Liniowy silnik usuwa ryzyko traktowania wyczerpanego backtrack limit jako false pod not/if. Przykłady/enum/default pozostają danymi, nie dyrektywami.
- Limity obejmują bajty/węzły/głębokość, ważony graf referencji, liczbę i pamięć regex oraz produkt kosztu schematu i rozmiaru instancji. Analiza odrzuca wykładnicze DAG i cykle bez postępu instancji, zachowując rekursję przez właściwości/elementy. Sprawdzenie authority następuje również po przetworzeniu katalogu/form/wyników, zanim dane zostaną zwrócone integratorowi. To limity strukturalne; twarda izolacja CPU nie jest tym odbiorem.
- InputSupport::elicitation_form jest jawne i domyślnie false. Pending ma form_requests oraz dotychczasowe URL input_requests. continue_with_responses sprawdza dokładny zestaw kluczy, tryb i content; accept wymaga danych zgodnych z FormSchema, decline/cancel nie mogą przemycić content. Obsługiwany płaski profil obejmuje string/number/integer/boolean, enum/oneOf i multi-select enum/anyOf, wymagane pola, granice i email/uri/date/date-time. Odrzucane są niezamówione pola, duplikaty wyborów, błędne default i niebezpieczna reprezentacja integer. Nie ma auto-akceptacji ani używania danych formularza jako authority. Broker katalogu nadal nie reklamuje elicitation; UI i runtime potrzebują osobnego podłączenia.
- Publiczne zależności pobrano w ograniczonym Dockerze, anonimowo, bez host credentials i bez kompilacji na hoście. Pierwsza próba zapisu przez bind mount była readonly; zastąpiono ją tmpfs i eksportem archiwum. [Pierwsza próba](evidence/2026-11-13-mcp-schema/dependency-fetch-readonly-failure.log), [fetch PASS](evidence/2026-11-13-mcp-schema/dependency-fetch-pass.log), [offline resolver](evidence/2026-11-13-mcp-schema/dependency-lock-pass.log). Dodano16 pakietów; wersje i checksumy istniejących pakietów pozostały bez zmian, poza aktywacją zależności/kwalifikowaniem nazw w lock. [Checksumy, licencje i metadata MSRV](evidence/2026-11-13-mcp-schema/schema-dependencies.json). Dodano notice jsonschema/MIT i borrow-or-share/MIT-0 oraz allow MIT-0; nie jest to wyjątek advisory. Resolver stosuje metadata1.92; rzeczywiste testy wykonano na1.99, bez deklarowania odbioru MSRV.
- Pierwszy MCP suite nie mógł rozpakować nowych źródeł w readonly registry; uzupełniono zweryfikowane źródła cache. [Log](evidence/2026-11-13-mcp-schema/mcp-schema-first.log). Następny zestaw105 MCP +6 HTTP +31 vault PASS, lecz Clippy wykrył nieużywane pole i dwa collapsible-if; poprawiono bez suppressions. [Log](evidence/2026-11-13-mcp-schema/mcp-schema-second.log). Kolejne106 i107 cases wraz z Clippy PASS: [106](evidence/2026-11-13-mcp-schema/mcp-schema-third.log), [107](evidence/2026-11-13-mcp-schema/mcp-schema-fourth.log). Rozszerzenia grafu/ref i danych adnotacji mają dodatkowe zachowane przebiegi.
- **Końcowy 110 MCP +6 HTTP +31 vault tests oraz all-targets Clippy -D warnings trzech pakietów PASS**, exit0, peak1216049152B/OOM0; [log](evidence/2026-11-13-mcp-schema/mcp-schema-client-verified.log). Source SHA256 144b014b5cd742a1b4556730e91c82fa83e7950354de75c65d19cd465ed13224. Testy obejmują no-HTTP dla złych argumentów, pin outputSchema, walidację wyników bez retry, zgodność draftów, rekursję i DAG, brak zewnętrznych odczytów, literalne dane enum/default, limity oraz form accept/decline/cancel, oba tryby w jednej rundzie, default form mode i odmowę po revoke.
- Pierwsza pełna regresja przed końcowym limiterem grafu: **578 relay +1 identity +67 session-runtime +27 terminal +37 usage PASS**,32 osobne acceptance pominięte, peak3483361280B/OOM0; [log](evidence/2026-11-13-mcp-schema/mcp-schema-server-first.log). Jeden późniejszy przebieg świadomie zatrzymano po aktualizacji kodu i nie liczono jako PASS; [superseded](evidence/2026-11-13-mcp-schema/mcp-schema-server-superseded.log). Końcowy odbiór aktualnej wersji jest zapisany poniżej.
- **Końcowy pełny server suite 578 relay +1 identity +67 session-runtime +27 terminal +37 usage PASS**, exit0,32 jawnie pominięte osobne acceptance, peak3488247808B/OOM0; [log](evidence/2026-11-13-mcp-schema/mcp-schema-server-verified.log). **All-targets Clippy -D warnings serwera/powiązanych bibliotek PASS**, peak2154561536B/OOM0; [log](evidence/2026-11-13-mcp-schema/mcp-schema-server-clippy.log). Oba zestawy i końcowy klient110 mają wspólny source SHA256 144b014b5cd742a1b4556730e91c82fa83e7950354de75c65d19cd465ed13224. Kontrola122 Markdown/linków/anchors i diff whitespace PASS. Frontend nie był zmieniany; odbiór UI formularzy nie jest deklarowany.
- [Kontrakt i limity](mcp-client.md#walidacja-schematów-narzędzi) opisują lokalny profil i granice SDK. E5.2 nadal wymaga brokera wykonania narzędzi, form UI/runtime, trwałego revoke/GC i CIMD; E5.3 obejmuje allowlisty/scoped secrets/kill switch. Nie użyto realnego konta, keyringu lub tailnetu. E5.2 pozostaje niezaznaczony; formalnie20/68.


## 2026-11-13 — E5.2: publiczny klient CIMD i eksport dokumentu

- Dodano jawny tryb authentication=client_metadata_document w operatorowym katalogu providerów oraz Registration::client_metadata_document w bibliotece. Wymaga kanonicznego HTTPS client_id z niepustą ścieżką, bez query/fragment/userinfo/dot segments i do256 bajtów. Discovery musi potwierdzić wsparcie CIMD, code/S256 i token auth none. Brak wsparcia daje odmowę przed utworzeniem pending/sekretu; nie ma fallback do prerejestracji lub DCR. Issuer/resource/origin/callback nadal są przypięte.
- ClientMetadataDocument tworzy niezmienny publiczny JSON z nazwą, dokładnymi callbackami, grant_types authorization_code/refresh_token, response_types code i auth none. Komenda dunlin-relay mcp-client-metadata --config PATH --provider ID eksportuje dokument bez startu serwera, otwierania bazy/keyringu i sieci. Odrzuca niejednoznaczne argumenty, inny tryb auth i sekrety klienta w CIMD. Operator publikuje JSON osobno pod dokładnym HTTPS client_id; nie otwarto publicznego relay/Funnel ani rzeczywistej usługi. Odczyt dokumentu i walidacja jego treści należą do AS. Brak private_key_jwt i DCR jest jawną granicą tego publicznego profilu.
- Tryb CIMD ma osobny durable credential binding nawet przy tym samym client_id co prerejestrowany public client. Istniejące public/basic/post zachowują poprzednie bindingi. Code exchange i refresh wysyłają dokładny URL client_id bez client secret; PKCE, resource binding, jednorazowy callback i kontrola authority pozostają wymagane. Dokument i tryb rejestracji nie nadają praw do narzędzi ani nie przekazują tokenów runnerowi.
- **112 MCP +6 HTTP +31 vault tests oraz all-targets Clippy -D warnings trzech pakietów PASS**, exit0, peak1219416064B/OOM0; [log](evidence/2026-11-13-mcp-cimd/mcp-cimd-client.log). Rozszerzone fixtures obejmują faktyczny code exchange/S256 i rotacje refresh w CIMD, odmowę bez wsparcia AS/PKCE/public auth, issuer pin oraz rozdzielenie bindingów. Source SHA256 cace9f56baef4ac02270ee67b46a9db0b8ed290e1338952323997a1f1412eed5.
- Pełna kompilacja serwera osiągnęła cgroup OOM przy RAM3328MiB i globalnych64 CGU: [log](evidence/2026-11-13-mcp-cimd/mcp-cimd-server-cgu64-oom.log). Próby128 CGU oraz64 tylko dla relay/8 dla zależności również OOM: [128](evidence/2026-11-13-mcp-cimd/mcp-cimd-server-cgu128-oom.log), [mixed](evidence/2026-11-13-mcp-cimd/mcp-cimd-server-mixed-cgu-oom.log). Ostatnia zachowuje df/du: około1,2GiB artefaktów na tmpfs. Limit wyłącznie suite server zwiększono do3584MiB, przy tym samym scratch1536MiB, CGU64, readonly/cache, UID/capabilities/PID/network none. VM testowa nadal4GiB, bez równoległych ciężkich suites. Launcher raportuje df/du także po błędzie.
- Pierwszy Clippy serwera zgłosił chunks_exact_to_as_chunks w nowym parserze CLI; poprawiono bez suppressions. [Log](evidence/2026-11-13-mcp-cimd/mcp-cimd-server-clippy-first.log). Pierwszy pełny skompilowany relay miał579 PASS/1 FAIL/32 ignored i peak3586859008B/OOM0: [log](evidence/2026-11-13-mcp-cimd/mcp-cimd-server-assertion-failure.log). Jedyny błąd był w nowej asercji oczekującej nieistniejącego envelope.client_id; rekord prywatny zapisuje binding, a dokładny client_id sprawdza fixture żądania AS. Poprawiono test na istniejące user_id/connection_id. Eksport dokumentu i cała ścieżka start/callback/catalog poprzedzająca tę asercję przeszły. Przebiegu z tą asercją nie zaliczono jako PASS całej regresji.
- **Końcowy all-targets Clippy -D warnings serwera i powiązanych bibliotek PASS**, exit0, peak2142756864B/OOM0; [log](evidence/2026-11-13-mcp-cimd/mcp-cimd-server-clippy-verified.log). Source SHA256 c063c44a4de316a94aad42ef4d59379b10c5cef997bc44655cb78391737ab643. [17 testów launchera](evidence/2026-11-13-mcp-cimd/mcp-cimd-launcher.log) oraz [14 testów CI/tooling](evidence/2026-11-13-mcp-cimd/mcp-cimd-ci-tooling.log) PASS przy końcowej konfiguracji RAM3584MiB i CGU64.
- **Końcowy pełny server suite 580 relay +1 identity +67 session-runtime +27 terminal +37 usage PASS**, exit0,32 jawnie pominięte osobne acceptance, peak3508068352B/OOM0, scratch1327932KiB/1572864KiB; [log](evidence/2026-11-13-mcp-cimd/mcp-cimd-server-verified.log). Nowy REST fixture sprawdza brak pending/secretu przy odmowie CIMD, authorize z URL client_id/S256, prawidłowy callback, dokładny client_id w token POST bez client secret/Authorization, prywatny envelope i udany katalog MCP przez broker. Test eksportu dokumentu z rzeczywistego operatorowego TOML również PASS. Końcowy server i Clippy mają source SHA256 c063c44a4de316a94aad42ef4d59379b10c5cef997bc44655cb78391737ab643.
- Końcowy klient112 i serwer mają identyczny kod biblioteki MCP/OAuth i runtime OAuth. Różnice snapshotów ograniczają się do poprawionego przygotowania TOML/asercji w testach serwera i as_chunks w parserze CLI. Odwrócenie dokładnie tych różnic odtwarza cały hash wcześniejszego client suite: [dowód](evidence/2026-11-13-mcp-cimd/snapshot-difference.json). Nie uruchamiano nowego Chromium ani realnego AS/hostingu dokumentu; frontend pozostaje bez zmian.
- [Konfiguracja i hosting CIMD](mcp-client.md#publiczny-klient-cimd) opisują granice publicznego profilu. E5.2 nadal wymaga brokera wykonania, formularzy UI/runtime i trwałego revoke/GC. E5.3 obejmuje allowlisty/scoped secrets/kill switch. E0–E10 pozostaje aktywnym celem; formalnie20/68. Nie użyto rzeczywistych kont, keyringu, tailnetu ani publikacji.
- Kontrola122 Markdown/local links/anchors i git diff --check PASS; [log dokumentacji](evidence/2026-11-13-mcp-cimd/mcp-cimd-docs.log).


## 2026-11-13 — E5.3: hostowe jednorazowe zgody MCP w dzienniku runtime

- Dodano McpInvocation, McpPermissionChallenge i nieklonowalne/niedeserializowane McpApproval/McpDispatchPermit. Zgoda wiąże provider ID, prywatne connection ID/revision, tool name, digest całej definicji i dokładne argumenty. Request ma hostowy origin dunlin.mcp.v1 zapisany w istniejącej kolumnie request; bez zmiany schematu bazy. Każdy request ID jest jednorazowy w sesji, również po odmowie, błędzie, unknown i ponownym otwarciu SQLite.
- Tool IDs z prefiksem dunlin:mcp: są generowane wyłącznie w hostowej ścieżce. Rejestracje i raporty ACP nie mogą ich tworzyć, zmieniać ani kończyć. Generyczne consume/delivery ACP nie mogą zużyć zgody MCP. Istniejący AHP dispatcher nadal weryfikuje dokładną jednorazową decyzję człowieka. Zmiana któregokolwiek pola wywołania blokuje status/dispatch; kod nie przyjmuje edycji zatwierdzonego payloadu.
- Consume tworzy permit tylko po approve-once; deny unieważnia zgodę bez permitu. Walidacja ponownie sprawdza aktualny lease/epoch, domain authority, TTL, aktywną turę i anulowanie oraz zwraca dowód urządzenia człowieka. Broker nadal musi niezależnie sprawdzić aktualne uprawnienia tego urządzenia, profil/allowlistę, checkout, schemat i generację OAuth. Te typy nie są poświadczeniami i nie wysyłają HTTP.
- Complete/failed i hostowy wynik AHP są zapisywane atomowo. Unknown oraz awaria po consumed zachowują durable unknown przy interrupt/recovery; request nie staje się ponownie dispatchable. Błąd event write lub limitu wycofuje także delivered. Wyniki MCP pozostają danymi w structuredContent, bez interpretowania ich jako AHP action/authority. Argumenty mają limit8KiB/1024 węzłów/depth32, retained result48KiB/4096/depth32, przy event limit64KiB. Licznik serializacji nie alokuje nieograniczonego bufora. Obowiązują też dotychczasowe limity256 zgód na sesję/8192 globalnie/8 aktywnych i TTL1–300s.
- Pierwszy suite miał71 PASS/1 FAIL: nowy fixture anulowania próbował ustawić nieistniejącą fazę SQL; [log](evidence/2026-11-13-mcp-permissions/mcp-permissions-sessions-first.log). Zmieniono go na rzeczywisty cancellation intent. Kolejna kompilacja fixture wykryła u64 bez ToSql; zastosowano istniejącą konwersję sql_integer: [log](evidence/2026-11-13-mcp-permissions/mcp-permissions-sessions-second.log). Nie zaliczono tych prób jako PASS.
- **Końcowe74 session-runtime tests PASS**, exit0, peak922275840B/OOM0; [log](evidence/2026-11-13-mcp-permissions/mcp-permissions-sessions-third.log). Siedem nowych przypadków obejmuje pin każdego pola, rzeczywistą decyzję AHP, próbę ponownego consume także przy skopiowanym prywatnym handle, blokadę ACP, denial/unknown/reopen/recovery ze starym permitem, expiry/cancel/authority/lease, limity, dwa niezależne connections SQLite i rollback completion przy błędzie eventu/oversized result. Source SHA2564e06c9bcb1a0b6bb4dd90d950104c74b39cdaeee94147653eca614802dbe78e5.
- **All-targets Clippy -D warnings serwera i powiązanych bibliotek PASS**, exit0, peak2107662336B/OOM0; [log](evidence/2026-11-13-mcp-permissions/mcp-permissions-clippy.log), ten sam snapshot.
- **Pełny server suite580 relay +1 identity +74 session-runtime +27 terminal +37 usage PASS**, exit0,32 jawnie pominięte osobne acceptance, peak3506888704B/OOM0; [log](evidence/2026-11-13-mcp-permissions/mcp-permissions-server.log). SDK74, Clippy i pełny serwer mają wspólny source SHA2564e06c9bcb1a0b6bb4dd90d950104c74b39cdaeee94147653eca614802dbe78e5. Nie zmieniono frontendu ani klienta MCP; nie uruchamiano nowego Chromium lub rzeczywistego konta/usługi.
- [Kontrakt zgód MCP](mcp-client.md#jednorazowa-zgoda-na-wywołanie-mcp-w-dzienniku-runtime) opisuje granice i obowiązki integratora. Produkcyjny broker HTTP/proxy runnera nadal nie jest podłączony. Do następnego przekroju pozostają allowlisty profilu, prywatny wybór connection/revision przy tworzeniu sesji, runtime OAuth journal/guard i rzeczywisty tools/call z powyższym permitem. E5.2/E5.3 pozostają otwarte; formalnie20/68. Cel E0–E10 pozostaje aktywny.
- Kontrola122 Markdown/local links/anchors oraz git diff --check PASS; [log](evidence/2026-11-13-mcp-permissions/mcp-permissions-docs.log).


## 2026-11-13 — E5.3: wersjonowane allowlisty profilu i piny katalogu MCP

- AgentDefinition schema2 dodaje mcp jako listę do32 dokładnych grantów provider_id/name/definition_digest dla najwyżej8 providerów. Brak wildcard, powtórzeń provider/name, sekretów, URL i prywatnych connection IDs. allows_mcp wymaga poprawności całej definicji i dokładnego dopasowania trzech pól. Schema1 pozostaje wspierany bez niepustej polityki MCP; serde pomija pustą listę, zachowując stare kanoniczne bajty i digest. Default nadal schema1; Builder przechodzi na2 po dodaniu narzędzia. Publikacje pozostają niezmienne i wymagają aktualnego ManageProject/CAS.
- Tool.definition_digest waliduje nazwę i schematy, sortuje rekurencyjnie obiekty także przy preserve_order i liczy SHA256(prefiks dunlin.mcp.tool.v1 +NUL +kompaktowy JSON UTF-8). Pin obejmuje nazwę, inputSchema i opcjonalne outputSchema, bez nieobsługiwanych adnotacji serwera. Kolejność tablic i reprezentacja liczb pozostają istotne. Niezależny golden Python dla lookup/type=object to11f2036a698b4a784c54872e1e1f1d19de3fd7a1fbcba2612ffb6eb8f9044587.
- Prywatne inspect dodaje tool_pins, zachowuje tools jako nazwy i odrzuca katalog większy niż256KiB. Piny nie są grantem wykonania. Backend sprawdza authority/generację po kompilacji/digest/serializacji, zanim udostępni wynik. UI sprawdza zgodność pinów z providerem prywatnego połączenia, obserwowanymi nazwami i unikatowość; starsze odpowiedzi bez pinów pozostają odczytywalne. Builder ma pola allowlisty i publikację; katalog pokazuje kopiowalny pin, bez surowych schematów i tokenów.
- Runtime MCP/proxy nie jest jeszcze podłączony: formularz blokuje profil wymagający MCP i pokazuje unsupported. Lokalny admission odmawia przed procesem, zdalny controller przed trwałym session/execution admission, DTO v3/v4 i bezpośredni session engine odmawiają pominięcia polityki. Nie uruchamiają agenta ze zignorowanym MCP. Prywatne connection/revision mają być wybierane w admission sesji, poza wspólną definicją.
- Pierwszy domain build wykrył próbę użycia serde_json w produkcyjnej domenie, gdzie jest dev dependency; usunięto nadmiarową serializację z validate, zachowując istniejący limit encoded64KiB w transport/storage bez nowej zależności. [Log](evidence/2026-11-13-mcp-policy/mcp-policy-domain-first.log). Końcowe **50 domain +35 relay-protocol tests PASS**, peak526667776B/OOM0; [log](evidence/2026-11-13-mcp-policy/mcp-policy-domain-second.log). Testy obejmują puste legacy bytes, dokładne piny, limity/brak prywatnych danych i odmowę v3/v4.
- **113 MCP +6 HTTP +31 vault tests i all-targets Clippy trzech pakietów PASS**, peak1224871936B/OOM0; [log](evidence/2026-11-13-mcp-policy/mcp-policy-client-first.log), source SHA2562795871405fe99ae8f93e6bafaa2d8b5f8948400d2bf5eab0eb09042478bf509. **138 frontend tests PASS**: [log](evidence/2026-11-13-mcp-policy/mcp-policy-frontend-first.log). Typecheck exit0: [log](evidence/2026-11-13-mcp-policy/mcp-policy-typecheck.log).
- Pierwszy Clippy serwera ujawnił moje błędne dodanie mcp do fixture TurnSpec zamiast wyłącznie AgentDefinition; usunięto bez suppressions: [log](evidence/2026-11-13-mcp-policy/mcp-policy-clippy-first.log). Końcowy **all-targets Clippy -D warnings serwera/powiązanych bibliotek PASS**, peak2093117440B/OOM0; [log](evidence/2026-11-13-mcp-policy/mcp-policy-clippy-second.log).
- **584 relay +1 identity +74 session-runtime +27 terminal +37 usage PASS**, exit0,32 osobne acceptance jawnie pominięte, peak3499057152B/OOM0; [log](evidence/2026-11-13-mcp-policy/mcp-policy-server.log). Cztery nowe cases sprawdzają niezmienność starej publikacji po dodaniu MCP oraz brak lokalnego procesu/zdalnego admission bez brokera. Serwer i Clippy mają wspólny source SHA256818eaf1ffec5c479175ab7b8f0b3a6b4658874a61048d898d70034381bfe28fb. Kod klienta113 jest identyczny; odwrócenie wyłącznie wczesnych odmów zdalnego controllera/ich testów i naprawy TurnSpec odtwarza cały wcześniejszy snapshot: [dowód](evidence/2026-11-13-mcp-policy/snapshot-difference.json).
- Browser używa teraz RAM3584MiB tak jak server, ponieważ kompiluje ten sam duży relay test binary; pozostałe granice PID/readonly/network/LSM/tmpfs pozostają bez zmian. [18 testów launchera](evidence/2026-11-13-mcp-policy/mcp-policy-launcher.log) oraz [14 CI/tooling](evidence/2026-11-13-mcp-policy/mcp-policy-ci-tooling.log) PASS. Realne konta, keyring, tailnet i publikacje nie były używane.
- **Produkcyjny Builder/creation Chromium PASS**, exit0, peak3638587392B/OOM0,5,46s; [log](evidence/2026-11-13-mcp-policy/mcp-policy-builder-browser.log). Zachowano cały lost202/retry/AHP/pojedynczy proces/przypięte instrukcje i skill scenariusz. Nowy fragment zapisuje i publikuje rzeczywistą schema2/MCP policy, odczytuje ją z API i wymaga disabled opcji z jawnym MCP unsupported. Układ1440/390px bez overflow: [desktop](evidence/2026-11-13-mcp-policy/agent-mcp-desktop.png), [mobile](evidence/2026-11-13-mcp-policy/agent-mcp-mobile.png), obejrzano oba zrzuty.
- **Produkcyjne MCP OAuth/catalog Chromium PASS**, exit0, peak3642368000B/OOM0,5,34s; [log](evidence/2026-11-13-mcp-policy/mcp-policy-oauth-browser.log). Rzeczywisty katalog pokazuje pin, który test parsuje z DOM i porównuje z niezależnym goldenem. Cały wcześniejszy scenariusz OAuth/PKCE/two sites/lost responses/unmount/private tokens nadal PASS, bez tools/call. Układ przy390px bez overflow; obejrzane [desktop](evidence/2026-11-13-mcp-policy/mcp-pins-desktop.png) i [mobile](evidence/2026-11-13-mcp-policy/mcp-pins-mobile.png). Oba browser suites wykonują produkcyjny Vite build.
- Końcowy serwer, Clippy i oba browser suites mają source SHA256818eaf1ffec5c479175ab7b8f0b3a6b4658874a61048d898d70034381bfe28fb. Kontrakt: [profile](agent-profiles.md), [piny MCP](mcp-client.md#przypięcia-definicji-narzędzi-i-allowlista-profilu). E5.2/E5.3 pozostają otwarte: następnym przekrojem są prywatne połączenia przy admission, runtime OAuth journal/guard, broker tools/call i proxy runnera z jednorazowymi zgodami. Formularze MRTR i trwałe revoke/GC również pozostają otwarte. Formalnie20/68; cel E0–E10 aktywny.
- Końcowa kontrola wykryła nieescapowany literalny myślnik we wzorcu HTML provider ID. Poprawiono pattern i dodano rzeczywiste Chromium checkValidity dla work-id (true) i bad? (false), bez zmiany serwerowej polityki. **Końcowy Builder Chromium PASS**, exit0,5,26s, peak3607269376B/OOM0; [log](evidence/2026-11-13-mcp-policy/mcp-policy-builder-browser-verified.log). Source SHA256b4ccf30653cd8d02edd8a9596c090fd7b94d3ba09d3730ac07d80b0c69446d45. Obejrzano końcowe [desktop](evidence/2026-11-13-mcp-policy/agent-mcp-final-desktop.png) i [mobile](evidence/2026-11-13-mcp-policy/agent-mcp-final-mobile.png).
- Po wcześniejszym wspólnym snapshot818eaf zmieniły się wyłącznie pattern w AgentBuilder.tsx i jego negatywne testy w ahp-create.mjs. Kod Rust i panel OAuth/katalogu są identyczne z odebranymi server/Clippy/OAuth-browser; odwrócenie dokładnie tych dwóch zmian odtwarza cały poprzedni hash: [dowód](evidence/2026-11-13-mcp-policy/pattern-snapshot-difference.json). Nie powtarzano niezmienionych suites.

## 2026-11-13 — E5.3: lokalny broker MCP, prywatne konta i zgoda przed wykonaniem

- Lokalny admission przypina dokładnie jedno własne connected połączenie/revision dla każdego providera opublikowanego profilu. Brak, nadmiar, obcy provider lub nieaktualna rewizja blokują start. Prywatne powiązania i digest konfiguracji wchodzą w fingerprint zamiaru; nie są częścią współdzielonej definicji. Legacy bez MCP zachowuje wcześniejsze bajty. Zdalne MCP nadal jest jawnie odmawiane w controller/worker/wire.
- Actor uruchamia prywatny broker Unix i stdio facade --mcp-tools. Sandbox otrzymuje dokładny socket i executable. Harness otrzymuje wyłącznie katalog allowlisty oraz losowy dowód sesji, bez OAuth/keyring/DB. Broker sprawdza piny przed procesem, schemat argumentów przed siecią i katalog przed/po jednorazowej zgodzie. Runtime OAuth journal i guard kontrolują urządzenia/OIDC, logical/native grants/binding/checkout, lease, konto/revision i generację tokena. Definite wynik jest utrwalany hostowym SDK; unknown lub błąd trwałego zapisu przerywa actor. Nie ma automatycznego retry.
- Facade nadaje nowy UUID każdej operacji, także przy ponownym użyciu JSON-RPC id. Broker deduplikuje powtórzoną ramkę o tym samym UUID/payload bez drugiego HTTP; zmiana payloadu jest konfliktem. Odpowiedź jest ponownie autoryzowana po serializacji. Dodano ograniczenia pamięci/ramek/receipts i sloty brokerów trzymane do rzeczywistego zakończenia wątku.
- Formularz sesji wymaga jawnego wyboru prywatnego konta dla każdego providera; nie wybiera automatycznie nawet jedynego konta. GET listy nie odświeża OAuth. Intent zamraża konta i rewizje także przy utracie odpowiedzi. Capability API odblokowuje MCP wyłącznie dla lokalnego Unix/opt-in z providerami. Parametry zgody MCP są domyślnie rozwinięte, tytuł wskazuje dostawcę i narzędzie.
- **Pierwszy rzeczywisty broker fixture PASS**, 1 test obejmujący zgodę/odmowę, zero tools/call przed decyzją, jedno po zgodzie, cache identycznej ramki i konflikt zmienionych argumentów: [log](evidence/2026-11-13-mcp-runtime/mcp-runtime-broker-first.log). Source28b65e4ab0796609e01d573e92b64d999490edbd5a8f994cda66cfcd70798a24, peak3546923008B/OOM0. To wcześniejszy snapshot, przed rozszerzeniem o disconnect/unknown i produkcyjny facade.
- **Pierwszy produkcyjny Chromium PASS**, 7,26s, peak3669696512B/OOM0: [log](evidence/2026-11-13-mcp-runtime/mcp-runtime-browser.log). Sourcee572c3135abe1d5fd6eb6a8c134bafcd30e070a184f991bf610689d59f304e6b. Zachowano rzeczywiste OAuth/PKCE/two sites/lost responses i dodano Builder→publikacja→jawne prywatne konto→AHP approve-once→broker tools/call→wynik. Provider odnotował dokładnie1 tools/call i4 katalogi. FakeACP używa w tym scenariuszu bezpośredniej ramki Unix; nie dowodzi to skompilowanego facade. Układ1440/390 bez overflow; obejrzano [desktop](evidence/2026-11-13-mcp-runtime/mcp-runtime-desktop.png) i [mobile](evidence/2026-11-13-mcp-runtime/mcp-runtime-mobile.png). Po tym przebiegu poprawiono tytuł/tekst zgody i domyślne rozwinięcie parametrów; końcowe odbiory po tych zmianach są opisane poniżej.
- **140 testów frontendu PASS** na hostowym Node22.17.1: [log](evidence/2026-11-13-mcp-runtime/mcp-runtime-frontend-all.log). Obejmują przypięcie kont przy uncertain retry i odrzucanie duplikatów/providerów/rewizji/zdalnych wyborów. **Kontenerowy TypeScript PASS**: [log](evidence/2026-11-13-mcp-runtime/mcp-runtime-typecheck-fixed.log). **18 launcher +14 CI tooling PASS**: [launcher](evidence/2026-11-13-mcp-runtime/mcp-runtime-launcher.log), [CI](evidence/2026-11-13-mcp-runtime/mcp-runtime-ci-tooling.log).
- **Produkcyjny executable + rzeczywisty bwrap/LSM + stdio proxy PASS**, 1,82s po build2m38s i test build52,65s: [log](evidence/2026-11-13-mcp-runtime/mcp-runtime-sandbox.log). Sourceab7b914cb9abe00071aacff86eca45e5bd4e9bf03ba011465d7eb02d64f25d42, peak3689709568B/OOM0. Cztery warianty: zgoda→jedno tools/call, odmowa→zero, disconnect po zgodzie→zero/stara sesja interrupted, utrata odpowiedzi po dispatch→jedno/trwałe unconfirmed/unknown bez retry. Rzeczywisty facade initialize/tools/list/tools/call używa legalnie ponownie JSON-RPC id1. Harness jest kontrolowanym Python ACP fixture w sandboxie; nie użyto realnego modelu ani konta. Jest to focused runtime acceptance, nie pełny zestaw pozostałych kernel/remote acceptance.
- **Końcowe all-targets Clippy -D warnings PASS**, exit0, peak2252632064B/OOM0, ten sam sourceab7b914c co sandbox: [log](evidence/2026-11-13-mcp-runtime/mcp-runtime-clippy-final.log). **Kontenerowe140 frontend tests PASS**: [log](evidence/2026-11-13-mcp-runtime/mcp-runtime-frontend-container.log).
- **Końcowy pełny server suite587 relay +1 identity +74 session-runtime +27 terminal +37 usage PASS**, exit0,33 jawnie pominięte osobne acceptance, peak3545083904B/OOM0, scratch1330092KiB/1572864KiB: [log](evidence/2026-11-13-mcp-runtime/mcp-runtime-server-final.log). Ten sam sourceab7b914c co Clippy i sandbox. Zawiera wszystkie cztery warianty brokera oraz dwa testy facade: legalne ponowne JSON-RPC id, brak retry po utracie odpowiedzi, odrzucenie przekroczonych/truncated ramek i wejścia stdio.
- Pierwszy production build wykrył tempfile zadeklarowane tylko jako dev-dependency: [log błędu](evidence/2026-11-13-mcp-runtime/mcp-runtime-production-build-initial-failure.log). Przeniesiono istniejącą zależność do dependencies; powyższy produkcyjny build i test po poprawce PASS. Runtime otrzymał ten sam limit3584MiB co server/browser, ponieważ wcześniejszy zmierzony peak przekraczał3328MiB; scratch/izolacja/granty bez zmian. Ponowny browser odbiór po poprawkach także PASS, jak poniżej.
- **Chromium na wspólnym sourceab7b914c PASS**, 7,00s, peak3647180800B/OOM0: [log](evidence/2026-11-13-mcp-runtime/mcp-runtime-browser-final.log). Nazwa dostawcy/narzędzia widoczna, parametry domyślnie rozwinięte; desktop/mobile obejrzane. Runtime, Clippy, pełny server i ten browser mają identyczne źródła: [manifest](evidence/2026-11-13-mcp-runtime/verified-source.json).
- Po przeglądzie renderu uproszczono parametry do samego arguments, bez technicznych connection/revision/digest; pełne przypięcie nadal jest w AHP i nie jest modyfikowane przez formatowanie. **Końcowy Chromium sourcefb00a745d8b2ef9fb43ddf62152f67694b911b42c3072b084c6baa25847ba6bc PASS**, 6,59s, peak3642871808B/OOM0: [log](evidence/2026-11-13-mcp-runtime/mcp-runtime-browser-ui-verified.log). Test wymaga widocznych dokładnych argumentów przed approve-once; rzeczywisty tools/call pozostaje pojedynczy. Układ1440/390 bez overflow; obejrzano końcowe [desktop](evidence/2026-11-13-mcp-runtime/mcp-runtime-verified-desktop.png) i [mobile](evidence/2026-11-13-mcp-runtime/mcp-runtime-verified-mobile.png). Produkcyjny build obejmuje TypeScript. **Końcowe141 frontend tests w kontenerze PASS**: [log](evidence/2026-11-13-mcp-runtime/mcp-runtime-frontend-ui-container.log).
- Różnica końcowego sourcefb00 wobec odebranego ab7 obejmuje dokładnie cztery pliki frontendu: renderer parametrów, helper tekstu, test jednostkowy i asercję browsera. Odwrócenie dokładnie tych zmian w prywatnym snapshotcie odtwarza cały hash ab7: [dowód](evidence/2026-11-13-mcp-runtime/ui-parameters-source-proof.json), [dokładny patch](evidence/2026-11-13-mcp-runtime/ui-parameters-only.patch). Rust, manifesty i fixtures są identyczne z pełnym server/Clippy/produkcyjnym sandboxem; nie powtarzano niezmienionych suites.
- [Kontrakt brokera](mcp-client.md#lokalny-broker-wykonania-mcp). E5.2/E5.3 nadal otwarte: zdalne proxy, MRTR UI, trwałe provider revoke/GC, ogólne scoped secrets i pozostałe odbiory. Nie użyto rzeczywistych kont/usług/tailnetu. Formalnie20/68; cel E0–E10 pozostaje aktywny.
- Kontrola122 Markdown/local links/anchors i git diff --check PASS; [log](evidence/2026-11-13-mcp-policy/mcp-policy-docs.log).

## 2026-11-13 — E5.3: wspólny rdzeń brokera i kontrakt zdalnego MCP v5

- Oddzielono hostowe Authority od lokalnego procesu. Wspólny Broker przechowuje własną kopię session/workspace/owner/Journal lease epoch+instance/starter device/binding revision/profile digest. Rdzeń sam odmawia zmiany każdego pinu, przekroczenia deadline i użycia prywatnego połączenia innego owner/workspace. Adapter ma osobne świeże kontrole poza SQL i wewnątrz transakcji OAuth; lokalny adapter zachowuje dotychczasowe native checkout/device/OIDC/policy cuts. Wspólna weryfikacja urządzenia zatwierdzającego jest używana także przez ACP, bez kopiowania zasad.
- Naprawiono pojemność preflight: slot należy do rzeczywistego Broker, także gdy await katalogu zostanie anulowany. Wcześniej przerwany waiter mógł zwolnić slot, gdy spawn_blocking jeszcze pracował. Nowy deterministyczny test zatrzymuje taki worker, anuluje waiter, wymaga nadal7/8 wolnych slotów i odzyskania ósmego dopiero po rzeczywistym zakończeniu. Pozostałe nowe testy sprawdzają8 zmienianych pinów, deadline/stop przed callbackami oraz obce owner/workspace przed SQL/HTTP/keyring.
- Dodano proponowany kontrakt session/broker dla dunlin.worker.v5. Tools/list jest przypięte do SessionFence, tools/call do TurnFence, w tym obu boots/channel/session/turn epochs. Referencja requestu SHA256 wiąże także request_id, alias i dokładny JSON argumentów; golden wyliczono niezależnie Python hashlib. Nie ma pola OAuth ani prywatnego connection ID. Argumenty mają8KiB/1024 węzły/depth32. Odpowiedzi do256KiB są dzielone na4KiB UTF-8 fragmenty pod limitem64KiB ramki; assembler odmawia obcych refs, reorder/duplicate, zmienionych headers i błędnego digestu, trwale zamykając daną operację. Niepełny wynik nie jest dostarczany.
- To kontrakt nieaktywny: nie zmieniono v4 codecs/Hello/admission ani odmowy profili MCP. Nowe testy wymagają odrzucenia tych ramek przez v4. Nie powstała jeszcze negocjacja/producer v5, trwałe prywatne powiązania remote, adapter remote Authority, socket bridge runnera i kompletny zdalny tools/call. Dokładne granice i kolejność: [plan v5](remote-session-plan.md#rozszerzenie-mcp-osobna-wersja-v5-w-realizacji).
- **Pełny server591 relay +1 identity +74 session-runtime +27 terminal +37 usage PASS**, exit0,33 osobne acceptance pominięte, peak3515256832B/OOM0: [log](evidence/2026-11-13-mcp-v5/mcp-authority-server-first.log). Source5a901524dd20bdd3cc46a0a91351a43a9b3c56128e1b2bdfac37b3d67574ffd2. Obejmuje cztery nowe testy authority/capacity i dotychczasowy rzeczywisty broker z grant/deny/disconnect/lost response.
- **50 domain +41 protocol tests PASS**, w tym6 nowych testów v5, peak582905856B/OOM0: [log](evidence/2026-11-13-mcp-v5/mcp-v5-protocol-first.log). **Końcowe all-targets Clippy -D warnings PASS**, peak2165653504B/OOM0: [log](evidence/2026-11-13-mcp-v5/mcp-authority-clippy-final.log). Wspólny source86564920ad4da44990e9a58607240deab6537d52e1522efcf86c3725d96cced4. Odwrócenie wyłącznie dwóch nowych nieużywanych modułów kontraktu v5 i ich deklaracji odtwarza cały source pełnego server suite; kod serwera/brokera/SDK/frontend jest identyczny: [dowód](evidence/2026-11-13-mcp-v5/source-proof.json).
- **Produkcyjny executable + realny bwrap/LSM + stdio broker PASS**, source86564920,1,81s po build, peak3659841536B/OOM0: [log](evidence/2026-11-13-mcp-v5/mcp-authority-sandbox-final.log). Te same cztery warianty zgody/odmowy/odłączenia/utraty odpowiedzi potwierdzają zachowanie lokalnego proxy po refaktorze. To focused runtime acceptance z kontrolowanym ACP fixture, bez rzeczywistego modelu/konta/tailnetu; nie jest odbiorem zdalnego MCP.
- **Końcowy produkcyjny Chromium PASS**,7,17s, peak3720654848B/OOM0: [log](evidence/2026-11-13-mcp-v5/mcp-authority-browser-final.log). Rzeczywisty OAuth/Builder/prywatne konto/AHP approve-once/tools/call nadal działa przez wspólny rdzeń; układ1440/390 bez overflow. Obejrzano [desktop](evidence/2026-11-13-mcp-v5/mcp-runtime-desktop.png) i [mobile](evidence/2026-11-13-mcp-v5/mcp-runtime-mobile.png). [Manifest wspólnego source86564920](evidence/2026-11-13-mcp-v5/verified-source.json). E5.2/E5.3 oraz cały cel E0–E10 pozostają otwarte; formalnie20/68. Nie użyto rzeczywistych usług ani kont.

## 2026-11-13 — E5.3: Hello/open v5, własna polityka operatora i dispatcher runnera

- Dodano osobne, rygorystyczne kodeki Hello5/oferty limitów i OpenBrokerSession. Nowy fingerprint obejmuje pełną definicję, tożsamości kanału/sesji, budżety i wybrane limity. Niezależny wzorzec Python SHA256 to7dbc95a9788cfff5a334f54f644b33e508a3cbc545b7367ba2f1cab97c6272a0: [preimage i wynik](evidence/2026-11-13-mcp-v5-open/open-codec-golden.json). Stare dekodery i v4 admission nadal odmawiają MCP. **50 domain +44 protocol PASS**, peak596992000B/OOM0: [log](evidence/2026-11-13-mcp-v5-open/mcp-v5-open-domain-final.log).
- Native ledger workera ma pasywne open_broker_session: wymaga opaque kontekstu z ofertą brokera, osobnej polityki operatora schema2 z mcp_broker:true oraz własnego bieżącego Read+Execute. Rewizje, digest profilu/polityki, limity i oba boots pozostają przypięte. Stare piny pomijają nowe opcjonalne pole, zachowując legacy payload/hash. Testy obejmują odmowę bez oferty/polityki, ograniczenie oferty, rollback SQL bez zużycia epoch, retry po reopen bez nowego lease, zmianę retry i próbę downgrade, revoke/regrant ABA oraz legacy sesję na broker-capable kontekście. **Pełny server596 relay +1 identity +74 SDK +27 terminal +37 usage PASS**,33 osobne acceptance pominięte, peak3537727488B/OOM0: [log](evidence/2026-11-13-mcp-v5-open/mcp-v5-open-server-first.log). Dla tego samego source68644d36 również **all-targets Clippy -D warnings PASS**: [log](evidence/2026-11-13-mcp-v5-open/mcp-v5-open-clippy-first.log).
- Przygotowano osobny dispatcher przyszłego socket owner v5: wspólna pojemność kolejki i wysłanych requestów do8, świeży UUID na operację, pełna korelacja/ReplyAssembly, kontrola wynegocjowanego rozmiaru przed rezerwacją pamięci, niezależny timer i sprawdzenie kanału przed wysłaniem oraz użyciem odpowiedzi. Wysłana operacja zachowuje slot po anulowaniu waitera. Oczekiwanie ma limit150s; runtime nadal musi ograniczyć je budżetem tury i sprawdzić własne native leases. Wygaśnięcie wysłanego requestu zamyka parę i zwraca OutcomeUnknown, bez ponowienia. Błąd dispatchera musi zatrzymać socket/runtime; sam dispatcher nie nadaje authority.
- **Pełny server605 relay +1 identity +74 SDK +27 terminal +37 usage =744 PASS**,33 osobne acceptance pominięte,138,02s głównej regresji, peak3551453184B/OOM0: [log](evidence/2026-11-13-mcp-v5-open/mcp-v5-open-dispatch-server-final.log). Wszystkie9 nowych testów dispatchera PASS: interleaving dużych fragmentów, canceled waiter przed/po wysłaniu, obcy boot, zmiana scope/reorder, rozmiar negocjowany, unknown bez retry po deadline/drop, aktualny guard przed wysłaniem/odpowiedzią/odebraniem i niezależny timer. Source74fe43a0; końcowa różnica dotyczy wyłącznie parsera null i jego asercji: [dowód zakresu źródeł](evidence/2026-11-13-mcp-v5-open/source-scope-proof.json), [odtwarzalny skrypt](evidence/2026-11-13-mcp-v5-open/verify-source-scope.py).
- Przegląd parsera wykrył, że serde Option przyjmował jawne mcp_broker:null jako brak pola w schema1. Dodano deserializację wyłącznie bool, odróżniając brak od null; oba warianty null są odrzucane. Schema1 dopuszcza wyłącznie brak pola, schema2 wymaga true. [Dokładny patch](evidence/2026-11-13-mcp-v5-open/explicit-null-policy.patch). Pierwszy Clippy nowego dispatchera wykrył nieużywany timer w snapshotcie sprzed dopisania testu zegara; [log diagnostyczny](evidence/2026-11-13-mcp-v5-open/mcp-v5-open-dispatch-clippy.log).
- **Końcowe all-targets Clippy -D warnings PASS**, sourcecb5c79b0,1m51s, peak2187988992B/OOM0: [log](evidence/2026-11-13-mcp-v5-open/mcp-v5-open-dispatch-clippy-final.log). Obejmuje także końcowy parser i wszystkie test targets; nie zastępuje wykonania asercji.
- **Końcowe56 worker_exports tests PASS** na tym samym sourcecb5c79b0,1,77s po build, peak3550785536B/OOM0: [log](evidence/2026-11-13-mcp-v5-open/mcp-v5-open-exports-final.log). Obejmuje faktyczne wykonanie asercji null dla obu wersji polityki, cały katalog/granty/revisions/ledger i nowe sesje brokera. [Manifest źródeł i logów](evidence/2026-11-13-mcp-v5-open/verified-source.json) wskazuje dokładne zakresy; odtworzenie wcześniejszych snapshotów odbywa się wyłącznie w prywatnej kopii. Bez powtarzania niezmienionego UI/browser/runtime acceptance. Kontrola122 Markdown/linków/anchors, format Rust i git diff --check PASS.
- Aktualny WSS nie negocjuje v5 i nie produkuje brokerowego ExecutionChannel. Remote profil MCP pozostaje zamknięty. Do aktywacji potrzebne są faktyczne opt-in/negocjacja, controller admission z prywatnym kontem, remote Authority, wpięcie dispatchera do WSS i lokalnego proxy oraz pełny zdalny odbiór. [Dokładna granica](remote-session-plan.md#rozszerzenie-mcp-osobna-wersja-v5-w-realizacji). E5.2/E5.3 i cały cel E0–E10 pozostają otwarte; formalnie20/68. Nie użyto rzeczywistych kont/usług/tailnetu.

## 2026-11-13 — E5.3: aktywna negocjacja transportu v5

- Podłączono odrębne Hello5 do rzeczywistego endpointu WSS i klienta workera. Controller wymaga execution_enabled/persistent_sessions/mcp_broker oraz żywego producenta sesji; worker wymaga enabled/persistent_sessions/mcp_broker i rzeczywistego Runtime wspierającego trwałe sesje. Nowa flaga jest domyślnie false po obu stronach. Wybrana wersja nie ma fallbacku po odmowie lub braku runtime. Stary config pozostaje v2/v3/v4, a istniejące fixtures jawnie zachowują mcp_broker:false.
- Hello5 ma osobny decoder, bez osłabiania root WorkerMessage/Hello v1–v4. Hello5 na kanale4 i Hello4 na kanale5 są odrzucane przed nonce proof i lease. Nadal wymagany jest pełny prywatny reverse proof; poprawny Hello5 bez niego nie tworzy lease. Po Accepted oferta limitów należy do konkretnego opaque SessionTransport z tym channel i oboma boots. Wspólne control/event ramki nadal obsługują profile bez MCP z dotychczasowym fingerprintem. Rate v5 wynosi4096/min jak v4, nieznana wersja6 zachowuje limit120.
- Transport nie nadaje MCP authority. Bez przypisanego hostowego brokera poprawne tools/list i tools/call są odrzucane jako Denied, bez wyboru konta, discovery, OAuth lub HTTP. Obcy channel/boot jest błędem protokołu. Nie podłączono jeszcze nowego OpenBrokerSession do runtime, prywatnych powiązań kontrolera, remote Authority ani dispatchera do lokalnego procesu. Osobna polityka exportu schema2/mcp_broker:true nadal nie jest zastępowana flagą kanału. [Granica i pozostały zakres](remote-session-plan.md#rozszerzenie-mcp-osobna-wersja-v5-w-realizacji).
- Monitor hosta przyjmuje connection.protocol_version5 i pokazuje etykietę trwałej sesji, zachowując observational:true i execution_ready:false. Nieznana6 jest odrzucana. **Końcowe141 frontend tests PASS**: [log](evidence/2026-11-13-mcp-v5-wire/mcp-v5-wire-frontend-final.log).
- **Pełny server609 relay +1 identity +74 SDK +27 terminal +37 usage =748 PASS**,34 osobne acceptance pominięte,139,09s głównej regresji, peak3551424512B/OOM0: [log](evidence/2026-11-13-mcp-v5-wire/mcp-v5-wire-server-final.log). Cztery nowe testy obejmują wszystkie brakujące kombinacje trzech flag i producenta, Hello4/5 mismatch, Hello5 bez reverse proof i brak obniżenia wersji bez runtime. Source2901151b, sprzed dodania osobnego ręcznego probe testu i asercji limitów; produkcyjny kod Rust różni się wyłącznie trzema komentarzami. [Dokładne różnice i odtwarzalny dowód](evidence/2026-11-13-mcp-v5-wire/source-scope-proof.json).
- **Realny produkcyjny executable + TLS/WSS/reverse proof + bwrap/LSM:5 zdalnych acceptance PASS**,54,99s po build: [log](evidence/2026-11-13-mcp-v5-wire/mcp-v5-wire-runtime-final.log). Obejmuje remote_execution_vertical v3, remote_session_process_restart, nowy remote_broker_transport v5, remote_session_journal oraz dotychczasowy remote_session_transport v4. Nowa wersja utrzymuje rzeczywistą sesję ACP, kolejne tury, literalne instrukcje/skill, odwołanie native grantu i zatrzymanie sandboxa. To transport sesji z profilem bez MCP, nie odbiór zdalnego tools/call. Source2359ca56; różnica końcowa to wyłącznie etykieta5 w frontendzie. Peak3758096384B równa się cap3584MiB, memory.events.max=41, OOM0; nie zwiększano zasobów. Użyto kontrolowanego lokalnego ACP i prywatnych daemon/CA fixtures; bez modelu, kont i rzeczywistego tailnetu.
- **Ręczny v5 peer przez rzeczywisty prywatny TLS/WSS/reverse proof PASS**,0,06s po build, sourcef9306722, peak3681312768B/OOM0: [log](evidence/2026-11-13-mcp-v5-wire/mcp-v5-wire-unbound-final.log). Klient bez native runtime/sesji przechodzi handshake, a następnie tools/list i tools/call pod sfabrykowanymi sesją/turą dostają dokładnie skorelowane Denied. Oferta2/65536 jest przypięta do rzeczywistego transportu. Nie publikuje się zdarzeń wykonania; ACK1/2/3 pozostają poprawne. Żądanie z obcym worker boot kończy kanał InvalidMessage. To kontrola, że zaufanie do transportu nie zastępuje własnego grantu i prywatnego admission MCP.
- **Końcowe all-targets Clippy -D warnings PASS**, sourcef9306722,1m54s, peak2172112896B/OOM0: [log](evidence/2026-11-13-mcp-v5-wire/mcp-v5-wire-clippy-final.log). **50 domain +44 protocol PASS** na tym samym snapshotcie, peak563257344B/OOM0: [log](evidence/2026-11-13-mcp-v5-wire/mcp-v5-wire-domain-final.log). **TypeScript --noEmit PASS** w ograniczonym kontenerze, także etykieta nowej wersji monitora: [log](evidence/2026-11-13-mcp-v5-wire/mcp-v5-wire-typecheck-final.log).
- [Manifest źródeł i logów](evidence/2026-11-13-mcp-v5-wire/verified-source.json) wiąże końcowy sourcef93067224e098288494f85f54eb8425bf6a25eb42be5cd45064691109122faef z właściwym zakresem każdego przebiegu. [Dowód](evidence/2026-11-13-mcp-v5-wire/source-scope-proof.json) i [skrypt](evidence/2026-11-13-mcp-v5-wire/verify-source-scope.py) odtwarzają wcześniejsze source w prywatnej kopii: starszy pełny server różni się wyłącznie nowymi testami/komentarzami i jedną etykietą UI; runtime różni się wyłącznie etykietą. Format Rust, git diff --check i kontrola122 Markdown/local links/anchors PASS. E5.2/E5.3 i cel E0–E10 pozostają otwarte; formalnie20/68. Nie użyto rzeczywistych kont, usług ani tailnetu.

## 2026-11-13 — E5.3: wspólne prywatne piny kont i odczyt zapisu

- Wydzielono binder do interactive/mcp/bindings. Lokalny admission korzysta z BoundSession::capture na całym zwalidowanym profilu, po czym przekazuje brokerowi dotychczasowe BoundConnection. Dotychczasowe identity JSON provider_id/connection_id/revision/configuration i wkład w fingerprint pozostają bez nowych pól. BoundSession nie implementuje Deserialize, Clone ani Serialize; jest wynikiem świeżego sprawdzenia, a prywatny snapshot stanowi wyłącznie dane.
- Prywatny zapis schema1 ma limit8KiB, owner/workspace/profile digest oraz do8 posortowanych pinów kont i konfiguracji operatora. Nie zawiera endpointu, issuera, client ID, tokenów, OAuth bindingu ani referencji vault. Restore sprawdza rozmiar/schema/kształt i dopiero ponownie odczytuje własne connected konto z bieżącą rewizją/providerem, sprawdza profil i digest konfiguracji. Nie rekonstruuje Journal lease, native grantu lub jednorazowej zgody. Capture/restore służą admission; aktywna rotacja nadal używa istniejącego credential journal i walidacji BoundConnection, która nie odcina własnego refreshing.
- Cztery nowe testy obejmują roundtrip przez osobną fixture SQLite i ponowne otwarcie OAuth Store, zachowanie starego identity JSON, brak sekretów w zapisie, zmianę owner/workspace/profile/provider config i próbę podmiany owner w zapisanych danych. Revoke→reauthorize nie przywraca starej rewizji; niepewny refresh blokuje odtworzenie. Odrzucane są obce wersje, zmienione digests/revisions, duplicate provider, dodatkowe pola tokenów i oversized/malformed JSON. Fixture przechowania nie stanowi integracji produkcyjnego remote Pins.
- **Pełny server614 relay +1 identity +74 SDK +27 terminal +37 usage =753 PASS**,34 osobne acceptance pominięte,139,78s głównej regresji, peak3596058624B/OOM0: [log](evidence/2026-11-13-mcp-binding-snapshot/mcp-binding-snapshot-server.log), source675ee7d086f5bd8978227b030a6879869ed79b6ccfc0cf28568a4c6383f72dc7. Obejmuje cztery nowe testy i dotychczasowy lokalny flow prywatnych połączeń/zgód. Pierwszy Clippy wykrył dwa needless borrow w nowych testach; poprawiono je przed tym snapshotem: [log diagnostyczny](evidence/2026-11-13-mcp-binding-snapshot/mcp-binding-snapshot-clippy.log).
- Produkcyjny odczyt/zapis tego formatu w zdalnym controller Pins, wersjonowany open/admission, remote Authority i podłączenie proxy pozostają otwarte. Publiczny Remote MCP nadal nie jest odblokowany. To fundament prywatnego admission, nie odbiór całego E5.3 ani scoped secrets. Cel E0–E10 pozostaje aktywny; formalnie20/68. Nie użyto rzeczywistych kont/usług/tailnetu.
- **Produkcja + realny bwrap/LSM + stdio MCP broker PASS**,1,75s po build, ten sam source675ee7d0: [log](evidence/2026-11-13-mcp-binding-snapshot/mcp-binding-snapshot-runtime.log). Cztery warianty approve/deny/disconnect/lost response zachowują dokładne1/0/0/1 zewnętrznych fixture tools/call i brak replay. Peak3758096384B równa się cap3584MiB, memory.events.max=61, OOM0. To odbiór lokalnego proxy po zmianie bindera, bez rzeczywistego konta/modelu/tailnetu; nie jest to odbiór remote MCP ani trwałego zdalnego admission.
- **Końcowe all-targets Clippy -D warnings PASS**,1m51s, peak2270818304B/OOM0: [log](evidence/2026-11-13-mcp-binding-snapshot/mcp-binding-snapshot-clippy-final.log). Pełny server, produkcyjny runtime i Clippy mają identyczny source675ee7d0: [manifest](evidence/2026-11-13-mcp-binding-snapshot/verified-source.json). Nie zmieniano UI ani protokołu, nie powtarzano ich osobnych suites. Format Rust, git diff --check i kontrola122 Markdown/linków/anchors PASS.

## 2026-11-13 — E5.3: prywatne piny w controller SQL i wersjonowane open

- Controller remote Pins ma opcjonalny mcp z prywatnym snapshotem kont i limitami. Legacy skip-none zachowuje stare bajty JSON. Nowe RemoteOpen jest jawnym wariantem zapisu: płaski v4 albo odrębny OpenBrokerSession v5. Walidacja fingerprintu i receipt jest wybierana według wariantu, bez przepuszczania MCP przez v4. Wykorzystano dotychczasowe bounded JSON columns; nie zwiększano user_version ani limitów storage.
- Osobne prepare_remote_mcp_session_in_transaction wymaga opublikowanego profilu MCP, aktywnego device/projektu, connected własnego konta i konfiguracji operatora oraz opaque SessionTransport z ofertą5. Trzy flagi kontrolera i node ID muszą odpowiadać kandydatowi; transport attests bieżący kanał/lease/producenta wewnątrz transakcji. Revalidation przed commit ponownie odczytuje konto i source slot. Intent v5 obejmuje prywatny snapshot i limity; komenda workera ich prywatnej części nie zawiera.
- Retry po reopen z nadal żywym transportem zwraca tę samą sesję bez nowego Journal lease i command. Osobny publiczny odczyt receipt bez żywego transportu nie jest jeszcze podłączony. MCP current/dispatch i stary Actor jawnie odmawiają obsługi bez scoped broker authority. Publiczny Remote MCP endpoint nie został odblokowany. Recovery waliduje tylko kształt/scope zapisu: może zatrzymać zamiar bez send cut także po odłączeniu OAuth, bez ponawiania.
- Pięć nowych testów sprawdza rozdzielenie prywatnego connection ID od wire command, wrapper5 i odmowę v4, zgodność bajtów JSON/hash legacy oraz brak nowego pola w starych pinach. Ponadto: retry/reopen bez drugiego lease/wiersza, revoke konta lub drop source producenta między prepare i commit, rollback całego Journalu po SQL insert failure i późniejsze poprawne ponowienie, recovery bez dispatch po disconnect. To SQL/opaque-slot fixtures, bez realnego WSS czy native wykonania MCP. Kontrolowany helper rejestracji jest dostępny wyłącznie cfg(test).
- Wstępne **18 targeted session-store tests PASS**, w tym cztery nowe przed dodaniem końcowej asercji legacy/recovery: [log](evidence/2026-11-13-mcp-controller-pins/mcp-controller-pins-focused.log). **Końcowy pełny server619 relay +1 identity +74 SDK +27 terminal +37 usage =758 PASS**,34 osobne acceptance pominięte,138,93s głównej regresji, peak3613949952B/OOM0: [log](evidence/2026-11-13-mcp-controller-pins/mcp-controller-pins-server-final.log), source94bce480928fa3396b841ff746617b9b64b04c458fcbeef4c6992fc48aeff302. Pierwszy Clippy wykrył częściowe przeniesienie testowego guarda producenta; zmieniono go na Option/take przed przebiegiem targeted: [log diagnostyczny](evidence/2026-11-13-mcp-controller-pins/mcp-controller-pins-clippy-first.log).
- Do aktywnego przekroju pozostają: REST/Actor z prywatnymi pinami i scoped authority, typed producent/dispatcher v5, połączenie z lokalnym proxy runnera i pełny zdalny tools/call z AHP approval. [Plan i granice](remote-session-plan.md#rozszerzenie-mcp-osobna-wersja-v5-w-realizacji). E5.2/E5.3 i cel E0–E10 pozostają otwarte; formalnie20/68. Nie użyto rzeczywistych kont/usług/tailnetu.
- **Produkcja + realny TLS/WSS/reverse proof + bwrap/LSM:5 zdalnych acceptance PASS**,55,60s po build, ten sam source94bce480: [log](evidence/2026-11-13-mcp-controller-pins/mcp-controller-pins-runtime-final.log). Obejmuje remote_execution_vertical v3, restart procesów, transport trwałej sesji v5, REST/AHP/Journal i transport v4. Są to rzeczywiste sesje z profilami bez MCP i kontrolowanym ACP fixture; nie jest to odbiór zdalnego tools/call. Peak3750617088B/OOM0, bez zwiększania cap3584MiB.
- **Końcowe all-targets Clippy -D warnings PASS**,1m53s, peak2271440896B/OOM0: [log](evidence/2026-11-13-mcp-controller-pins/mcp-controller-pins-clippy-final.log). Pełny backend, pięć runtime acceptance i Clippy mają identyczny source94bce480: [manifest](evidence/2026-11-13-mcp-controller-pins/verified-source.json). Nie zmieniano UI ani crate protokołu, nie powtarzano ich suites. Format Rust, git diff --check i kontrola122 Markdown/local links/anchors PASS.

## 2026-11-13 — E5.3: scoped kontrola SQL i live OAuth refresh

- Dodano RemoteMcpScope bez Deserialize/Serialize/Clone. Capture wymaga aktualnego Journal lease, prywatnych pinów i opaque transportu5. Przypina session/workspace/owner/device/binding revision/profile digest, private intent fingerprint, open identity i monotoniczny deadline. Każdy cut odczytuje bieżące role/profil/piny/konto/config, weryfikuje lease/kanał/producenta, zamknięcie i cancellation oraz SDK authorize_dispatch. Obcy Journal epoch/instance i wymieniony source slot nie ożywiają starego proof.
- Context otwiera własną transakcję wyłącznie z autocommit; context_in_transaction wymaga już istniejącej transakcji i nie wykonuje FS/network. Capture/restore admission nadal wymagają connected; osobne validate_live_snapshot dopuszcza refreshing do weryfikacji już działającej sesji. Nie zwraca odtworzonego bindingu ani permitu. Credential journal zachowuje kontrolę konkretnej generation/operation ID. Rzeczywisty checked OAuth journal w teście przechodzi begin_refresh→context cut→commit i zwiększa generation, zamiast odcinać własną rotację.
- Observe przyjmuje typed świeży Describe zebrany przez host poza SQL, wymaga identycznego export/grant/settings/fence oraz Execute i końcowej kontroli deadline. Sam context nie zastępuje zdalnego Describe. Runner catalog wymaga native SessionReceipt i całego SessionFence; tools/call wymaga active sesji, accepted TurnReceipt, dokładnego TurnFence oraz SDK running. Zakończona SDK tura odcina spóźniony request nawet przy pozostawionym accepted record źródła.
- Sześć nowych testów obejmuje działający cut w transakcji rotacji, odmowę złego rodzaju transakcji, aktywną kontrolę device/account/provider config/host Actor callback, stale lease/source i expired deadline, odwołany Execute w source observation, brak/obcy native receipt oraz pozytywną zaakceptowaną turę i odmowę po zmianie epoch/zakończeniu SDK. To SQL/opaque-slot fixtures, bez rzeczywistego zdalnego MCP. **Focused6 tests PASS**,0,73s po build, peak3568947200B/OOM0: [log](evidence/2026-11-13-mcp-scope/mcp-scope-focused-second.log).
- Pierwszy Clippy wykrył nieużywany reexport przed podłączeniem producenta; udostępniono moduł zamiast importu. Pierwsza kompilacja testów wymagała jawnego typu błędu dwóch closure fixture transakcji; dodano adnotacje. [Clippy diagnostyczny](evidence/2026-11-13-mcp-scope/mcp-scope-clippy-first.log), [kompilacja diagnostyczna](evidence/2026-11-13-mcp-scope/mcp-scope-focused-first.log).
- Do podłączenia pozostają hostowy adapter Authority (Actor/IdP/host settings, fresh Describe przed/po HTTP i monotoniczny deadline tury), producent v5, proxy i REST/AHP. Publiczny Remote MCP oraz stary dispatcher/Actor pozostają zamknięte; nowy scope nie włącza ich sam. E5.2/E5.3 i cel E0–E10 pozostają otwarte; formalnie20/68. Nie użyto rzeczywistych kont/usług/tailnetu.
- **Pełny server625 relay +1 identity +74 SDK +27 terminal +37 usage =764 PASS**,34 osobne acceptance pominięte,139,46s głównej regresji, peak3613315072B/OOM0: [log](evidence/2026-11-13-mcp-scope/mcp-scope-server-final.log), sourcef44ac2f408dd802c033724ef33f15bb27056256413ce73cc7598b4e4e03030f5. Obejmuje sześć nowych testów scope, dotychczasowy zapis pinów/open i lokalne flow MCP.

- **Produkcyjny executable + realny bwrap/LSM + lokalne stdio MCP PASS**,1,83s po build, peak3731644416B/OOM0: [log](evidence/2026-11-13-mcp-scope/mcp-scope-runtime-final.log). Approve/deny/disconnect/lost response zachowują1/0/0/1 tools/call bez retry. To regresja lokalnego proxy, nie odbiór zdalnego MCP. **All-targets Clippy -D warnings PASS**,1m53s, peak2278473728B/OOM0: [log](evidence/2026-11-13-mcp-scope/mcp-scope-clippy-final.log). Pełny backend, lokalny runtime i Clippy mają identyczny sourcef44ac2f4: [manifest](evidence/2026-11-13-mcp-scope/verified-source.json).

## 2026-11-13 — E5.3: żywy lease aktora i nieodnawialny zegar tury

- Zdalny Actor zatrzymuje oryginalny Journal lease otrzymany wyłącznie przy nowym admission. Dodatnie cut open/claim/submit/renew/monitor i queue porównują session/owner/epoch/instance ze świeżym snapshotem. SQL nie może podmienić lease działającego aktora. Retry admission nadal nie tworzy aktora ani zegara.
- Po własnym claim, przed SubmitTurn, Actor tworzy lokalny zegar z dokładną komendą i minimum deadline sesji oraz budżetu tury. Wysyłana komenda musi być identyczna, a oczekiwanie submit jest ograniczone także tym deadline. Ponowny begin nie przedłuża czasu; running zatrzymuje pozytywne działania po wygaśnięciu. Receipt accepted/replay nie tworzy ani nie resetuje zegara. Odczyt zatwierdzonej aktywności bez bieżącej tury usuwa zegar, zachowując najwyższy indeks; cudzy aktywny identyfikator i reuse indeksu są odrzucane.
- Cztery nowe testy obejmują pierwotny deadline mimo oczekiwania/duplicate start, krótszy deadline sesji, podmianę komendy nawet z prawidłowym nowym hashem, rozdzielenie kolejnych tur, odmowę rekonstrukcji zegara z SQL oraz brak zużycia indeksu po błędnym/expired claim. To lokalne testy żywego stanu, nie pełny remote MCP acceptance. Hostowy adapter Authority i producer/proxy v5 nadal wymagają podłączenia; publiczny remote MCP pozostaje zamknięty. E5.2/E5.3 i cel E0–E10 pozostają otwarte, formalnie20/68. Bez rzeczywistych kont/usług/tailnetu.

- **Pełny backend629 relay +1 identity +74 SDK +27 terminal +37 usage =768 PASS**,34 osobne acceptance pominięte,140,06s głównej regresji, peak3571417088B/OOM0: [log](evidence/2026-11-13-remote-actor-clock/remote-actor-clock-server.log). Wszystkie cztery nowe testy zegara wykonane. Source d84033db; końcowy source31b60485 różni się wyłącznie zawinięciem jednego wywołania przez rustfmt: [patch](evidence/2026-11-13-remote-actor-clock/final-format.patch), [dowód odtworzenia hasha w prywatnej kopii](evidence/2026-11-13-remote-actor-clock/source-proof.json).

- **Produkcja + rzeczywisty TLS/WSS/reverse proof + bwrap/LSM:5 zdalnych acceptance PASS**,55,55s po build, source31b60485, peak3758096384B/max1/OOM0: [log](evidence/2026-11-13-remote-actor-clock/remote-actor-clock-runtime.log). Obejmuje v3 execution, restart procesów, trwały transport v5, REST/AHP/Journal i v4 transport, w tym kolejne tury. To regresja profili bez MCP i kontrolowanego ACP fixture; nie jest odbiorem zdalnego tools/call ani rzeczywistego konta/modelu/tailnetu.

- **Końcowe all-targets Clippy -D warnings PASS**,1m53s, source31b60485, peak2264731648B/OOM0: [log](evidence/2026-11-13-remote-actor-clock/remote-actor-clock-clippy.log). [Manifest źródeł i logów](evidence/2026-11-13-remote-actor-clock/verified-source.json) rozróżnia snapshot pełnego backendu i identyczne finalne snapshoty runtime/Clippy; różnica to wyłącznie udokumentowany format jednego wywołania. Nie zmieniano UI ani crate protokołu, nie powtarzano ich suites. Format Rust, git diff --check oraz kontrola122 Markdown/linków/anchors PASS.

## 2026-11-13 — E5.3: hostowy adapter Remote Authority i SQL policy cut

- Dodano Remote implementujący wspólne Authority. Capture pochodzi z żywego Actor i pierwotnego Journal lease; dopasowuje AppState/Context, a owner/session/epoch/instance są zamrożone przez scoped proof. Adapter przechowuje Weak Actor/Service, nie AppState. Stop, finished, utrata właściciela lub zmiana stanu odcina przyszłe cut. Turn pochodzi wyłącznie z żywego zegara; validate_request łączy go z native receipt/TurnFence i SDK running, z kontrolą także po oczekiwaniu.
- Check zbiera nowy Describe przez produkcyjną fabrykę metadanych poza Store, z blocking workera/socket thread, i kończy świeżą SQL kontrolą scope/export/grant/settings/Execute/IdP/polityki hosta. Nie blokuje właściciela WebSocket podczas oczekiwania. Deadline jest sprawdzany przed i po operacji. Nie ma cache dodatniego Describe ani rekonstrukcji zegara z SQL.
- Wspólny broker pobiera SqlAuthority przed Store lock w Journal, permit i OAuth credential journal. Remote cut zajmuje bounded slot kontroli polityki i Management lock, odczytuje bounded/no-follow host_accounts przed SQL i zachowuje politykę przez cały SQL/OAuth cut. Callback w IMMEDIATE robi tylko bieżące kontrole SQL/atomics/monotonic clock; nie czyta plików, nie robi network ani nested transaction. Bezpośrednie stare context Remote odmawia bez cut. Cut nie jest trzymany podczas HTTP lub oczekiwania na zgodę. Dotychczasowy lokalny adapter zachowuje swoje kontrole.
- Zmieniono powiadomienie Authority.changed tak, aby Remote mógł obudzić trwały Journal/AHP po zatwierdzonym zapisie, używając już trzymanego Store. Nie wchodzi ponownie pod ten mutex. Adapter nie dodaje silnego powiązania z Actor/Service. Integracja właściciela brokera musi dodatkowo wyeliminować ścieżkę przez AppState.remote_sessions, jeśli Actor będzie przechowywał broker; sam test adaptera nie dowodzi zwalniania całego przyszłego producenta.
- Pierwszy Clippy wykrył rozróżnienie Tokio/std Instant i prywatny semafor lokalnego interactive Service. Ujednolicono zegar adaptera z kontraktem Tokio; Remote korzysta z bounded policy_slot własnego Context. [Log diagnostyczny](evidence/2026-11-13-remote-mcp-adapter/remote-mcp-adapter-clippy-first.log). **Wstępne5 focused tests PASS**,0,82s, peak3596812288B/OOM0: [log](evidence/2026-11-13-remote-mcp-adapter/remote-mcp-adapter-focused-first.log). Końcowa pełna regresja obejmuje dodatkowo świeży Describe i końcowe kontrole po oczekiwaniu.
- Testy korzystają z rzeczywistego Service i produkcyjnego adaptera/SQL/OAuth journal, z kontrolowanym aktorem bez dispatch oraz synthetic opaque source slot/native receipts. Dodatkowy responder używa prawdziwej kolejki/fabryki Describe, ale nie jest WSS. To nie jest pełny odbiór zdalnego tools/call. Capture adaptera nie zostało jeszcze wpięte w publiczne admission ani producenta v5; broker/runner proxy/REST pozostają do podłączenia. Publiczny Remote MCP pozostaje zamknięty. E5.2/E5.3 i cel E0–E10 pozostają otwarte, formalnie20/68. Bez rzeczywistych kont/usług/tailnetu.

- Przegląd permit wykrył dodatkowy legacy odczyt host_accounts przy sprawdzaniu urządzenia zatwierdzającego. Dodano permission_device do SqlAuthority: Remote używa tego samego przechowanego policy cut i świeżej SQL kontroli owner/device/IdP, bez FS pod Store. Lokalny adapter zachowuje dotychczasową kontrolę. Test polityki obejmuje teraz również obcego właściciela, drugie urządzenie tego samego ownera, jego revocation i fail-closed kolejnego cut przy malformed settings. SDK nadal osobno weryfikuje oryginalną jednorazową zgodę/permit. Końcowe suites muszą obejmować tę poprawkę; wcześniejszy snapshot nie stanowi jej odbioru.

- **Wstępny pełny backend635 relay +1 identity +74 SDK +27 terminal +37 usage =774 PASS**,34 osobne acceptance pominięte,140,89s głównej regresji, peak3656441856B/OOM0: [log](evidence/2026-11-13-remote-mcp-adapter/remote-mcp-adapter-server-first.log), source11c32a0f. Obejmuje sześć nowych testów, w tym nowe Describe na każde check i odmowę po odebraniu Execute. Ten snapshot poprzedza końcową kontrolę urządzenia zatwierdzającego przez SqlAuthority i nie stanowi odbioru tej ostatniej poprawki.

- **Końcowy all-targets Clippy -D warnings PASS**,1m55s, peak2181033984B/OOM0, source8a0f33b165b67dac5fe5a97d88a9f8c44de9a2ad81ab1b2c7072da25fd56b0f9: [log](evidence/2026-11-13-remote-mcp-adapter/remote-mcp-adapter-clippy-final.log). Obejmuje końcową kontrolę urządzenia przez SqlAuthority; wykonanie końcowych asercji potwierdzają osobne suites.

- **Końcowy pełny backend635 relay +1 identity +74 SDK +27 terminal +37 usage =774 PASS**,34 osobne acceptance pominięte,140,40s głównej regresji, peak3594715136B/OOM0: [log](evidence/2026-11-13-remote-mcp-adapter/remote-mcp-adapter-server-final.log). Source8a0f33b1 jest identyczny z końcowym Clippy. Wykonano wszystkie sześć testów adaptera wraz z dodatkowymi asercjami urządzenia zatwierdzającego i końcowymi kontrolami po oczekiwaniu.

- Pierwszy focused runtime zbudował produkcyjny executable, lecz linker programu testowego zakończył się SIGBUS przed wykonaniem asercji: [log diagnostyczny](evidence/2026-11-13-remote-mcp-adapter/remote-mcp-adapter-runtime-link-failure.log). Peak równy cap3584MiB, memory.events.max136/OOM0; przed test build tmpfs miał341372KiB wolnego. Nie zaliczono runtime na podstawie tego przebiegu. Pierwsza próba poprawki launchera zachowywała rzeczywisty produkcyjny executable przez hardlink /scratch/dunlin-relay-e2e i cargo clean -p dunlin-relay usuwa wyłącznie dev artefakty tego pakietu przed linkowaniem testu. Dependencies pozostają w tmpfs; cap RAM/scratch/PID/file/wall, non-root, read-only/no-network i polityki bwrap/LSM nie zostały zwiększone ani wyłączone. Dodano końcowe df/du także dla runtime. **18 testów launchera PASS** (w tym kolejność build→preserve executable→package clean→test oraz niezmieniona izolacja): [log](evidence/2026-11-13-remote-mcp-adapter/remote-mcp-adapter-launcher-tests.log). **14 testów CI PASS**: [log](evidence/2026-11-13-remote-mcp-adapter/remote-mcp-adapter-ci-tests.log). Kod Rust pozostaje source8a0f33b1; odbiór runtime jest ponawiany ze zmienioną organizacją jego scratch.

- Próba hardlink/package clean nie zmniejszyła rzeczywistego wykorzystania tmpfs: usunięto aliasy binarium, którego inode nadal zachowywał executable. Drugi link również zakończył się SIGBUS przed testami, max516/OOM0: [log](evidence/2026-11-13-remote-mcp-adapter/remote-mcp-adapter-runtime-clean-link-failure.log). Usunięto ten wariant. Końcowy launcher najpierw kompiluje testy przez --no-run bez zatrzymanego produkcyjnego executable, potem buduje produkcyjny executable, a dopiero potem wykonuje wymagane asercje. Full runtime nadal wykonuje całą preparatory regression po zbudowaniu produkcji; focused kompiluje tylko root test binary i wykonuje wyłącznie wskazany ignored acceptance. Powrócono do DUNLIN_RELAY_E2E_BIN=/scratch/target/debug/dunlin-relay; nie ma package clean/hardlink ani podniesienia limitów. Końcowe18 testów launchera sprawdza kolejność i zachowanie obu zakresów;14 testów CI także PASS (logi powyżej zaktualizowano).

- **Końcowy produkcyjny executable + realny bwrap/LSM + lokalne stdio MCP PASS**,1,79s po build: [log](evidence/2026-11-13-remote-mcp-adapter/remote-mcp-adapter-runtime-final.log). Approve/deny/disconnect/lost response zachowują1/0/0/1 tools/call bez replay. Test binary został zlinkowany jako pierwszy, potem rzeczywisty executable produkcyjny; odbiór wykonał się dopiero po obu build. Peak3749437440B, memory.events.max0/OOM0; końcowy scratch89%,177780KiB wolnego. Nie podniesiono limitów. To odbiór lokalnego wspólnego brokera po zmianie SQL cut, nie pełnego zdalnego MCP.
- [Końcowy manifest](evidence/2026-11-13-remote-mcp-adapter/verified-source.json) wiąże identyczny Rust source8a0f33b1 pełnego backendu, Clippy i produkcyjnego runtime, a także hash końcowego launchera i wyniki18+14 jego/CI testów. Nie zmieniano UI ani crate protokołu. Format Rust, git diff --check i kontrola122 Markdown/linków/anchors PASS. Adapter nadal wymaga podłączenia do producenta v5 i pełnego odbioru runner proxy/REST/AHP. Cel E0–E10 pozostaje aktywny, formalnie20/68.

## 2026-11-13 — E5.3: session-owned producent transportu MCP v5

- WSS kontrolera dispatchuje poprawne żądania v5 do session-owned SessionBroker zamiast zawsze odmawiać bound sesjom. Rejestr jest tworzony wewnątrz opaque SessionTransport. RAII registration ma prywatną generację; map przechowuje Weak callback, owner trzyma rzeczywistą referencję. Bind wymaga v5/live i ID ASCII, jest unikalny oraz ograniczony do8 sesji. Owner drop usuwa wyłącznie własny wpis i interruptuje; kanał drop/replacement/shutdown interruptuje wszystkie jego wpisy. Sam bind nie jest native/private/SDK authority.
- Socket select ma osobny Exchange: async callback futures oraz gotowe odpowiedzi, bez oczekiwania inline na zgodę/HTTP. Queued/waiting work i fragmentowane odpowiedzi dzielą wynegocjowane max_pending; slot pozostaje zajęty do ostatniego fragmentu. Przed chunkowaniem sprawdzany jest limit odpowiedzi i pełny JSON. Każdy fragment sprawdza aktualność właściciela/kanału. Malformed/oversized wynik, timeout150s albo utrata producenta interruptuje pracę i daje OutcomeUnknown, bez częściowego wyniku ani retry. Wysłanych IDs nie da się ponowić; historia ma limit4096/kanał. Callback musi osobno zachować sloty rzeczywistych blocking/HTTP workers po anulowaniu waitera.
- Bez właściciela nadal Denied bez discovery/OAuth/HTTP, v4 bind odmawia, obcy channel/boot zamyka protokół. Readiness UI/REST nie zmieniono. Hostowa fabryka łącząca Common Broker/Remote Authority/prywatne konta z tym callbackiem oraz runner dispatcher/proxy i nowy open/admission wciąż wymagają podłączenia. Publiczny remote profil MCP pozostaje zamknięty.
- Sześć nowych unit tests sprawdza zajętość slotu przez cały wielofragmentowy UTF-8 wynik, odmowę capacity przed dispatch, ID replay, owner replacement bez reroute/late częściowego wyniku, expiry/drop bez retry, malformed/negotiated oversized odpowiedzi, unikalność/limit8 i source stop wszystkich ownerów, v4/unbound/foreign boot bez callbacku. Rzeczywisty TLS/WSS/reverse proof fixture ma dodatkowy bound producer: gate czeka na kontrolowaną decyzję, a następny heartbeat jest zapisany/potwierdzony przed zwolnieniem odpowiedzi; później katalog i call przechodzą tym transportem, a foreign boot odcina producenta. To kontrolowany transportowy callback, bez SQL/native receipts/konta/SDK approval ani zewnętrznego MCP HTTP; nie jest pełnym remote tools/call acceptance.
- **Pełny backend642 relay +1 identity +74 SDK +27 terminal +37 usage =781 PASS**,34 osobne acceptance pominięte,143,87s głównej regresji, peak3586408448B/OOM0: [log](evidence/2026-11-13-mcp-v5-producer/mcp-v5-producer-server-final.log), sourcedfca30e163fbe3da6d26e0913e252d70c952fe4ded0213bda650fcf432720092. Wykonano wszystkie siedem nowych testów, dotychczasową odmowę unbound i istniejące regresje. Wstępny all-targets Clippy także PASS przed dopisaniem końcowego TLS/capacity/source-stop zakresu: [log](evidence/2026-11-13-mcp-v5-producer/mcp-v5-producer-clippy-first.log). To wcześniejszy snapshot, nie końcowy odbiór wszystkich nowych asercji.
- E5.2/E5.3 i cel E0–E10 pozostają otwarte, formalnie20/68. Nie użyto rzeczywistych kont/usług/tailnetu. Następne zależności: hostowa fabryka producenta z własnym scoped brokerem i bez cyklu przez AppState.remote_sessions, runner dispatcher/proxy z native lease guards, pełny REST/AHP→TLS/WSS→runner→MCP odbiór.

- **Końcowy all-targets Clippy -D warnings PASS**,1m54s, peak2228850688B/OOM0: [log](evidence/2026-11-13-mcp-v5-producer/mcp-v5-producer-clippy-final.log). **Produkcyjny executable + realny TLS/WSS/reverse proof + bwrap/LSM:5 zdalnych acceptance PASS**,54,92s po build, peak3588894720B/max0/OOM0: [log](evidence/2026-11-13-mcp-v5-producer/mcp-v5-producer-runtime-final.log). Obejmuje v3 execution, restart procesów, v5 session transport, REST/AHP/Journal i v4 transport dla profili bez MCP. To regresja rzeczywistego runtime/transportu, nie pełnego zdalnego tools/call.
- [Końcowy manifest](evidence/2026-11-13-mcp-v5-producer/verified-source.json) wiąże identyczny sourcedfca30e1 pełnego backendu, Clippy i runtime. Nie zmieniano UI ani crate protokołu, nie powtarzano ich suites. Format Rust, git diff --check i kontrola122 Markdown/linków/anchors PASS. Formalnie20/68, cel E0–E10 aktywny. Do domknięcia E5 pozostaje rzeczywiste spięcie hostowej fabryki brokera/autoryzacji/konta z nowym open i producentem, runner dispatcher/proxy oraz pełny REST/AHP→runner→MCP odbiór z revoke/unknown/no replay.

## 2026-11-13 — E5.3: hostowa fabryka v5, prywatne konto, SDK consent i rzeczywisty MCP HTTP

- Jednorazowe install dla oryginalnego Actor tworzy Remote Authority, świeżo odtwarza prywatne połączenia z controller snapshot, przygotowuje Common Broker i wiąże go jako rzeczywisty SessionBroker opaque transportu5. Started nie jest odtwarzany z wire/SQL; fail/cancel setup sygnalizuje stop. Finish zamyka RAII registration. Scope nadal wymaga oryginalnego Journal lease, konta owner/workspace/revision/config i aktualnego źródła; żadne flagi gotowości/DTO nie zastępują tych kontroli.
- Kopia AppState przechowywana przez Hosted i Core ma oddzielny pusty remote_sessions OnceLock. Usuwa to rzeczywistą ścieżkę Actor→registration→producer→Broker→AppState→Service→Actor. Test wpisuje Service do oryginalnego AppState, porzuca bez ręcznego finished aktora, usuwa zewnętrznych właścicieli i wymaga zaniku producenta/Service oraz powrotu8 runtime slotów.
- Native request może wyprzedzić import receipt. SQL readiness zwraca pending tylko dla oryginalnego opening z send markerem albo oryginalnego sent turn command z SDK dispatching. Callback czeka do2s/min RPC deadline, budzony zmianą aktora, stale rewaliduje authority i nie tworzy zgody ani nie wywołuje narzędzia do exact native receipt + SDK running + własnego zegara. Nie ponawia open/submit/HTTP.
- Slot per-session pracy jest przenoszony do rzeczywistego spawn_blocking, który zachowuje Hosted/Core do końca HTTP także po abort waitera/utracie aktora. Test blokuje rzeczywistą odpowiedź HTTP już po dispatch, anuluje waiter i usuwa aktora, wymaga nadal zajętego slotu sieciowego i runtime, po czym zwalnia odpowiedź i wymaga pełnego zwrotu pojemności. Nie utożsamia zaniku Weak z zakończeniem drop wszystkich pól/permitów.
- Core request_bounded propaguje deadline RPC do zgody/dispatch/końcowego guard i respektuje limit odpowiedzi sesji. Katalog ma strony w tym budżecie; zbyt duża pojedyncza definicja odcina setup. Wynik większy niż limit daje unknown/interrupt bez częściowej dostawy i replay. Test32KiB sesji/40KiB poprawnego wyniku wymaga zakończonego SDK mcpOutcome:complete, lecz odmowy dostarczenia większego wyniku runnerowi po dokładnie jednym HTTP tools/call. Lokalne request zachowuje swój limit256KiB i dotychczasową granicę stron192KiB. Dodano końcowy deadline fence po Journal cut.
- Wyłącznie lokalne błędy walidacji sprzed MCP dispatch są InvalidRequest i nie zatrzymują zdrowej sesji. Test błędnego aliasu wymaga0 paid calls i dopuszcza poprawne świeże żądanie ze zgodą oraz1 call. Pozostałe niepewne wyjścia zatrzymują aktora jako unknown. Tokeny, referencje vault i connection IDs nie są przekazywane do runnera.
- Siedem testów mostu używa faktycznej fabryki, Common Broker, checked OAuth journal, prawidłowo wydanej/utrwalonej po PKCE odpowiedzi TokenSet, prywatnego MemoryStore i rzeczywistego lokalnego HTTP MCP. Obejmuje approve/deny/disconnect przy użyciu poświadczenia po decyzji/lost reply z dokładnym1/0/0/1 tools/call, brak drugiego dispatch, wyścig session/turn receipts przed SDK consent, cancelled waiter/capacity, owner cycle/drop i negotiated reply limit. Native receipts/aktor i responder metadanych pozostają kontrolowanymi fixtures; transport driver jest in-process, nie WSS/runner. To nie odbiór pełnego REST/AHP→TLS/WSS→runner→MCP pionu.
- Pierwszy Clippy wymagał idiomatycznego RangeInclusive.contains: [log](evidence/2026-11-13-mcp-host-bridge/mcp-host-bridge-clippy-first.log). Drugi wykrył reexport install nieużywany w non-test; zastąpiono go normalną funkcją delegującą do fabryki: [log](evidence/2026-11-13-mcp-host-bridge/mcp-host-bridge-clippy-second.log). Pierwszy focused4/6 wymagał synchronizacji fixture: disconnect hook ustawiany dopiero po pojawieniu się promptu, a zwrot pojemności sprawdzany po rzeczywistym drop permitów: [log](evidence/2026-11-13-mcp-host-bridge/mcp-host-bridge-focused-first.log). Poprawione focused6/6 PASS,3,72s, peak3678633984B/OOM0: [log](evidence/2026-11-13-mcp-host-bridge/mcp-host-bridge-focused-second.log). Końcowy zakres dodaje odrzucenie aliasu/poprawny followup.
- **Końcowy pełny backend649 relay +1 identity +74 SDK +27 terminal +37 usage =788 PASS**,34 osobne acceptance pominięte,144,13s głównej regresji, peak3618299904B/OOM0: [log](evidence/2026-11-13-mcp-host-bridge/mcp-host-bridge-server-final.log), source466e0c33885c5b9946fe03b8711ece976376b381033cc8c0189b1d345ab4ae0e. Wszystkie siedem nowych testów wykonane. **All-targets Clippy -D warnings PASS**,1m52s, peak2253795328B/OOM0, ten sam source: [log](evidence/2026-11-13-mcp-host-bridge/mcp-host-bridge-clippy-final.log).
- Fabryka jest gotowa wewnętrznie, ale jej install oraz nowe positive cuts/import receipt/negative cleanup nie są jeszcze wywoływane przez publiczny remote Actor/admission. Typed open5, runner dispatcher/proxy i pełny pion pozostają do podłączenia; publiczne remote MCP nadal odmawia. E5.2/E5.3 i E0–E10 pozostają otwarte, formalnie20/68. Nie użyto rzeczywistych kont/usług/tailnetu.

- **Produkcyjny executable + realny bwrap/LSM + lokalne stdio MCP PASS**,1,59s po build, ten sam source466e0c33: [log](evidence/2026-11-13-mcp-host-bridge/mcp-host-bridge-runtime-final.log). Approve/deny/disconnect/lost response zachowują1/0/0/1 wywołań bez replay. Peak3627876352B/max0/OOM0, scratch89%,175588KiB wolnego. To odbiór wspólnego lokalnego brokera, nie pełnego zdalnego MCP. [Manifest](evidence/2026-11-13-mcp-host-bridge/verified-source.json) wiąże pełny backend788, Clippy i ten runtime z identycznym snapshotem Rust oraz hashem launchera.

## 2026-11-13 — E5.3: typowane otwarcie v5 w kolejce kontrolera

- SessionTransport ma open_broker_session oraz wewnętrzną kolejkę Control z odrębnymi wariantami legacy i broker. Oryginalna komenda5 przechodzi przez istniejącą bounded kolejkę i WS owner; encode wysyła open_broker_session, bez projekcji na legacy v4. Przed enqueue i ponownie przed send sprawdzane są żywy kanał/producent, wynegocjowana oferta i limity, fingerprint całego payload oraz authenticated host/workspace/node/enrollment/credential revision. V4 odmawia bez zajęcia slotu/kolejki. Legacy request/ACK zachowuje swój format JSON i reguły.
- Pending control jest korelowany z oryginalnym wariantem. SessionAccepted musi przejść walidację receipt5, a Rejected wskazać dokładny kanał i całą open identity; zmiana payload z dawnym hashem odcina także korelację odmowy. SessionReady/events nie konsumują RPC. Nie zmieniano DTO protokołu, polityki autoryzacji, limitu8 control RPC, deadline ani obsługi heartbeat.
- Dwa nowe unit tests obejmują odrębny wire5/odmowę kodeka4, dokładny receipt/odmowę, obcy boot/identity/budget/session, readiness bez ACK, zmiany limitów/payload oraz authenticated host/workspace/node/epoch/revision. Każda odmowa send pozostawia pustą kolejkę i wszystkie sloty. Istniejące testy legacy pozostają w pełnej regresji.
- **Wstępny pełny backend651 relay +1 identity +74 SDK +27 terminal +37 usage =790 PASS**,34 osobne acceptance pominięte,146,23s głównej regresji, peak3688136704B/max0/OOM0: [log](evidence/2026-11-13-mcp-open-transport/mcp-open-transport-server-first.log). Sourcef0a4e82996c2c26fbf1257578c2916ce3f9325676c944b7f3217ec2f1beea0fd. Po tym snapshotcie rozszerzono test rzeczywistego TLS/WSS/reverse proof o nowe open5 i odpowiedzi; jego końcowy odbiór jest jeszcze wymagany.
- Zmiana dostarcza transportową ścieżkę typed open5; wywołanie jej przez publiczny Actor/admission, scoped SQL dispatch/import/negative cleanup, odbiorca runtime runnera oraz lokalne proxy nadal pozostają do podłączenia. Publiczny remote MCP nadal odmawia. Nie oznaczono E5.2/E5.3 jako ukończonych; formalnie20/68, cel E0–E10 aktywny. Bez rzeczywistych kont/usług/tailnetu.

- **Końcowy pełny backend790 PASS**, w tym rzeczywisty TLS/WSS/reverse proof z typowanym open5, skorelowanym SessionAccepted oraz dokładnym Rejected; obcy hash odmowy i SessionReady nie kończą waitera. Późniejszy bound MCP callback nadal czeka niezależnie od heartbeat, a foreign boot zamyka kanał. To transportowy peer fixture, bez native wykonania MCP: [log](evidence/2026-11-13-mcp-open-transport/mcp-open-transport-server-final.log). Source9eb1f84f1798be9bd4ff816e35dc10854286617709fbb27d5fd15861cd570b2e,145,44s głównej regresji, peak3635245056B/max0/OOM0. Końcowy kod sprawdza limit całego envelope open5 przed enqueue i send, nie tylko wewnętrznej komendy. Wstępny Clippy PASS przed tą ostatnią kontrolą: [log](evidence/2026-11-13-mcp-open-transport/mcp-open-transport-clippy-first.log). Końcowy runtime i Clippy pozostają do wykonania na source9eb1f84f.

- **Produkcyjny executable + rzeczywisty TLS/WSS/reverse proof + bwrap/LSM:5 zdalnych acceptance PASS**,55,11s po build, source9eb1f84f: [log](evidence/2026-11-13-mcp-open-transport/mcp-open-transport-runtime-final.log). Obejmuje execution v3, restart procesów, trwałe sesje v5, REST/AHP/Journal i v4 dla profili bez MCP. Peak3728822272B/max0/OOM0, scratch89%,174984KiB wolnego. W scenariuszu restartu kontrolowany LocalAPI HTTP responder wypisał background BrokenPipe przy write odpowiedzi do zamkniętego socketu (worker_fixture.rs, test-only); wszystkie pięć asercji scenariuszy zakończyło się PASS i launcher exit0. Nie jest to odbiór pełnego zdalnego tools/call.

- **Końcowy all-targets Clippy -D warnings PASS**,1m53s, peak2201538560B/max0/OOM0: [log](evidence/2026-11-13-mcp-open-transport/mcp-open-transport-clippy-final.log). [Manifest](evidence/2026-11-13-mcp-open-transport/verified-source.json) wiąże identyczny source9eb1f84f pełnego backendu790, pięciu produkcyjnych scenariuszy zdalnych i Clippy oraz hash launchera. Nie zmieniano UI ani crate protokołu i nie powtarzano ich osobnych suites. Format Rust, git diff --check i kontrola122 Markdown/linków/anchors PASS. Formalnie20/68, cel E0–E10 aktywny. Następny przekrój to scoped SQL dispatch/import/negative cleanup i Actor korzystający z typed open5, a następnie rzeczywisty odbiorca/proxy runnera i pełny REST/AHP→runner→MCP odbiór.

## 2026-11-13 — E5.3: scoped SQL runtime i wewnętrzny aktor MCP

- Wspólne dodatnie operacje przeniesiono do RemoteSessionRuntime. Wariant legacy pozostaje zamknięty dla MCP; wariant scoped pożycza oryginalny RemoteMcpScope i konfigurację, bez publicznych pól/konstruktora, Clone lub serde. Każda operacja rewaliduje pierwotną sesję/lease/intent/profil, prywatne konto/config i bieżący kanał. Nie można odblokować MCP samym callbackiem lub wierszem SQL. To te same kernels dla legacy i MCP, bez kopii logiki SDK/projekcji. Testowe Store wrappers zachowują istniejące fixtures v4.
- Udana instalacja hostowego brokera zapisuje Arc scope w Actor.mcp; finish usuwa go wraz z RAII bindingiem. Actor positive cuts, monitor, queue eligibility, utracony turn receipt i importer potwierdzeń/zdarzeń wybierają wariant scoped dopiero z tym dowodem. Open pozostaje RemoteOpen aż do wysłania właściwego v4/v5; receipt5 jest ponownie walidowany. Service przechowuje wyłącznie Arc konfiguracji, bez dodawania AppState lub cyklu przez remote_sessions. Dodatnie mutacje mają końcowy deadline fence przed COMMIT, a Actor ponownie sprawdza running przed powrotem z transakcji dispatch.
- Service source/scope służy korelacji właściwego node/host/workspace i opaque kanału, bez wymagania aktywnego konta OAuth. Dodatnia autoryzacja jest osobnym wymogiem SQL. Capture receipt do close, interrupt, negative terminal i rzeczywisty StopProof mogą zakończyć sesję po disconnect; nie nadają Ready/lease/zgody. Dodatni import/renew/projekcja oraz kolejka bez aktualnego scoped dowodu odmawiają. Publiczny admission nadal nie uruchamia MCP ani instalacji brokera; source runtime/proxy i pełny REST/AHP→runner→MCP pozostają otwarte.
- Dodano trzy testy scoped SQL: pełny native lifecycle/renew/projekcja uporządkowanych zdarzeń, odmowa legacy/obcej sesji/callbacku/source/account/config przed send markerem oraz rollback SDK przy późnym błędzie zapisu receipt i negative stop po disconnect. Dodatkowy test łączy realną fabrykę/prywatny OAuth/HTTP MCP z faktycznymi cuts aktora i importerem native receipts. Source metadata, receipts i request exchange pozostają kontrolowanymi fixtures, bez WSS/native wykonania MCP; nie jest to pełny zdalny acceptance.
- Pierwszy Clippy wskazał nieużywany dawny Store wrapper monitora po przełączeniu aktora na adapter; usunięto go: [log diagnostyczny](evidence/2026-11-13-mcp-scoped-session-runtime/clippy-first.log). Pierwszy focused zatrzymał się przed testami na E0283 w nowym testowym domknięciu transakcji; dodano jawny StoreError: [log diagnostyczny](evidence/2026-11-13-mcp-scoped-session-runtime/focused-first.log). Końcowa weryfikacja jest w toku. E5.2/E5.3 i E0–E10 pozostają otwarte; formalnie20/68. Bez rzeczywistych kont/usług/tailnetu.

- **Focused29 MCP session-store/actor/bridge tests PASS**,7,73s po kompilacji, peak3630514176B/max0/OOM0: [log](evidence/2026-11-13-mcp-scoped-session-runtime/focused-second.log). Source439b6a8e099d152771cf0c1e8d0dc89f9505f22c4149eae06e6591c8fb108464. Wykonano cztery nowe testy oraz wcześniejsze przypadki scope/OAuth refresh/konta/RAII/zgód/capacity. Test aktora uzyskał właściwy open5, importował SessionAccepted/Ready/TurnAccepted przez produkcyjny importer, wykonał dokładnie1 tools/call po SDK consent, odrzucił renew/ready/queue po disconnect i przyjął matching stop bez nowego wywołania. To nadal kontrolowane source receipts/in-process request exchange, bez publicznego admission ani runner proxy.

- **Pełny backend655 relay +1 identity +74 SDK +27 terminal +37 usage =794 PASS**,34 osobne acceptance pominięte,146,43s głównej regresji, peak3738468352B/max0/OOM0: [log](evidence/2026-11-13-mcp-scoped-session-runtime/server-final.log). Source439b6a8e, identyczny jak focused29. Końcowy source9738f16036ea27056b755b4d00e7f65596c39a98f5efe1424f211fd69eb7ba6e różni się wyłącznie dodaniem cfg(test) do konwertera into_legacy, używanego już tylko przez dawne testowe Store wrappers. [Dowód](evidence/2026-11-13-mcp-scoped-session-runtime/source-scope-proof.json) i [skrypt](evidence/2026-11-13-mcp-scoped-session-runtime/verify-source-scope.py) odtwarzają dokładny hash439b6a8e w prywatnej kopii; testowe bodies pozostają takie same. Końcowa kompilacja non-test/Clippy i produkcyjny runtime są weryfikowane osobno na9738f160.

- **Końcowy all-targets Clippy -D warnings PASS**,1m53s, peak2213687296B/max0/OOM0, source9738f160: [log](evidence/2026-11-13-mcp-scoped-session-runtime/clippy-final.log). Weryfikuje także non-test produkcję po usunięciu nieużywanego konwertera v4 z jej kompilacji; nie ukrywano ostrzeżeń broad allow. Produkcyjny zdalny runtime jest w toku.

- **Produkcyjny executable + rzeczywisty TLS/WSS/reverse proof + bwrap/LSM:5 zdalnych acceptance PASS**,54,94s po build, source9738f160: [log](evidence/2026-11-13-mcp-scoped-session-runtime/runtime-final.log). Obejmuje execution v3, restart procesów, trwały transport v5, REST/AHP/Journal i v4 dla profili bez MCP. Peak3635740672B/max0/OOM0, scratch89%,174348KiB wolnego. Nie jest to pełny zdalny tools/call. [Manifest](evidence/2026-11-13-mcp-scoped-session-runtime/verified-source.json) wiąże backend794/focused29 na439b6a8e, końcowy Clippy/runtime na9738f160, odtwarzalny dowód jedynej różnicy cfg(test) oraz hash launchera. Format Rust, git diff --check i kontrola122 Markdown/linków/anchors PASS. Nie zmieniano UI ani crate protokołu. E5.2/E5.3 i cel E0–E10 pozostają otwarte; formalnie20/68. Następnie: source dispatcher podłączony do WSS i opaque broker connection runtime, lokalne proxy z native lease/FS checks, instalacja brokera przy publicznym admission i pełny REST/AHP→runner→MCP odbiór.

## 2026-11-13 — E5.3: dispatcher w WSS runnera i żywy broker connection runtime

- Źródłowy session_with_runtime tworzy SocketOwner/Client po zweryfikowanym Accepted i reverse proof wyłącznie dla v5. Hello5 i owner korzystają z tej samej oferty8/256KiB. W pętli select są odbiór skorelowanych MCP replies, osobne next/send oraz timer100ms przed incoming frames, niezależne od HTTP/zgody i native session jobs. Outbound zachowuje oryginalny deadline RPC, a send jest ograniczony także TTL kanału i SEND. Nie ma retry po niepewnym send/timeout.
- Client zawiera niezmienną ofertę i attestation pełnego ChannelFence, żywego guard oraz otwartej kolejki i globalnych slotów. Runtime.broker_connection wymaga rzeczywistego opaque klienta; Connected.authorize/context ponownie go sprawdza i dopiero wtedy tworzy ExecutionChannel::verify_broker. JSON/boolean/komenda nie zastępują klienta. Zamknięty owner nie odzyskuje authority po ustawieniu Connected.open lub odnowieniu samej flagi TTL. Legacy connection/context pozostaje bez oferty.
- Dwa nowe testy dispatchera sprawdzają original deadline, oddzielne dekodery ramek i zachowanie pending, oddanie wyłącznie całej odpowiedzi, odmowę obcego boot oraz owner drop; oversized/unmatched replies zamykają parę bez częściowego wyniku. Dwa testy Connected sprawdzają oryginalny fence, niezależny open guard i trwałą utratę proof po drop owner. Nie uruchamiają procesu ani sandboxa.
- Odbiór produkcyjny rozszerzono o obserwację klienta utworzonego przez rzeczywistą pętlę v5 (cfg(test), RAII usuwane tylko dla własnego fence). Kontrolowany hostowy responder trzyma pierwszy katalog, drugi odpowiada niezależnie, a heartbeat jest zapisany przed zwolnieniem wielofragmentowego80KiB wyniku. Nowy osobny timeout scenario ma zamknąć rzeczywisty kanał, zatrzymać aktywny sandbox i zapisać native stopped/reservation0 bez ponownego session/new. To transportowe probes dla profilu bez MCP, bez prywatnego konta, one-use SDK zgody lub wykonania tools/call; pełny zdalny MCP nadal wymaga source proxy/native checks i publicznego admission.
- Wstępny Clippy podstawowego podłączenia PASS przed dodaniem nowych testów i testowego observera: [log](evidence/2026-11-13-mcp-source-socket/clippy-first.log). Końcowe odbiory są w toku. Publiczny profil MCP nie został odblokowany. E5.2/E5.3 i E0–E10 pozostają otwarte; formalnie20/68. Bez rzeczywistych kont/usług/tailnetu.

- **Pełny backend659 relay +1 identity +74 SDK +27 terminal +37 usage =798 PASS**,35 osobne acceptance pominięte,145,76s głównej regresji, peak3662053376B/max0/OOM0: [log](evidence/2026-11-13-mcp-source-socket/server-final.log), source2392372a1ab2231385a06bb666d8f91e0f1f10ac65623f5c0f98bf4b89d221e7. Wszystkie cztery nowe testy wykonane. Produkcyjny runtime został rozszerzony z5 do6 scenariuszy; nowy timeout nie został zaliczony na podstawie zwykłej regresji. Jego rzeczywisty odbiór jest w toku.

- **Produkcyjny executable + rzeczywisty TLS/WSS/reverse proof + bwrap/LSM:6 zdalnych acceptance PASS**,58,54s po build, source2392372a: [log](evidence/2026-11-13-mcp-source-socket/runtime-final.log). Rozszerzony v5 scenario utrzymał heartbeat przy wstrzymanej odpowiedzi, dostarczył drugi request niezależnie i złożył pełny80KiB UTF-8 wynik. Nowy broker_client_timeout uzyskał1 kontrolowany RPC, zwrócił OutcomeUnknown, zakończył worker loop dokładnym worker_broker_expired i zatrzymał aktywny sandbox; native SQL potwierdził stopped/reservation0, dokładnie1 session/new i brak effects. Pozostałe v3/restart/v4/REST-AHP-Journal także PASS. Peak3725541376B/max0/OOM0, scratch89%,173544KiB wolnego. Profil wykonania nadal bez MCP, hostowy callback transportowy bez OAuth/SDK consent/paid tools; nie jest to pełny zdalny tools/call.

- **Końcowy all-targets Clippy -D warnings PASS**,1m54s, peak2214739968B/max0/OOM0: [log](evidence/2026-11-13-mcp-source-socket/clippy-final.log). [Manifest](evidence/2026-11-13-mcp-source-socket/verified-source.json) wiąże identyczny source2392372a backendu798, sześciu runtime acceptance i Clippy oraz hash launchera. Format Rust, git diff --check i kontrola122 Markdown/linków/anchors PASS. Nie zmieniano UI ani crate protokołu; nie powtarzano ich osobnych suites. Publiczny MCP pozostaje zamknięty do czasu źródłowego odbiorcy open5, lokalnego proxy z native session/turn/FS cuts, konfiguracji MCP agenta i podłączenia admission/startup. E5.2/E5.3 i E0–E10 otwarte; formalnie20/68. Bez rzeczywistych kont/usług/tailnetu.

## 2026-11-13 — E5.3: natywne proxy MCP runnera, odbiór w toku

- Runner zachowuje typowane open5 przez kolejkę sesji i otwiera natywną sesję
  wyłącznie z oryginalnym broker Client, ofertą kanału, lokalną polityką MCP
  oraz własnym grantem Read/Execute. Legacy open4 pozostaje oddzielny.
- Dodano prywatny Unix socket i rzeczywisty helper `--mcp-tools` w sandboxie.
  Agent dostaje wyłącznie adres tego socketu i losowy dowód sesji; żądania
  katalogu/narzędzia idą przez istniejący WSS do brokera kontrolera. Runner
  nie otrzymuje tokenów OAuth ani konfiguracji zewnętrznych endpointów.
- Proxy sprawdza oryginalne native session/turn leases, bieżący grant, checkout,
  zakres tury i nieodnawialny deadline przed RPC, po odpowiedzi oraz przed
  lokalnym dostarczeniem. Wybrany limit odpowiedzi jest sprawdzany przed
  składaniem fragmentów; utrata potwierdzonego wyniku lokalnie zatrzymuje sesję.
- **Backend 799 PASS** (660 relay +1 identity +74 SDK +27 terminal +37 usage),
  38 osobnych acceptance pominiętych; źródło
  `72f507d109012b0ca3d82d5827f1d261a857b3c0e3bc397f942c95b98361a602`,
  peak3652395008B, OOM0: [log](evidence/2026-11-13-mcp-native-proxy/server-before-delivery-fix.log).
  Ten wynik poprzedza poprawkę zatrzymania po błędzie zapisu Unix, ostatnią
  poprawkę lint i czwarty nowy scenariusz; nie dowodzi odbioru końcowego źródła.
- Dodano cztery produkcyjne scenariusze: katalog i dwie tury tools/call,
  cofnięcie lokalnego Execute przy wstrzymanym wyniku, limit odpowiedzi
  mniejszy od oferty kanału oraz zamknięcie lokalnego socketu po wysłaniu
  tools/call. Razem ze starszymi scenariuszami uruchomiono odbiór dziesięciu
  przypadków na rzeczywistym executable/TLS/WSS/bwrap/LSM; wynik pozostaje
  w toku. Hostowy responder jest kontrolowanym fixture, nie zewnętrznym
  kontem OAuth ani pełnym publicznym admission/SDK consent.
- Dwie wstępne próby Clippy wykryły martwy helper i użycie map_err wyłącznie
  dla efektu ubocznego; poprawki są w kodzie, końcowy Clippy pozostaje otwarty.
  Publiczny endpoint nadal odmawia zdalnego MCP do czasu połączenia i odbioru
  pełnej ścieżki. Formalnie20/68; E5.3 i cały cel E0–E10 pozostają otwarte.

- **Produkcyjny runtime10/10 PASS**,63,18s po build,
  source9ad77c5af9a1e20d730af40b58b1c2e4bbe9934f22991bb243be9096e9637d8c:
  [log](evidence/2026-11-13-mcp-native-proxy/runtime-final.log).
  Wszystkie cztery nowe native MCP cases zaliczone: dwie tury z rzeczywistym
  helperem stdio i Unix proxy, odmowa dostarczenia wyniku po cofnięciu Execute,
  odmowa odpowiedzi20KiB przy wybranym limicie8KiB oraz zatrzymanie po utracie
  lokalnego odbiorcy. Negatywne cases potwierdziły stopped/reservation0, jedno
  wywołanie hosta, brak effects i zatrzymanie potomka setsid z odziedziczonym FD.
  Peak3738112000B/max0/OOM0, scratch90%,172888KiB wolnego.
  [Rekonstrukcja zakresu](evidence/2026-11-13-mcp-native-proxy/source-scope-proof.json)
  potwierdza dokładny delta czterech plików względem backendu799;
  [weryfikator](evidence/2026-11-13-mcp-native-proxy/verify-source-scope.py)
  odtwarza jego hash w prywatnej kopii. To nie jest odbiór publicznego admission
  ani konta OAuth/SDK consent w tym samym zdalnym przepływie. Końcowy Clippy
  dla source9ad77c5a uruchomiony osobno po zakończeniu runtime.

- **Końcowy all-targets Clippy -D warnings PASS**,1m57s,
  peak2225958912B/max0/OOM0:
  [log](evidence/2026-11-13-mcp-native-proxy/clippy-final.log).
  [Manifest](evidence/2026-11-13-mcp-native-proxy/verified-source.json)
  wiąże identyczny source9ad77c5a runtime10 i końcowego Clippy, hash launchera
  oraz osobny, dokładnie odtworzony zakres backendu799 sprzed końcowej
  poprawki dostarczenia Unix. Format Rust, składnia Python fixture,
  git diff --check i122 Markdown/linków/anchors PASS.
  Następna praca: publiczny admission z prywatnymi selections i receipt lookup
  bez nowego dispatch, instalacja hostowej fabryki przed wysłaniem typed open5,
  następnie pełny REST/AHP→zgoda SDK→host HTTP→runner odbiór bez replay.
  E5.2/E5.3 oraz E0–E10 nadal otwarte; formalnie20/68.

## 2026-11-13 — E5.3: publiczny admission MCP i lookup receipt bez kanału, w toku

- Publiczny create przyjmuje istniejące prywatne mcp_connections dla zdalnego
  targetu. Service sprawdza zgodność AppState, politykę użytkownika/urządzenia,
  profil, własne konto i rzeczywisty transport5 przed nowym admission.
  Aktor instaluje hostowy broker przed typed open5, w oryginalnym deadline
  lease. Błąd/anulowanie instalacji przechodzi wspólne negative cleanup.
- Startup jest własnością rzeczywistego joba admission po commit, niezależnie
  od HTTP waitera. Przekazywany AppState ma odłączony remote_sessions OnceLock;
  fabryka zachowuje tę samą ochronę przed cyklem własności.
- SDK ma read-only admission_lookup_checked: callback hosta uruchamia się
  po aktualnej autoryzacji właściciela/projektu i nie uzyskuje runtime lease.
  Store odtwarza fingerprint z aktualnego profilu, bindingu, grantów i prywatnych
  pinów kont; zapisane limity służą wyłącznie dopasowaniu historycznego zamiaru.
  Odczyt receipt nie potrzebuje żywego runnera ani transportu.
- Dodano test SDK i test Store: retry po drop transportu, brak publicznych
  runtime/authority, odmowa innego profilu, rewizji konta, braku selections
  i odłączonego konta. Zapis pozostaje pojedynczy. Pierwsza kompilacja wykryła
  błędne odwołanie testu do prywatnych pól Session i nieistniejącą nazwę disable;
  poprawiono na publiczną serializację oraz istniejący CredentialJournal.disable:
  [log](evidence/2026-11-13-mcp-public-admission/server-first-compile-failure.log).
- Nowy produkcyjny case łączy REST, AHP approval, hostową fabrykę, checked
  OAuth/vault z kontrolowanym dostawcą HTTP, TLS/WSS i rzeczywisty helper
  runnera w bwrap/LSM. Sprawdza dwie tury, ACK faults i offline REST retry.
  Wynik tego case pozostaje niezaliczony do czasu rzeczywistego uruchomienia.
  Wczesny runtime build przerwano po wykryciu skopiowanego, nieprawidłowego
  importu fixture, zanim dotarł do serwera; snapshot z poprawką uruchomiono
  ponownie. To świadoma korekta znanego kodu, nie ponowienie po observation timeout.
- Cały E5.3 i cel E0–E10 pozostają otwarte; formalnie20/68. Bez realnych
  kont/usług/tailnetu. Odbiór nowych zmian w toku.

- Pierwszy pełny runtime wykonał10 starych cases PASS, ale nowy product MCP
  case odrzucił pierwszy AHP pendingMessageSet: runtime unavailable.
  [Log](evidence/2026-11-13-mcp-public-admission/runtime-startup-queue-race.log)
  ujawnił okno między commit admission i instalacją fabryki. Queue cut
  dostał obsługę pending MCP z oryginalnym żywym Actor/SDK lease/transportem:
  lokalny, świeżo walidowany scope służy tylko sprawdzeniu kolejki, nie jest
  instalowany na Actor. Warunek wymaga pending, dispatched:false i braku
  native receipt. Wykonanie nadal wymaga proof z udanej instalacji fabryki.
  Usunięto nieużywany testowy wrapper create. Poprawiona wersja runtime
  została uruchomiona ponownie; nie zaliczono case na podstawie samej poprawki.
  UI nadal blokuje remote MCP; wymaga osobnego flag/wyboru kont i odbioru.

- Drugi pełny runtime przejął pierwszy prompt, ale product case nie uzyskał
  zgody SDK po celowo zgubionym TurnAccepted:
  [log](evidence/2026-11-13-mcp-public-admission/runtime-lost-turn-receipt-deadlock.log).
  Stary v4 odzyskiwał native receipt ze zdarzenia tekstowego, lecz agent MCP
  potrzebuje zgody zanim wyemituje tekst. Dodano niezależne powiadomienie
  TurnAccepted dla v5 z aktualnego, walidowanego oryginalnego WorkerTurnLease.
  Outbox ogranicza ponowne wysyłanie samego receipt do500ms, sprawdza jego
  niezmienność i limit4096; nie wysyła ponownie Submit ani narzędzia.
  Dla lease bez brokera zachowano dotychczasowy outbox. Fault observer liczy
  także rzeczywisty open5; transportowy fixture koreluje powtórzone receipts
  z przyjętą turą. Ponowny odbiór11 cases w toku.
- Dodano osobny test pending queue: przyjęcie kolejki nie instaluje scope
  dispatch na Actor, próba open nadal odmawia, a disable konta odbiera także
  kolejkę. Ten test zostanie wykonany w końcowej regresji backendu.

- Następny runtime doszedł do rzeczywistego SDK pending-confirmation mimo
  utraty TurnAccepted. AHP odrzuciło approval przez wcześniejszą blanket
  blokadę Remote tool approvals are unavailable:
  [log](evidence/2026-11-13-mcp-public-admission/runtime-ahp-approval-gate.log).
  Dodano Service.approvals_allowed: wymaga aktualnego queue cut i scope
  z udanej instalacji fabryki. Pending queue proof ani zwykłe zdalne ACP
  nie włączają zgód. AHP używa tej kontroli przy action i w approveTools meta;
  SDK nadal weryfikuje dokładny tool/turn/option oraz jednorazową decyzję,
  a hostowy broker sprawdza ją ponownie przed HTTP. Odbiór11 cases ponowiony.

- AHP przyjęło zgodę, ale helper runtime wymagał identycznego echa klienta:
  [log](evidence/2026-11-13-mcp-public-admission/runtime-canonical-consent-echo.log).
  Produkcyjny SDK dodał poprawne confirmed:user-action i meta human-one-use,
  humanApproved oraz permissionNonce. Helper sprawdza teraz dokładnie cały
  canonical action wraz z tymi polami i nonce z wybranej opcji; nie usuwa ich
  i nie zmienia autoryzacji. Kolejny runtime11 w toku, bez zmiany produkcyjnego
  kodu od próby z canonical echo.

- **Produkcyjny runtime11/11 PASS**,71,99s po build,
  source8e0a15784fa3b2df52d8294e692a420cab784a20ea0aa438c484ae231142126c:
  [log](evidence/2026-11-13-mcp-public-admission/runtime-final.log).
  Nowy pełny product case wykonał REST create i dwie tury przez rzeczywisty
  AHP dispatch/one-use approval, hostowy Common Broker i checked OAuth/vault
  z kontrolowanym dostawcą HTTP, TLS/WSS/reverse proof, native session/turn
  leases, sandbox i helper stdio/Unix. Dokładnie1 open/session-new,2 Submit,
  2 HTTP tools/call i2 efekty; celowa utrata TurnAccepted, text/ACK i replay
  gap zaliczone. Source stop/reservation0/ACK5 i zatrzymanie potomka z FD
  potwierdzone. Retry REST po zamknięciu kanału dał202/replayed:true/ten sam ID.
  Peak3702333440B/max0/OOM0, scratch90%,171576KiB wolnego.
  To kontrolowany HTTP OAuth/MCP i browser AHP client w teście, nie realne
  konto dostawcy, ręczny użytkownik ani renderowany Chromium UI.
  Clippy all-targets uruchomiony po zakończeniu runtime; pełny backend
  z trzema nowymi testami nadal wymaga wykonania.

- **Końcowy all-targets Clippy -D warnings PASS** na tym samym source8e0a1578
  co runtime11: [log](evidence/2026-11-13-mcp-public-admission/clippy-final.log).
  Pełny backend uruchomiony po zakończeniu Clippy, bez nakładania ciężkich
  buildów w4GiB VM. Nie zmieniano UI ani crate protokołu; ich osobnych suites
  nie powtarzano. Trzy nowe testy SDK/Store/pending queue oczekują wykonania
  w tej końcowej regresji.

- **Pełny backend802 PASS**:662 relay +1 identity +75 SDK +27 terminal
  +37 usage,40 osobnych acceptance pominiętych:
  [log](evidence/2026-11-13-mcp-public-admission/server-final.log).
  Wszystkie trzy nowe testy wykonane. Peak3676917760B/max0/OOM0,
  scratch86%,232012KiB wolnego.
  [Manifest](evidence/2026-11-13-mcp-public-admission/verified-source.json)
  wiąże identyczny source8e0a1578 backendu802, runtime11 i Clippy oraz hash
  launchera. Niezależny snapshot aktualnego drzewa potwierdził ten sam hash.
  Rustfmt17 zmienionych plików, git diff --check i122 Markdown/linków/anchors
  PASS. Nie oznaczono E5.3 ani celu E0–E10 jako zakończonych.
  Następne przekroje: UI wyboru kont dla zdalnego MCP z prawdziwym odbiorem
  browsera; pełne deny/lost-HTTP-reply/revoke cases tej połączonej ścieżki;
  MRTR/revoke/GC i pozostałe scoped secrets/A2A oraz dalsze E6–E10.
  Realne konta/usługi/tailnet pozostają osobną bramką zgody operatora.

## 2026-11-13 — E5.3: zdalny MCP w formularzu i Chromium, w toku

- Katalog profili zwraca oddzielny remote_mcp_sessions_supported: wymaga
  rzeczywistego Service, opt-in brokera kontrolera i skonfigurowanego dostawcy.
  Flaga opisuje obsługę po stronie kontrolera, nie zastępuje świeżego admission
  ani polityki wybranego runnera. Local MCP pozostaje osobną możliwością.
- Formularz pobiera prywatne połączenia także dla zdalnego targetu, wymaga
  jawnego konta dla każdego dostawcy i czyści wybór przy zmianie celu.
  Nie zapisuje tokenów ani kont w profilu agenta. Klient retry zachowuje
  oryginalne selections razem z profilem/bindingiem i Idempotency-Key.
- **Frontend142 PASS**: [log](evidence/2026-11-13-remote-mcp-ui/frontend-before-remote-client-fix.log).
  Dodany test sprawdza typowaną, jawną flagę remote niezależną od local.
  **TypeScript PASS**: [log](evidence/2026-11-13-remote-mcp-ui/typecheck-before-remote-client-fix.log).
  Pierwsza próba typecheck nie znalazła domyślnego web/node_modules; użyto
  istniejącego readonly target/web-checks/node-deps/node_modules, bez instalacji.
- Launcher dopuszcza runtime-browser dla dokładnego product MCP case obok
  dotychczasowego journal case;18 testów launchera PASS. Zachowano CPU/RAM,
  scratch, readonly dependency caches, PID256 dla Chromium oraz domyślny
  brak browsera. Nie rozszerzono flagi na dowolny filter.
- Dodano Node/Chromium odbiór: wybór remote przy wyłączonym local MCP, obowiązkowe
  konto/reset przy zmianie celu, utrata prawdziwej REST odpowiedzi i retry
  identycznego zamiaru, nowa sesja, prompt i jednorazowa zgoda z UI, desktop/
  mobile screenshots i brak przepełnienia poziomego. Rust weryfikuje później
  actual source stop/ACK, liczbę sesji/tur/efektów i zatrzymanie potomka z FD.
  Nowy case został uruchomiony na produkcyjnym runtime; wynik pozostaje otwarty.
  Konto zostało zainstalowane kontrolowanym OAuth flow fixture, nie przez
  realne konto dostawcy ani nowy browser login flow.
- E5.3 i cały cel E0–E10 pozostają otwarte, formalnie20/68. Negatywne
  cases pełnej ścieżki oraz MRTR/revoke/GC/scoped secrets/A2A i E6–E10
  nadal wymagają pracy. Bez realnych kont/usług/tailnetu.

- Pierwszy Chromium case zatrzymała asercja fixture, która zakładała null
  configured_harness. Katalog poprawnie podaje domyślne opencode, mimo
  wyłączonego lokalnego MCP/runtime:
  [log](evidence/2026-11-13-remote-mcp-ui/runtime-default-harness-fixture-failure.log).
  Usunięto założenie o braku domyślnej nazwy harnessu. Nadal sprawdzane są
  local MCP:false, remote MCP:true i rzeczywisty formularz/wykonanie.
  Kod produkcyjny nie został zmieniony z powodu tej asercji; ponowny odbiór
  runtime+Chromium jest w toku. Pierwsza kompilacja bez błędów, OOM0.

- Chromium przeszedł wybór konta/reset celu, ale ujawnił stary zakaz w
  SessionCreationClient: remote target z mcp_connections był odrzucany
  przed REST. [Log](evidence/2026-11-13-remote-mcp-ui/runtime-client-remote-mcp-guard.log).
  Usunięto ten zakaz, zachowując kontrolę profilu, binding revision, providerów
  i prywatnych connection/revision. Test zamrożenia niepewnego zamiaru obejmuje
  teraz local i remote: mutacja konta/bindingu po begin nie zmienia retry.
  **Frontend143 PASS i TypeScript PASS** po poprawce; końcowe logi wyżej,
  poprzednie142/typecheck zachowane jako before-remote-client-fix.
  Produkcyjny runtime+Chromium uruchomiono ponownie.

- Następny Chromium case przeszedł REST/retry i dotarł do rzeczywistej zgody MCP,
  ale wykrył poziome przepełnienie przy390px:
  [log](evidence/2026-11-13-remote-mcp-ui/runtime-mobile-overflow.log).
  Formularz dostał minmax(0,1fr) dla label/grid, width100%/min-width0 dla select
  i min-width0 dla fieldset. Nie ukryto poziomego overflow na body. Asercja
  mobilna została zachowana i wzbogacona o diagnostykę elementów poza viewportem.
  Ponowny runtime+Chromium w toku.
- Wymagany runtime-sandbox w CI uruchamia teraz pełny odbiór z runtime-browser
  i jawnymi readonly cache dependencies apps/web oraz apps/web/e2e. npm ci
  --ignore-scripts przygotowuje oba locked trees. Nie zwiększono900s/30min,
  RAM ani scratch i nie osłabiono AppArmor/seccomp/network. Kontrakt CI
  [PASS](evidence/2026-11-13-remote-mcp-ui/ci-contract-final.log) oraz actionlint
  PASS (pusty log). To lokalna walidacja workflow, nie wykonanie GitHub Actions.

- Runtime+Chromium po naprawie szerokości **PASS**,15,22s po build:
  [log przed poprawką komunikatu](evidence/2026-11-13-remote-mcp-ui/before-clear-error-runtime-final.log).
  Source potwierdził2 sesje,3 tury/efekty/HTTP calls, reservation0 i sumę ACK8,
  bez drugiej sesji po retry browsera. Desktop/mobile PNG obejrzane.
  Kontrola wizualna znalazła stary alert wyniku nieznanego po udanym retry,
  mimo prawidłowego przyjęcia sesji. Profiles czyści teraz ten lokalny błąd
  po przejściu admission do accepted; Chromium sprawdza zniknięcie alertu.
  Zrzuty sprzed poprawki zachowano jako before-clear-error-desktop/mobile.png.
  Nowe zrzuty są robione po scrollTo(0,0), żeby fixed sidebar/skip link nie
  były pozornie przesunięte przez pełnostronicowy screenshot po autoscrollu.
  Kolejny runtime+Chromium uruchomiony; końcowy odbiór UI pozostaje w toku.

- Chromium potwierdził zniknięcie starego alertu i wykonał narzędzie, lecz finalny
  source stop/ACK drugiej sesji przekroczył15s:
  [log](evidence/2026-11-13-remote-mcp-ui/runtime-stop-ack-race.log).
  Nie przedłużono timeoutu ani nie usunięto warunku ACK. Kontrola kodu wykazała
  ścieżkę reconcile-before-stream: zapis StopRecord zwalnia rezerwację i oznacza
  stopped przed ostatnimi events, po czym wcześniejszy kod odrzucał te events
  i usuwał sesję z cleanup hints.
- Dodano kończenie łańcucha po takim stopie: tylko do dokładnego final_sequence/
  digest/reason zapisanego zweryfikowanego StopRecord. Late text przechowuje
  wyłącznie digest, nie treść; brak SDK projection i dodatniej authority.
  ACK nie jest podnoszony dla niezweryfikowanego prefixu; dopiero poprawny
  końcowy stop odblokowuje ACK. Cleanup zachowuje hints do wypełnienia łańcucha.
  Test SQL deterministycznie ustawia StopRecord przed text/terminal/stop,
  sprawdza brak projection, Unknown bez zamiany na Completed, odmowę złego
  końca i events poza anchorem oraz faktyczne potwierdzenie ACK końca.
  Rozszerzono diagnostykę timeoutu o liczniki obu baz, bez treści i sekretów.
- Launcher dopuszcza też dokładny remote_ prefix, który uruchamia oba
  produkcyjne browser branches i remote regresje. Nadal odmawia filtrom
  pozbawionym browser acceptance;18 unit tests PASS. Po SQL regresji
  planowany jest wspólny runtime11 z oboma Chromium branches.

- **Pełny backend803 PASS**:663 relay +1 identity +75 SDK +27 terminal
  +37 usage,40 osobnych acceptance pominiętych:
  [log](evidence/2026-11-13-remote-mcp-ui/server-final.log).
  Deterministyczna regresja reconcile-before-stream wykonana.
  Peak3708997632B/max0/OOM0, scratch86%,231652KiB wolnego.
  Wspólny runtime11 z opt-in obu Chromium branches uruchomiony po backendzie.

- **Wspólny runtime11/11 + oba produkcyjne Chromium branches PASS**,85,75s
  po build: [log](evidence/2026-11-13-remote-mcp-ui/runtime-final.log).
  Nowy MCP UI case i dotychczasowy legacy Remote UI case wykonały prawdziwe
  REST/AHP/WSS/ACP/stdio/sandbox; pozostałe remote restart/timeout/quota/revoke
  także PASS. MCP:2 sesje/3 tury/3 HTTP calls/3 efekty, ACK8, reservation0,
  stop z potomkiem FD i brak duplikatu po utracie REST. Alert nieznanego
  wyniku zniknął po accepted; desktop/mobile bez poziomego przepełnienia.
  Zrzuty [desktop](evidence/2026-11-13-remote-mcp-ui/desktop.png) i
  [mobile](evidence/2026-11-13-remote-mcp-ui/mobile.png) zapisane; wersje
  po poprawce alertu i szerokości zostały obejrzane.
  Peak3758096384B/max333/OOM0, scratch90%,168956KiB wolnego. Nie podnoszono
  zasobów; max333 oznacza reclaim/hit limitu, nie OOM.
  Końcowy Clippy i frontend/typecheck dla bieżącego drzewa pozostają w toku.

- **Końcowy Clippy all-targets -D warnings PASS**,1m59s,
  peak2247417856B/max0/OOM0:
  [log](evidence/2026-11-13-remote-mcp-ui/clippy-final.log).
  Końcowy frontend143 i TypeScript PASS po ostatniej poprawce alertu/CSS:
  [frontend](evidence/2026-11-13-remote-mcp-ui/frontend-final.log),
  [typy](evidence/2026-11-13-remote-mcp-ui/typecheck-final.log).
  [Manifest](evidence/2026-11-13-remote-mcp-ui/verified-source.json) wiąże
  identyczny pełny source00887f71 backendu803, runtime11+obu Chromium branches
  i Clippy; frontend/typecheck mają odrębne, mniejsze zbiory wejściowe snapshotu.
  Dodano hashe PNG, launchera, workflow i ich testów oraz wyniki18/14/actionlint.
  Node syntax, rustfmt, git diff --check i122 Markdown/linków/anchors PASS.
  E5.3 i cały cel E0–E10 nadal otwarte, formalnie20/68. Następne: full-pipeline
  deny/lost HTTP response/revoke, następnie MRTR/revoke/GC/scoped secrets/A2A
  i dalsze etapy. Zgoda na realne konta/usługi/tailnet pozostaje osobną bramką.

- Niezależny końcowy snapshot aktualnego drzewa potwierdził source00887f71;
  końcowy mobile PNG ponownie obejrzany. Manifest i122 Markdown/linków/anchors
  zweryfikowane. Brak otwartych procesów weryfikacji tego przekroju.


### 2026-11-13 — połączone negatywne scenariusze zdalnego MCP — w realizacji

- Dodano trzy przypadki produkcyjnej ścieżki REST/AHP/WSS/ACP/stdio: odmowa
  jednorazowej zgody, utrata odpowiedzi HTTP po przyjęciu tools/call oraz
  owner-only disconnect konta przez REST podczas wstrzymanego HTTP narzędzia.
- Odmowa ma pozwolić na dalszą turę tej samej sesji bez zewnętrznego wywołania.
  Niepewne operacje mają zakończyć turę Unknown, zatrzymać proces i potomka z FD,
  potwierdzić końcowy stop/ACK i zwolnić reservation bez ponowienia Submit/call.
  Ponowna dostawa tej samej zgody i tego samego zamiaru REST także jest sprawdzana.
- Uzupełniono brakujący moduł mcp_failure. Uruchomiono cztery scenariusze
  remote_mcp_product_ w izolowanym Linux runtime (trzy nowe i dotychczasowy
  pozytywny). [Pierwszy log](evidence/2026-11-13-mcp-joined-failures/runtime-first.log).
  Wynik jeszcze nieodebrany; ostatnia pełna akceptacja pozostaje source00887f71.
  Bez realnych kont/usług/tailnetu. Formalny postęp nadal20/68.

- Pierwszy runtime:3/4 PASS (lost HTTP, disconnect podczas HTTP i pełny pozytywny
  pipeline). Deny zatrzymała błędna asercja testowego session_dispatch: canonical
  odmowa ma reason:denied, podczas gdy helper zakładał confirmed:user-action
  dla obu decyzji. Porównano z SDK permissions.rs i rozdzielono oczekiwane
  pola, zachowując pełną kontrolę nonce/humanApproved/evidence i całej akcji.
  Brak zmiany kodu produkcyjnego. Pierwszy build i runtime: OOM0.
- Uruchomiono wspólną regresję remote_ (14 scenariuszy) z obiema gałęziami
  produkcyjnego Chromium. [Log](evidence/2026-11-13-mcp-joined-failures/runtime-final.log).
  Wynik w toku, bez zwiększania limitów RAM/scratch/czasu ani osłabiania sandboxa.

- **Runtime14/14 z oboma Chromium branches PASS**,110,76s po build,
  sourcea5e0899a: odmowa, lost HTTP reply i disconnect podczas HTTP oraz
  dotychczasowe transport/restart/timeout/quota/revocation/UI regresje.
  Lost/revoke:1 sesja/1 tura/1 HTTP call, Unknown, stop/ACK2, reservation0,
  brak późniejszego wyniku w projekcji, brak checkout effects, zatrzymany
  potomek z FD, replay zgody i zamiaru REST bez ponownego Submit/tools/call.
  Deny:1 proces ACP/2 tury,0 HTTP calls/0 checkout effects i zachowany kontekst.
  Poprawny MCP UI oraz legacy UI także PASS. PNG desktop/mobile zapisano
  w evidence/2026-11-13-mcp-joined-failures. Peak3758096384B/max189/OOM0,
  scratch90%,168896KiB wolnego. Clippy dla tego samego source w toku.
  Nie wykonywano ponownie niezmienionego frontend143 ani pełnego backend803;
  ich ostatni wynik odnosi się do wcześniejszego snapshotu00887f71.

- **Końcowy Clippy all-targets -D warnings PASS**,1m57s,
  peak2235305984B/max0/OOM0. Ten sam sourcea5e0899a co runtime14; niezależny
  snapshot bieżącego drzewa potwierdził identyczny digest. [Manifest](evidence/2026-11-13-mcp-joined-failures/verified-source.json)
  wiąże oba logi i PNG. [Desktop](evidence/2026-11-13-mcp-joined-failures/desktop.png)
  i [mobile](evidence/2026-11-13-mcp-joined-failures/mobile.png) obejrzane,
  bez przepełnienia poziomego i starego alertu po accepted.
  Rustfmt, składnia Python fixture, git diff --check i122 dokumenty/linki PASS.
  Poprawiono nieaktualny zapis E5.2 o niepodłączonym brokerze; obecny zakres
  jest odebrany w E5.3. MRTR UI/runtime, trwałe provider revoke/GC, ogólne scoped
  secrets, A2A oraz dalsze etapy pozostają otwarte. Formalnie nadal20/68.


### 2026-11-13 — trwały dziennik formularzy MCP/AHP — w realizacji

- Rozpoczęto połączenie MRTR form z runtime/UI od warstwy trwałego żądania.
  Nowa migracja41 dodaje interactive_mcp_inputs, przypięte do oryginalnego
  consumed MCP permit. Handle pozostaje host-only/non-Clone/non-Deserialize;
  requestState nie trafia do dziennika ani AHP. Kontynuacja HTTP nadal wymaga
  żywego oryginalnego klienta/Pending i świeżych kontroli konta/authority.
- SDK rejestruje standardowe chat/inputRequested i jednorazowe
  chat/inputCompleted. Accept wymaga jawnego czystego walidatora przypiętego
  schematu; zwykły dispatcher odmawia accept. Decline/cancel nie przyjmują
  content. Dedupe/first-wins i transakcja obejmują decyzję i canonical AHP.
  Parent invalidation/recovery unieważnia formularze; już odebrana odpowiedź
  nie może być użyta ponownie. Adaptory HTTP/UI jeszcze tego nie reklamują.
- Pierwszy [SDK78 PASS](evidence/2026-11-13-mcp-input-journal/sdk-first.log)
  obejmuje trzy nowe testy: reopen/dedupe/consume/recovery, decline/cancel/
  competing answer oraz zmienioną invocation, draft/unknown/type/schema failure
  i przerwanie. Po tym odbiorze dodano jeszcze domknięcie oczekujących inputs
  przy definitywnym wyniku rodzica i dwa testy migracji40→41/collision rollback.
  Pełny backend dla zaktualizowanego źródła uruchomiony:
  [log](evidence/2026-11-13-mcp-input-journal/server-final.log).
- Kolejne konieczne części: mapowanie FormSchema↔AHP, rzeczywista walidacja
  serwera, formularz UI i konwersja odpowiedzi do MRTR oraz odbiór całego
  przepływu z kontrolowanym dostawcą. E5.2/E5.3 i formalne20/68 bez zmian.

- **Pełny backend808 PASS**:665 relay +1 identity +78 SDK +27 terminal +37 usage,
 43 osobne acceptance pominięte. Nowe testy migracji40→41/preserved session
 i collision rollback PASS. OOM0, peak3688968192B, scratch86%,230544KiB wolnego.
 Regresja runtime14 z oboma produkcyjnymi Chromium branches uruchomiona po
 zakończeniu backendu, dla tego samego snapshotu. Jest to odbiór dotychczasowych
 sesji po migracji, nie dowód formularza w przeglądarce ani kontynuacji MRTR.

- Pierwszy runtime po migracji:13/14 PASS. Pozytywny MCP pipeline przerwał
  kontroler z Conflict przed ukończeniem tury:
  [log](evidence/2026-11-13-mcp-input-journal/runtime-first-conflict.log).
  Nie oznaczono regresji jako odebranej. Dodano test-only diagnostykę etapu
  positive cut i kodu odrzucenia RPC, bez parametrów/promptów/sekretów; uruchomiono
  dokładny pipeline z Chromium. [Diagnostyka](evidence/2026-11-13-mcp-input-journal/runtime-conflict-diagnostic.log).
  Docelowy runtime-final zostanie zapisany dopiero po wyjaśnieniu problemu.

- Dokładny pipeline z diagnostyką PASS21,30s z Chromium. To nie rozstrzyga
  sporadycznego Conflict. Dodano osobny kontrolowany case renew_terminal_race:
  prawdziwy provider HTTP wstrzymuje odpowiedź, a test dopiero po przygotowaniu
  RenewTurn zwalnia HTTP i czeka na rzeczywisty źródłowy TurnCompleted.
  Następnie wysyła oryginalny RenewTurn, bez podmiany receipt/lease/events.
  Celem jest deterministyczne sprawdzenie obsługi już zakończonej tury, a nie
  powtarzanie zielonego testu do skutku. [Przebieg przed poprawką](evidence/2026-11-13-mcp-input-journal/runtime-renew-race-before-fix.log)
  uruchomiony; wynik w toku. Dotychczasowe packet loss/replay/ACK faults zachowano.

- Wymuszony race przed poprawką **FAIL4,77s**: dokładne RPC renew_turn odrzucone
  kodem StaleAttempt, potem controller Conflict i przerwana tura. Jest to
  deterministyczny dowód wyścigu terminal-vs-renew, nie skutek zwiększenia limitów.
- Kontroler rozróżnia teraz tylko ten przypadek: po poprawnie skorelowanym
  StaleAttempt dla RenewTurn odczytuje ograniczony, uwierzytelniony pierwotny
  stream. Następnie świeży Describe/authority/SQL cut wymaga canonical terminal
  dokładnie tej tury. Brak terminala lub aktualnej authority nadal przerywa;
  proponowany licznik renewal dla nadal aktywnej tury wycofuje się w transakcji.
  Nie wysyła drugiego RenewTurn, Submit ani narzędzia i nie przedłuża budżetu.
  Runner nadal odrzuca renewal zakończonej tury.
- Uruchomiono pełny runtime15 z oboma Chromium branches po poprawce:
  [log](evidence/2026-11-13-mcp-input-journal/runtime-final.log). Wynik w toku;
  poprzedni backend808 jest sprzed tej dodatkowej poprawki kontrolera.

- **Runtime15/15 z oboma Chromium branches PASS**. Wymuszony renew/terminal
  race wykonał prawdziwe HTTP, rzeczywisty source terminal, odrzucony RenewTurn
  i uzgodnienie canonical terminal bez ponowienia Submit/narzędzia. Dwie tury,
  zachowany kontekst, dwa efekty i końcowy source ACK5/reservation0. Przeszły
  też wcześniejszy pozytywny MCP UI i wszystkie denial/lost HTTP/disconnect/
  restart/timeout/quota/revoke scenariusze. To regresja istniejącego MCP,
  nie odbiór formularza MRTR w UI. Pełny backend po dodatkowej poprawce
  kontrolera uruchomiono ponownie; poprzedni808 zachowano w
  [logu sprzed poprawki](evidence/2026-11-13-mcp-input-journal/server-before-renew-fix.log).

- **Końcowy backend808 PASS po poprawce kontrolera**:665 relay +1 identity
  +78 SDK +27 terminal +37 usage,44 acceptance osobno pominięte. Relay146,83s,
  OOM0/peak3758096384B/max21, scratch86%,230512KiB wolnego. Runtime15 z oboma
  Chromium branches PASS123,59s, OOM0/peak3758096384B/max216, scratch90%,
 167976KiB wolnego. Nie zwiększono limitów ani nie osłabiono warunków testów.
- **Clippy all-targets -D warnings PASS**,1m57s,
  peak2271641600B/max0/OOM0. [Manifest](evidence/2026-11-13-mcp-input-journal/verified-source.json)
  wiąże backend/runtime/Clippy z identycznym sourcecde81b78, zrzuty PNG i
  oczekiwany czerwony test przed naprawą. Niezależny bieżący snapshot potwierdził
  identyczny digest. Rustfmt, git diff --check i122 dokumenty/linki/anchors PASS.
  Mobile PNG obejrzany; oba viewporty sprawdzane w rzeczywistym Chromium.
  Nie zmieniano frontendu ani nie powtarzano jego wcześniejszych143 testów/tsc.
- Odbiór dotyczy trwałej warstwy formularzy i poprawki terminal-vs-renew,
  nie pełnego MRTR form flow. Następne prace: mapowanie FormSchema↔AHP,
  exact FormSchema walidator w transakcji publicznego dispatch, formularz
  użytkownika i MRTR begin/continue w oryginalnym żywym kliencie z kontrolą
  urządzenia odpowiadającego użytkownika przed/po HTTP. E5.2/E5.3 oraz cel
  E0–E10 pozostają aktywne i otwarte, formalnie20/68. Realne konta/usługi/tailnet
  nadal wymagają osobnego odbioru. Brak otwartych procesów weryfikacji przekroju.


### 2026-11-13 — formularze MRTR w publicznym runtime/UI — w realizacji

- Dodano mapowanie wszystkich płaskich typów FormSchema na standardowe pytania
  AHP, provider/origin/tool w prezentacji, jawny exact FormSchema::validate
  w transakcji publicznego inputCompleted oraz kontrolę installed remote factory.
- Broker używa begin_call/continue_with_responses w jednym uwierzytelnionym
  kliencie i oryginalnym Pending. requestState pozostaje host-only. Nie zmienia
  arguments/permit/deadline i sprawdza urządzenia odpowiadające na wszystkie
  dotychczasowe formularze przed kontynuacją i po HTTP. Form support w runtime
  jest jawny, URL support nadal wyłączony. Revoke/error nie daje automatycznego retry.
- UI dodaje tekst/liczby/integer/boolean/select/multi-select oraz jawne
  accept/decline/cancel, z domyślnymi wartościami wyłącznie do przeglądu.
  Zachowuje0/false/empty enum/empty required array i bezpieczne nazwy właściwości,
  w tym __proto__. Outbox zamraża całe odpowiedzi. Dodano trace AHP input/draft/
  rounds do niezmienionego pinned SDK oracle: pierwszy SDK77/78 oczekiwanie FAIL,
  zaimportowano wygenerowany rzeczywisty wzorzec, a nie stany ręcznie wymyślone.
  [Log oracle](evidence/2026-11-13-mcp-form-flow/sdk-oracle-first.log).
- Pierwszy frontend146/147: nowy test użył starego helpera echo wybierającego
  pierwszy dispatch zamiast kolejnej decyzji. Poprawiono tylko ten test na
  dokładny origin drugiego zamiaru. Typecheck PASS. Kolejny odbiór frontend/
  typów po aktualizacji fixtures pozostaje do wykonania.
- Controlled HTTP provider ma dwie rzeczywiste rundy formularza i odmowę/cancel
  bez content. Dodano trzy produkcyjne runtime cases, API/AHP duplicate response
  oraz warianty Chromium: wszystkie typy/empty choice/0/false, brak autoselect
  defaults, brak requestState w DOM i brak nawigacji do form URI. Pełny backend
  uruchomiono; brak odbioru całego przepływu na tym etapie.
  [Log backendu](evidence/2026-11-13-mcp-form-flow/server-first.log).
  Formalne20/68 i E5.2/E5.3 pozostają otwarte, bez realnych kont/usług/tailnetu.

- Pierwszy pełny backend809 PASS (666 relay +1 identity +78 SDK +27 terminal
  +37 usage,47 acceptance pominięte) obejmuje mapper i wygenerowany SDK oracle.
  Jest sprzed dalszej poprawki obsługi niepoprawnych odpowiedzi formularza.
- Publiczne inputCompleted ma teraz canonical, trwałe odrzucenie dla błędnych
  danych, bez zużycia input i bez zamknięcia WS. Kolejna poprawna odpowiedź
  wymaga nowego clientSeq; stary rejected zamiar pozostaje immutable. SDK testy
  uwzględniają tę regułę zamiast zmieniać treść starego seq. API case wysyła
  ułamkowy integer, a Chromium błędne URI, po czym oba poprawiają dane.
- Dodano kontrolę urządzeń odpowiadających także w Store cut przed zapisaniem
  końcowego wyniku MCP. Nie zmieniono pierwotnej zgody, scope ani deadline.
  Końcowy frontend/typy uruchomione; następne SDK i rzeczywisty runtime.

- Końcowy frontend147 PASS i TypeScript PASS po poprawce helpera oraz obsłudze
  poprawy formularza: [frontend](evidence/2026-11-13-mcp-form-flow/frontend-final.log),
  [typy](evidence/2026-11-13-mcp-form-flow/typecheck-final.log).
  SDK78 PASS po canonical rejection zamiast błędu transportu, razem z oracle
  actual AHP7 traces: [log](evidence/2026-11-13-mcp-form-flow/sdk-final.log).
  Runtime18 z Chromium uruchomiony, w tym trzy nowe pełne form cases:
  [pierwszy log](evidence/2026-11-13-mcp-form-flow/runtime-first.log).

- Pierwszy runtime18:17 PASS, forms accept FAIL na asercji helpera echo:
  standardowy ChatInputNumberAnswerValue jest f64, więc canonical0.0 nie
  porównuje się w serde_json z surowym0. Nie zmieniono danych ani walidacji:
  helper porównuje teraz pełną canonical akcję po typed SDK serde roundtrip,
  a fixture HTTP oczekuje tej reprezentacji. Form decline/cancel z Chromium
  przeszły, podobnie jak wszystkie poprzednie15 scenariuszy. OOM0.
- Dodano forms-revoke: rzeczywisty owner-only disconnect po pojawieniu się
  formularza; wymagane Unknown, canceled input, brak kontynuacji HTTP,1 call,
  stop/ACK2 i reservation0, bez powtórzeń. Pełny runtime19/Chromium uruchomiony:
  [log](evidence/2026-11-13-mcp-form-flow/runtime-final.log).
- Prezentacja AHP może powielić enum value jako label: poprawny MCP schema16KiB
  i message8KiB mogą przekroczyć wcześniejsze24KiB SDK display. Ustawiono jawny
  bound48KiB/4096 nodes, nadal poniżej incoming AHP64KiB, bez zwiększania limitu
  schematu16KiB/content64KiB/deadline/Docker. Nowy SDK test rejestruje taki
  poprawny rozwinięty formularz i sprawdza odrzucenie danych ponad48KiB bez
  częściowego zapisu. Odbiór nowego testu zostanie wykonany w pełnym backendzie.

- **Runtime19/19 z wszystkimi browser branches PASS183,28s**: accept w dwóch
  rundach, poprawa integer/URI po canonical rejection, duplicate input response,
  decline/cancel i forms-revoke oraz wcześniejsze15 regresji. Forms accept:
  API2 operations/4 continuation HTTP, z UI3/6; efekty2/3, source ACK5/8,
  reservation0. Decline/cancel:0 checkout effects, HTTP response bez content,
  API2/2 i z UI3/3. Disconnect podczas input:1 initial call/0 continuation,
  Unknown, canceled input, stop/ACK2, reservation0 i martwy potomek z FD.
  OOM0/peak3758096384B/max40, scratch90%,167172KiB wolnego.
- PNG wyodrębniono per-case: accept/decline/cancel/tool, desktop/mobile.
  Accept desktop i mobile obejrzane, bez poziomego overflow; potwierdzone
  widoczne provider/origin/tool, wartości proponowane i wszystkie decyzje.
- **Końcowy backend810 PASS**:666 relay +1 identity +79 SDK +27 terminal
  +37 usage,48 osobnych acceptance pominiętych. Nowy SDK expanded-enum/display
  test PASS, razem z exact mapping i canonical invalid-input rejection.
  Relay144,87s; OOM0/peak3704070144B/max0, scratch86%,229912KiB wolnego.
  Backend i runtime mają ten sam sourceab27a153. Końcowy Clippy uruchomiony.

- **Końcowy Clippy all-targets -D warnings PASS1m54s**,
  peak2307633152B/max0/OOM0. [Manifest](evidence/2026-11-13-mcp-form-flow/verified-source.json)
  wiąże backend810, runtime19 i Clippy z identycznym sourceab27a153,
  frontend147 i TypeScript z ich właściwymi węższymi snapshotami. Niezależne
  bieżące snapshoty wszystkich trzech zestawów wejść potwierdzają te digests.
  Rustfmt, Node/Python fixture syntax, git diff --check i122 dokumenty/linki PASS.
- [Accept desktop](evidence/2026-11-13-mcp-form-flow/accept-desktop.png) i
  [mobile](evidence/2026-11-13-mcp-form-flow/accept-mobile.png) obejrzane.
  Odrębne PNG dla decline/cancel/tool w tym samym katalogu zachowują kontekst
  testu; nie nadpisano ich jednym obrazem z ostatniego przypadku.
- Odbiór publicznych formularzy form MRTR ukończony w kontrolowanym zdalnym
  Linux runtime. Obejmuje wszystkie płaskie typy i poprawę błędów, bez claimu
  pełnego conformance wszystkich MCP metod/platform ani realnego dostawcy.
  E5.2/E5.3 nadal otwarte: trwałe provider remote revoke/GC, URL elicitation,
  ogólne scoped secrets/kill switch i realne integracje wymagają dalszej pracy;
  potem A2A i pozostałe etapy. Formalne20/68 pozostaje bez zmiany. Cel E0–E10
  aktywny; brak otwartych procesów weryfikacji tego przekroju.

## 2026-11-13 — trwała tożsamość koperty przed zapisem sekretu

- CredentialJournal wymaga prepare_write przed SecretStore.set. Każdy adapter
  (Mutex, BrowserJournal, RuntimeJournal i Store) przekazuje tę operację przez
  istniejącą kontrolę uprawnień; nie ma domyślnego no-op. Commit wymaga zgodnego
  przygotowanego hasha, rewizji, konfiguracji, generacji i oryginalnego zamiaru.
- Migracja42 zapisuje metadane przy referencji, zachowując je po błędzie vaulta,
  przegranym CAS i disconnect. Nie zapisuje tokenów w SQL. Historyczne odłączone
  i wycofane koperty bez dowodu pozostają nieznane; migracja nie przypisuje im
  bieżącej rewizji. Staged bez otrzymanej koperty ma znaną tożsamość bez hasha.
- Dodano test kolejności przed sekretowym I/O i odmowy prepare bez zapisu,
  niezmienności koperty, restartu po revision fence oraz atomowej migracji42.
  Istniejący test rzeczywistego HTTP/SecretStore sprawdza teraz przygotowany hash
  przed set i zachowanie metadanych przy wszystkich awariach.
- Pierwszy mcp build zatrzymał się na brakującym imporcie Digest w nowym teście;
  import poprawiony. Pełny backend uruchomiony; odbiór tego przyrostu jeszcze
  nie jest zaliczony. Logi w evidence/2026-11-13-mcp-write-ledger/.
- To fundament trwałego provider revoke/GC, a nie ich ukończenie. E5.2/E5.3,
  formalne20/68 i cały cel E0–E10 pozostają otwarte. Nie wykonywano operacji
  na prawdziwych kontach, keyringu ani tailnecie.

- Pierwszy backend build wykrył niewłaściwy alias Result w nowej metodzie
  RuntimeJournal; poprawiono go na istniejący NetResult. Ponowny pełny backend
  **812 PASS**:668 relay +1 identity +79 SDK +27 terminal +37 usage,48 acceptance
  pominiętych. Relay150,22s, kompilacja3m08s; OOM0/peak3728224256B/max0,
  scratch86%,229828KiB wolnego. [Log](evidence/2026-11-13-mcp-write-ledger/server-final.log).
- **MCP/HTTP/vault151 PASS** (114/6/31), wraz z Clippy tych trzech pakietów
  all-targets -D warnings PASS30,32s; OOM0/peak1216577536B/max0.
  [Log](evidence/2026-11-13-mcp-write-ledger/mcp-final.log).
  Oba odbiory mają source2e57e18b. Produkcyjny zdalny MCP pipeline/Chromium
  uruchomiony, Clippy serwera do wykonania; nie są jeszcze zaliczone.

- **Produkcyjny remote MCP pipeline z Chromium PASS15,72s** na tym samym
  source2e57e18b: actual REST/AHP/WSS/native proxy/stdio helper/Linux sandbox
  z kontrolowanym OAuth/MCP i ACP fixture. Jest to jeden wybrany scenariusz;
  w tym przyroście nie uruchamiano ponownie wszystkich19 przypadków poprzedniego
  odbioru formularzy. [Log](evidence/2026-11-13-mcp-write-ledger/runtime-final.log).
  OOM0/peak3758096384B/max375, scratch90%,167072KiB wolnego. Oryginalne
  [desktop](evidence/2026-11-13-mcp-write-ledger/desktop.png) i
  [mobile](evidence/2026-11-13-mcp-write-ledger/mobile.png) obejrzane, bez
  poziomego overflow. Końcowy Clippy serwera uruchomiony.

- **Clippy serwera all-targets -D warnings PASS1m59s**,
  OOM0/peak2281922560B/max0. [Log](evidence/2026-11-13-mcp-write-ledger/clippy-final.log).
  [Manifest](evidence/2026-11-13-mcp-write-ledger/verified-source.json) wiąże
  backend812, MCP/HTTP/vault151 wraz z ich Clippy, produkcyjny remote pipeline
  z Chromium i Clippy serwera z identycznym source2e57e18b. Niezależny snapshot
  bieżącego drzewa potwierdził ten digest. Rustfmt, git diff --check oraz122
  dokumenty/linki PASS. Wszystkie procesy walidacji tego przyrostu zakończone.
- Przyrost jest odebrany jako trwały zapis tożsamości koperty przed sekretnym
  I/O. Następny krok: trwały dziennik odwołania tokenów dostawcy, a następnie
  powtarzalne ujemne sprzątanie zachowujące tombstones dla spóźnionych zapisów.
  Provider revoke/GC i cały E5.2/E5.3 nadal otwarte; formalne20/68 bez zmiany.

## 2026-11-13 — trwałe provider revoke i receipt w REST/UI (odbiór przekroju ukończony)

- Oddzielny RevocationJournal i migracja43: atomowe disabled/revision fence,
  bounded snapshot wszystkich referencji, jednorazowy claim unknown przed I/O,
  kontrola pochodzenia koperty oraz świeżej authority przed/po każdym AS POST.
  Vault adapter i produkcyjny owner-only REST /revoke nie przekazują sekretów
  runnerowi ani przeglądarce. GET ujawnia tylko bezpieczny receipt/stan/liczniki.
- UI rozróżnia lokalne odłączenie, odwołanie u dostawcy i jawny retry na nowej
  rewizji. Zakończony unconfirmed pozwala na nową autoryzację; unfinished po
  przerwaniu/recovery blokuje nowe operacje do osobnego rozstrzygnięcia.
- Dodano HTTP testy dwóch kopert, utraty odpowiedzi, jawnego retry, odwołania
  urządzenia między refresh/access POST, zmiany konfiguracji i utraty odpowiedzi
  na rotację. Store testuje jednorazowe claims, crash/reopen, unknown snapshot,
  migration43 rollback i brak potwierdzenia przy zerze udowodnionych kopert.
- Pierwsze dwa buildy backendu zatrzymały się na konwersji Option<u64> z SQLite
  oraz mutacji Arc<Config> w nowym teście. Poprawiono oba miejsca, bez zaliczenia
  tych uruchomień. Trzeci pełny backend uruchomiony.
- **Frontend150 PASS, TypeScript PASS**; logi w evidence/2026-11-13-mcp-provider-revoke/.
  Runtime browser fixture sprawdzi utratę odpowiedzi /revoke, GET receipt bez
  ponowienia HTTP oraz rzeczywisty helper/sandbox przy DUNLIN_RELAY_E2E_BIN.
  Launcher dopuszcza tylko nowy jawny filtr tego odbioru.
- GC, bezpieczne rozstrzyganie unfinished i prawdziwe AS/keyring/tailnet pozostają
  otwarte; formalne20/68 i cały cel E0–E10 bez zmiany.

- Trzeci backend:672 relay PASS/3 nowe HTTP cases FAIL; wszystkie cztery nowe
  Store revocation/migration/recovery testy PASS. Fixture AS reklamował revoke
  bez wspieranej metody auth publicznego klienta; istniejący guard prawidłowo
  blokował wysłanie tokenów. Dodano jawne revocation auth methods none w obu
  kontrolowanych providerach; guard klienta nie został osłabiony.
- Focused OAuth:38 PASS/1 FAIL/3 ignored. Ostatni test wykrył rzeczywistą potrzebę
  nowej rewizji przy logowaniu po ukończonym revoke: stary CAS nie może odwołać
  nowej autoryzacji ani pomylić jej z historycznym receipt. prepare zwiększa
  teraz rewizję po istnieniu receipt; testy wymagają także odrzucenia starego
  /revoke po rozpoczęciu nowego logowania. UI zachowuje historyczny unconfirmed
  przy nowej rewizji, bez traktowania go jako wyniku nowej autoryzacji.
- Na source3c2fd600 **MCP/HTTP/vault151 i Clippy tych pakietów PASS25,83s**,
  OOM0/peak1296969728B/max0. Ponowny frontend150, TypeScript oraz tooling
  (CI workflows i18 launcher tests) PASS. Produkcyjny Chromium revoke/runtime
  uruchomiony; pełny backend i Clippy serwera pozostają do końcowego odbioru.

- **Backend819 PASS** na source3c2fd600 (675 relay +1 identity +79 SDK +27 terminal
  +37 usage,49 acceptance ignored), w tym HTTP retry i nowa autoryzacja po revoke.
  [Log](evidence/2026-11-13-mcp-provider-revoke/server-before-progress.log).
- **Chromium/runtime PASS7,07s** na source3c2fd600: produkcyjny helper, Linux
  sandbox, AHP tool approval, cztery AS revoke POST dla dwóch kopert i utrata
  odpowiedzi REST po zatwierdzeniu. GET odtwarza confirmed_known_tokens bez
  drugiego /revoke; AS4/authorizations2/tokens2/tools1, bez delete sekretów.
  [Log](evidence/2026-11-13-mcp-provider-revoke/runtime-before-progress.log).
  Oryginalne desktop/mobile obrazy potwierdziły widoczny status odwołania.
- Uzupełniono postęp retry: zachowanie potwierdzeń identycznych kopert, skipping
  już potwierdzonych, least-recently-attempted ordering dla pozostałych i odmowa
  kolejnego revoke na już potwierdzonej nadal disabled rewizji. HTTP loss+retry
  wymaga teraz3+2 POST, nie ponownego wysyłania potwierdzonej koperty. Nowe testy
  sprawdzają reboot, niewzruszalność flags snapshot, nową rejestrację i brak
  zagłodzenia niepodjętych pozycji. **Backend821 PASS** na source537950d6:
  [log](evidence/2026-11-13-mcp-provider-revoke/server-progress.log).
- Dodano bounded audit handling przy pełnych100000 wpisach: negatywne odłączenie
  działa z prywatnym trwałym receipt i bez wzrostu wspólnego audytu. Nowy test
  używa rzeczywistego SQLite in-memory na pełnej pojemności. **Końcowy Clippy
  all-targets -D warnings PASS2m00s** na source063db015, OOM0/peak2321743872B/max0.
  [Log](evidence/2026-11-13-mcp-provider-revoke/clippy-final.log).
  Pełny backend822 oraz ponowny MCP/Chromium tego ostatniego źródła są w toku
  lub do wykonania; nie zostały jeszcze zaliczone.

- **Końcowy backend822 PASS**:678 relay +1 identity +79 SDK +27 terminal +37 usage,
  49 osobnych acceptance ignored. Relay157,06s, kompilacja3m17s,
  OOM0/peak3758096384B/max34, scratch86%,229104KiB wolnego.
  [Log](evidence/2026-11-13-mcp-provider-revoke/server-final.log).
- **Końcowe MCP/HTTP/vault151 i ich Clippy all-targets -D warnings PASS25,83s**,
  OOM0/peak1293910016B/max0.
  [Log](evidence/2026-11-13-mcp-provider-revoke/mcp-final.log).
- **Końcowy Chromium/runtime PASS7,34s**: rzeczywisty produkcyjny helper/bwrap,
  Builder, prywatne konto, AHP approval i1 tools/call, potem4 AS revoke POST,
  utrata odpowiedzi /revoke i GET potwierdzenia bez retry. OOM0/peak3758096384B/
  max304, scratch90%,165932KiB wolnego. To jeden wybrany scenariusz,726 innych
  testów odfiltrowano; nie jest to ponowny odbiór wszystkich remote cases ani
  pełnego MCP conformance.
  [Log](evidence/2026-11-13-mcp-provider-revoke/runtime-final.log).
- [Desktop](evidence/2026-11-13-mcp-provider-revoke/revoke-desktop.png) i
  [mobile](evidence/2026-11-13-mcp-provider-revoke/revoke-mobile.png) są oryginalnymi
  PNG z końcowego źródła; obejrzane, szerokości1440/390. Potwierdzają widoczny
  status odłączonego konta i potwierdzenie zapisanych tokenów, bez obietnicy
  cofnięcia całej zgody AS. Testy nie usuwają sekretów.
- [Manifest](evidence/2026-11-13-mcp-provider-revoke/verified-source.json) wiąże
  backend822, MCP151, Chromium1 i Clippy serwera z identycznym source063db015.
  Frontend150, TypeScript i CI/launcher tooling14+18 mają właściwe węższe
  snapshoty. Niezależne bieżące snapshoty potwierdziły wszystkie cztery zestawy
  wejść. Rustfmt wybranych plików, Node/Python syntax, git diff --check oraz122
  dokumenty/linki PASS. Istniejący CI runtime bez filtra obejmuje nowy case;
  nie uruchamiano nowego GitHub Actions workflow.
- Odbiór tego przyrostu jest ukończony w kontrolowanych fixtures Linux/Chromium.
  Brak prawdziwego AS/konta/keyringu/tailnetu i native macOS acceptance. Wszystkie
  procesy walidacji tego przyrostu zakończone. GC z zachowaniem tombstones,
  rozstrzyganie unfinished, URL elicitation, ogólne scoped secrets/A2A i dalsze
  etapy pozostają otwarte. Formalne20/68 i cały aktywny cel E0–E10 bez zmiany.

## 2026-11-13 — lokalne sprzątanie potwierdzonych tokenów (odbiór fixtures ukończony)

- CleanupJournal/vault adapter oraz migracja44 dodają osobny trwały dziennik
  przebiegów. Owner-only REST /cleanup i UI mają osobny licznik CAS, fresh cookie/
  device/IdP/membership guards i ochronę aktualnej/staged generacji. Nie ma AS
  I/O ani plaintext reads. Dwa osobne blocking slots pozostają zajęte do końca I/O.
- Każdy przebieg wraca także do poprzednich delete ACK, aby usuwać spóźnione
  zapisy. Tombstones/ref metadata pozostają w SQL. unfinished delete można
  jawnie zastąpić z nowym nonce; stare rozpoczęte I/O dotyczy wyłącznie starego
  UUID. Cleanup nie blokuje nowej autoryzacji i nie usuwa jej generacji.
- Dodano Store testy restartu/claims/nonce, aktywnej generacji, potwierdzonego
  subsetu i atomowej migracji44. HTTP testy obejmują spóźniony zapis po delete,
  błąd po usunięciu, odebranie urządzenia, nowy service i nowe logowanie.
  Istniejący test pełnego audytu sprawdza też cleanup bez wzrostu tabeli ponad cap.
- **Pierwszy backend829 PASS**:685 relay +1 identity +79 SDK +27 terminal
  +37 usage,49 acceptance ignored; sourcec167978c. Relay160,77s,
  OOM0/peak3758096384B/max161, scratch86%,228568KiB wolnego.
  [Log](evidence/2026-11-13-mcp-secret-cleanup/server-final.log).
- **Frontend152 i TypeScript PASS**. Wstępny Clippy implementacji PASS przed
  dodaniem końcowych tests/audit; końcowy Clippy/MCP pozostają do wykonania.
  Chromium rozszerzono o utratę odpowiedzi cleanup, GET status, drugi jawny
  przebieg i rzeczywisty late write; wymaga4 delete i braku dodatkowego AS I/O.
  Uruchomiono odbiór runtime z produkcyjnym helperem i Linux sandboxem.
- Limit256 referencji nie jest resetowany, niepotwierdzone u AS sekrety nie są
  usuwane, brak automatycznego daemon sweep/retention policy. Te wymagania i
  unfinished AS reconciliation pozostają otwarte, podobnie jak cały E5.2/E5.3
  i cel E0–E10; formalne20/68 bez zmiany. Bez prawdziwego AS/keyringu/tailnetu.

- **Końcowe MCP/HTTP/vault151 oraz ich Clippy all-targets -D warnings PASS**,
  testy114+6+31, Clippy26,57s, OOM0/peak1274994688B/max0.
  [Log](evidence/2026-11-13-mcp-secret-cleanup/mcp-final.log).
- **Końcowy Clippy serwera i pakietów towarzyszących all-targets -D warnings
  PASS1m57s**, OOM0/peak2261188608B/max0.
  [Log](evidence/2026-11-13-mcp-secret-cleanup/clippy-final.log).
- **Chromium/runtime PASS7,41s**: produkcyjny stdio helper i Linux sandbox,
  jednorazowa zgoda AHP, cztery AS revoke POST, utrata odpowiedzi revoke,
  utrata odpowiedzi pierwszego cleanup, odczyt trwałego receipt i drugi jawny
  przebieg po jednym symulowanym spóźnionym zapisie. Łącznie4 delete, bez nowego
  AS I/O; końcowy sweep2/complete/ACK2, sekrety nieobecne w MemoryStore.
  Jeden wybrany case;733 innych testów odfiltrowano. OOM0/peak3758096384B/max497,
  scratch90%,165032KiB wolnego.
  [Log](evidence/2026-11-13-mcp-secret-cleanup/runtime-final.log).
- Oryginalne [desktop](evidence/2026-11-13-mcp-secret-cleanup/cleanup-desktop.png)
  i [mobile](evidence/2026-11-13-mcp-secret-cleanup/cleanup-mobile.png) PNG obejrzano;
  szerokości1440/390, status i jawne ponowienie widoczne. Mobile fullPage zawiera
  fixed skip-link ponad faktycznie przyciętym viewportem; brak szerszej deklaracji
  odbioru dostępności.
- [Manifest](evidence/2026-11-13-mcp-secret-cleanup/verified-source.json) wiąże
  backend829, MCP151, Chromium1 i Clippy z tym samym sourcec167978c. Frontend152
  i TypeScript mają właściwe węższe snapshoty. Niezależne bieżące snapshoty
  potwierdziły wszystkie trzy zakresy. Rustfmt wybranych plików, Node syntax,
  git diff --check i122 dokumenty/linki PASS. Launcher/CI bez zmian w tym
  przyroście; istniejący runtime job obejmuje case, nie uruchamiano GitHub Actions.
- Odbiór tego przyrostu ukończony, wszystkie procesy walidacji zakończone.
  Kontrolowany AS/ACP i MemoryStore nie zastępują odbioru prawdziwego dostawcy,
  konta, keyringu, tailnetu ani native macOS. Formalne20/68 bez zmiany.
  Następna luka: rozstrzyganie unfinished AS po potwierdzeniu zakończenia starego
  wykonawcy, następnie retencja/odzyskiwanie limitu i pozostałe E5.2/E5.3.

## 2026-11-13 — rozstrzyganie unfinished OAuth (odbiór fixtures ukończony)

- Dodano Store lease utrzymywany przez rzeczywisty blocking worker odwołania:
  local AtomicBool plus kernel flock dla SQLite na dysku. Zastąpienie Service,
  utrata waitera i timeout nie stanowią dowodu zakończenia. Jedna blokada na bazę;
  konflikt zamiast równoległego odwołania. Lock inode pozostaje, CLOEXEC i kontrola
  owner/mode/links odrzucają niebezpieczne wpisy.
- Owner-only REST i UI reconcile-revocation kończą tylko lokalny Running receipt,
  zachowując potwierdzenia i unknown. Zero sekretów/AS I/O, brak automatycznego
  retry. Fresh cookie/device/IdP/member revision są sprawdzane wewnątrz SQL cut.
- Dziennik zapisuje lease_protected. Legacy Running bez tego pola pozostaje
  zablokowany do osobnej migracji po potwierdzonym zatrzymaniu starego procesu;
  wolny nowy lock nie jest dowodem zakończenia starego binarnego pliku.
- Dodano testy blokady między procesami i zwolnienia po SIGKILL, niebezpiecznych
  inode, restartu SQL, zachowania ACK/unknown, obcego lease, starego CAS, legacy
  i cofniętej authority. HTTP fixture próbuje reconcile podczas żywego AS POST,
  potem po cofnięciu urządzenia i zastąpieniu Service sprawdza rozstrzygnięcie
  bez konfiguracji providera/sekretów. Odbiór backendu jest w toku.
- Chromium rozszerzono o przygotowany trwały cut unknown, utratę odpowiedzi
  reconcile i odczyt receipt bez ponowienia; rzeczywisty SIGKILL jest osobnym
  testem kernel lease, nie deklaracją crashu w tym browser fixture. Wyniki nowego
  runtime, frontend/TypeScript i końcowego Clippy pozostają do wykonania.
- Cel E0–E10 pozostaje aktywny, formalne20/68 bez zmiany. Retencja/limit referencji,
  legacy migration, URL elicitation i pozostały zakres E5 nadal otwarte.

- Pierwsza pełna regresja: **backend834 PASS** (690 relay +1 identity +79 SDK
  +27 terminal +37 usage;49 acceptance ignored), źródło0aad79b1. Wszystkie pięć
  początkowych testów odzyskiwania/lease PASS, także faktyczne wykluczenie procesu
  i zwolnienie po SIGKILL. Relay161,13s; OOM0/peak3758096384B/max267.
  [Log](evidence/2026-11-13-mcp-reconcile/server-first.log).
  Ten snapshot poprzedza ostatnie guards legacy/fresh transaction proof oraz
  rozszerzony browser fixture; nie jest końcowym odbiorem aktualnego źródła.
- **Frontend153 i TypeScript PASS** dla końcowego UI, z walidacją wspieranej
  ścieżki reconcile, dokładnych liczników/rewizji oraz braku retry po utracie
  odpowiedzi. Końcowy Clippy uruchomiony na source63eadf5f; backend/runtime
  tego snapshotu pozostają do odbioru.
- **Końcowy Clippy all-targets -D warnings PASS1m59s** na source63eadf5f,
  OOM0/peak2352218112B/max0. Obejmuje kompilację końcowych guards/testów i browser
  fixture. [Log](evidence/2026-11-13-mcp-reconcile/clippy-final.log).
  Pełny backend końcowego źródła uruchomiono; runtime Chromium jeszcze niezaliczony.
- **Końcowy backend835 PASS**:691 relay +1 identity +79 SDK +27 terminal
  +37 usage,49 osobnych acceptance ignored; source63eadf5f. Wszystkie sześć nowych
  testów odzyskiwania/lease PASS, rozszerzony pełny audit też PASS.
  Kompilacja3m17s, relay160,76s, OOM0/peak3758096384B/max230,
  scratch86%,228380KiB wolnego.
  [Log](evidence/2026-11-13-mcp-reconcile/server-final.log).
  Końcowy Chromium/runtime uruchomiony, wynik jeszcze niezaliczony.

- **Końcowy Chromium/runtime PASS8,83s** na source63eadf5f,739 innych testów
  odfiltrowano. UI rozstrzyga przygotowany unknown cut, traci odpowiedź POST,
  odczytuje unconfirmed z GET bez retry i odblokowuje ponowne logowanie. Dalej
  przechodzi poprzedni flow prywatnego MCP/AHP/OAuth/revoke/cleanup. AS2 auth/
  2 token/4 revoke/1 tool; private writes2/deletes4/late write1. OOM0/
  peak3758096384B/max713, scratch90%,164768KiB wolnego.
  [Log](evidence/2026-11-13-mcp-reconcile/runtime-final.log).
- [Desktop](evidence/2026-11-13-mcp-reconcile/reconcile-desktop.png) i
  [mobile](evidence/2026-11-13-mcp-reconcile/reconcile-mobile.png) to oryginalne
  obejrzane PNG szerokości1440/390: stan niepotwierdzony pozostaje widoczny,
  ponowne logowanie i osobne odwołanie są dostępne po rozstrzygnięciu.
- [Manifest](evidence/2026-11-13-mcp-reconcile/verified-source.json) wiąże backend835,
  Clippy i Chromium1 z identycznym source63eadf5f. Frontend153 i TypeScript mają
  właściwe węższe snapshoty; niezależne bieżące snapshoty potwierdziły wszystkie
  trzy zakresy. Rustfmt wybranych plików, Node syntax, git diff --check i122
  dokumenty/linki PASS. Biblioteki MCP/vault bez zmian w tym przyroście; ich
  poprzednich151 testów nie powtarzano. Launcher/CI bez zmian i bez uruchomienia
  nowego GitHub Actions workflow.
- Odbiór tego przyrostu jest ukończony w kontrolowanych fixtures; wszystkie
  procesy walidacji zakończone. Rzeczywisty SIGKILL dotyczy lease, a browser
  używa przygotowanego trwałego cut; nie deklarujemy crash/restartu całego
  serwera w tej ścieżce. Brak realnego AS/konta/keyringu/tailnetu/macOS acceptance.
  Cel E0–E10 nadal aktywny, formalne20/68 bez zmiany. Retencja/limit referencji,
  legacy migration i pozostałe E5 pozostają do wykonania.

## 2026-11-13 — lease zapisującego (odbiór fixtures ukończony; retencja otwarta)

- Migracja45 dodaje writer_protected=false dla istniejących referencji. Nowe
  rezerwacje pod lease mają true; pochodzenie nie jest podnoszone przy migracji.
- Callback, browser catalog i runtime access_token_or_refresh mają wspólny
  osobny kernel writer lease. Zwalniany przed MCP I/O i oczekiwaniem AHP/MRTR,
  bez mutexu Store przez keyring/sieć. Disconnect/revoke nie czekają na writer;
  ich ujemny CAS nadal blokuje spóźniony commit. Lease trwa po utracie waitera.
- Dodano Store test wykluczenia kolejnego Store i ochrony metadanych generacji,
  atomowej migracji45/collision i test HTTP utraty callback waitera podczas
  rzeczywistego SecretStore.set. Istniejący kernel SIGKILL test obejmuje teraz
  oba namespace locków. Chromium fixture sprawdza obecność lease przy set,
  jego zwolnienie przed autoryzowanym MCP HTTP i marker obu zapisanych generacji.
- Pierwszy all-target Clippy -D warnings PASS2m01s przed ostatnimi browser
  assertions; pełny backend tego przyrostu jest w toku. Końcowe Clippy/runtime
  pozostają do wykonania. To warunek bezpiecznej retencji, nie odzyskany limit:
  aktywne256, archiwizacja/metadane potwierdzeń i bounded pruning nadal otwarte.
  Cały cel E0–E10 aktywny i formalne20/68 bez zmiany.
- Pierwszy backend **niezaliczony**:617 relay PASS/77 FAIL. Migracja tworzyła
  user_version45, lecz maksymalny SCHEMA_VERSION pozostał44; ponowne otwarcia
  i asercje końcowej wersji zostały prawidłowo odrzucone. Poprawiono stałą45.
  Test utraty callback waitera i migracji45 PASS w pierwszym przebiegu.
  [Log diagnostyczny](evidence/2026-11-13-mcp-writer-lease/server-first.log).
- Dodano bounded admission wait do5s bez mutexu Store; nie powtarza token/refresh
  ani set. Runtime zachowuje oryginalny deadline/lifetime. Po uzyskaniu lease
  callback ponownie sprawdza authorizing/revision przed token exchange. Dwa nowe
  HTTP testy sprawdzają krótką kolejkę równoległych odczytów i anulowanie
  oczekującego callbacku z zerem token POST. Końcowy backend uruchomiony.
- Drugi Clippy PASS2m01s po poprawce45 i browser assertions, lecz przed ostatnim
  ponownym authorizing check/testem. Nie zastępuje końcowego Clippy aktualnego
  źródła; ten i runtime pozostają do wykonania.
- **Końcowy backend840 PASS**:696 relay +1 identity +79 SDK +27 terminal
  +37 usage;49 osobnych acceptance ignored, source da8f9009. Wszystkie pięć nowych
  Store/HTTP testów i rozszerzony kernel test obu locków PASS. Relay161,49s,
  OOM0/peak3758096384B/max521. [Log](evidence/2026-11-13-mcp-writer-lease/server-final.log).
  Chromium/helper z nowymi assertions uruchomiony; końcowy Clippy ostatniego
  źródła pozostaje do wykonania. Frontend/biblioteki MCP/vault bez zmian.
- Pierwszy runtime nie uruchomił Chromium: rustc test ELF został zabity SIGKILL,
  cgroup potwierdza OOM1/oom_kill1 przy limicie3584MiB (scratch82%,287984KiB
  wolnego). To błąd zasobów kompilacji, nie negatywny wynik scenariusza.
  [Log OOM](evidence/2026-11-13-mcp-writer-lease/runtime-oom.log).
  Końcowy Clippy ostatniego źródła uruchomiony; runtime zostanie ponowiony raz
  z tym samym źródłem i budżetami, bez podnoszenia VM/RAM.
- **Końcowy Clippy all-targets -D warnings PASS1m58s** na source da8f9009,
  OOM0/peak2415919104B/max543. [Log](evidence/2026-11-13-mcp-writer-lease/clippy-final.log).
  Ponowiono runtime dokładnie tego źródła z niezmienionym budżetem3584MiB;
  proces walidacji jest w toku, bez zaliczenia scenariusza przed wynikiem.
- Powtórny runtime również OOM1/oom_kill1 przed Chromium (test rustc ELF,
  max667). [Drugi log](evidence/2026-11-13-mcp-writer-lease/runtime-oom-repeat.log).
  Launcher zmienia wyłącznie runtime codegen-units64→128, bez zwiększenia
  RAM3584MiB, scratch1536MiB, jednego cargo job, CPU lub zakresu testów.
  Nagłówek logu publikuje codegen_units. Tooling14 CI-contract +18 launcher tests
  PASS, także assertions128/pojedynczy job/prywatny sandbox w obu runtime modes.
  [Log tooling](evidence/2026-11-13-mcp-writer-lease/tooling-final.log).
  Trzecia kompilacja/runtime jest w toku; wpływ128 na rzeczywisty peak/odbiór
  nie jest jeszcze potwierdzony. Rust source da8f9009 i poprzedni backend840/
  Clippy pozostają bez zmiany. Żaden GitHub Actions job nie został uruchomiony.
- Przy CGU128 test ELF i production relay skompilowały się bez OOM (3m13s/33,17s,
  OOM0/max1473). Chromium wykrył błąd nowej asercji obserwacyjnej: natychmiastowy
  Store.try_lock mylił krótkie zajęcie przez inny WS/controller thread z trzymaniem
  mutexu przez caller MCP. [Log](evidence/2026-11-13-mcp-writer-lease/runtime-observation-failure.log).
- Poprawiono wyłącznie ignorowany browser fixture: obserwacja ma2s limit i zwalnia
  Store przed czekaniem. Własny caller I/O nie może zakończyć pracy przed powrotem
  hooka, więc trzymanie własnego lease przy MCP lub brak lease przy set nadal
  kończą się niepowodzeniem. Nie zmieniono kodu produkcyjnego ani zwykłych testów.
- [Dowód zakresu](evidence/2026-11-13-mcp-writer-lease/source-delta.json) i
  [patch](evidence/2026-11-13-mcp-writer-lease/fixture-observation.patch): zastąpienie
  wyłącznie fixture zachowanymi poprzednimi bajtami odtwarza dokładny source
  da8f9009 zaliczonego backend840. Bieżący source6481dd1e różni się tylko tym
  ignorowanym fixture; backend840 nie jest przedstawiany jako przebieg tego
  identycznego pełnego drzewa. Nie powtarzano zwykłej regresji po takiej poprawce.
- **Bieżący Clippy all-targets -D warnings PASS2m03s**, source6481dd1e,
  OOM0/peak2333192192B/max0. [Log](evidence/2026-11-13-mcp-writer-lease/clippy-final.log).
  Ponowiony Chromium bieżącego fixture jest w toku.

- **Bieżący Chromium/runtime PASS8,56s**, source6481dd1e, CGU128,744 innych
  testów odfiltrowano. Nowe probes: writer lease zajęty przy prywatnym set,
  dostępny przy autoryzowanym MCP HTTP, obie callback generacje writer_protected1.
  Przeszedł także poprzedni reconcile/lost reply/AHP tool/revoke/cleanup flow.
  OOM0/peak3758096384B/max466, scratch90%,159736KiB wolnego.
  [Log](evidence/2026-11-13-mcp-writer-lease/runtime-final.log).
- [Manifest](evidence/2026-11-13-mcp-writer-lease/verified-source.json) rozróżnia
  backend840 na da8f9009 od bieżącego Clippy/Chromium na6481dd1e. Dowód overlay
  rekonstruuje dokładny stary hash przez zastąpienie wyłącznie ignorowanego
  browser fixture; produkcja i zwykłe testy pozostały identyczne. Nie deklarujemy
  identycznego pełnego drzewa wszystkich odbiorów. Tooling14+18 ma osobny snapshot.
  Frontend153 i TypeScript z poprzedniego odbioru użyto ponownie po niezależnym
  potwierdzeniu identycznych węższych źródeł i hashów logów. Biblioteki MCP/vault
  bez zmian. Rustfmt wybranych plików, Python syntax, git diff --check i122
  dokumenty/linki PASS. Brak nowego GitHub Actions execution.
- Wszystkie procesy tego przyrostu zakończone. To potwierdzenie lifecycle hostowego
  workera, nie dowód zakończenia operacji już przekazanej backendowi sekretów.
  GC przed pruningiem potrzebuje dodatkowego trwałego potwierdzenia zakończenia
  zapisu. Aktualny cleanup nadal zachowuje tombstones i limit256 nie jest odzyskany.
  Odbiór obejmuje kontrolowany AS/ACP/MemoryStore i Linux, bez realnych kont,
  keyringu, tailnetu czy native macOS. Formalne20/68 i cały aktywny cel E0–E10
  pozostają bez zmiany. Dalsze kroki retencji zapisano w mcp-client.md.

## 2026-11-13 — potwierdzenie pojedynczego zapisu sekretu (odbiór fixtures ukończony)

- Migracja46 i hostowy CredentialJournal dodają nonce referencji, jednorazowy
  claim przed set i osobny Ack po sukcesie. Rezerwacja/prepare zachowują dokładną
  tożsamość koperty; claim nie jest odtwarzany po utracie wyniku. Protected
  claim/ack/commit wymagają writer lease. Stare rekordy mają NULL/0/0.
- Ack omija wyłącznie wygasłą dodatnią authority i aktualną rewizję, lecz sprawdza
  całą oryginalną capability/metadane i właściciela. Nie publikuje credential,
  nie zmienia uprawnień/statusu i nie jest dostępny przez REST/MCP. Commit nadal
  wymaga świeżego proof/SQL fence; disconnect/revoked device nie są cofane.
- Dodano2 testy biblioteki (jednorazowy claim oraz macierz set/ack/commit/revoke),
  4 Store testy dokładnej capability, restartu, atomowej migracji46 i błędu SQL
  receipt oraz2 HTTP testy set success po fence i error-after-effect bez retry.
  Istniejący Chromium fixture wymaga obu generacji z claimed1/acknowledged1.
- Pierwsze **MCP/HTTP/vault153 i Clippy PASS** (116+6+31), bez OOM,
  peak1276182528B/max0. [Log](evidence/2026-11-13-mcp-write-receipt/mcp-first.log).
  Snapshot poprzedza ostatnie zmiany wyłącznie serwerowe; końcowy odbiór źródła
  pozostaje do wykonania. Pełny backend jest w toku, runtime/Clippy do wykonania.
- Kod UI/launcher/CI bez zmian. Retencja/limit256, legacy unknown, pozostałe E5
  i cały cel E0–E10 nadal otwarte; formalne20/68 bez zmiany. Bez realnych kont,
  keyringu, tailnetu lub native macOS acceptance.
- Pierwszy server build nie uruchomił testów: test rustc otrzymał SIGKILL,
  OOM1/oom_kill1 przy CGU64, RAM3584MiB, scratch81%/307372KiB wolnego.
  [Log](evidence/2026-11-13-mcp-write-receipt/server-oom.log).
  Server przełączono na CGU128, zachowując RAM/scratch/CPU/pojedynczy Cargo job
  oraz cały zakres testów. Tooling14+18 PASS; pełny server uruchomiono ponownie.
- Server CGU128 także OOM1 przed uruchomieniem testów (max1477, scratch82%,
  284664KiB wolnego). [Log](evidence/2026-11-13-mcp-write-receipt/server-oom-128.log).
  Server/runtime przełączono na CGU256 i strip=symbols, bez zmiany optymalizacji,
  debug assertions, zakresu testów ani limitów RAM/scratch/CPU. Header zapisuje
  oba ustawienia. Tooling14+18 PASS; rzeczywisty wpływ jest sprawdzany przez
  ponowną pełną kompilację/server, nie przez sam kontrakt mockowanego launchera.
- **Końcowy backend846 PASS**:702 relay +1 identity +79 SDK +27 terminal
  +37 usage;49 osobnych acceptance ignored. Wszystkie6 nowych Store/HTTP cases
  PASS. Kompilacja3m17s, relay165,94s. CGU256+strip=symbols dały OOM0,
  peak3754598400B/max0 i scratch62%,605924KiB wolnego (wobec wcześniejszych OOM
  CGU64/128). [Log](evidence/2026-11-13-mcp-write-receipt/server-final.log).
  Bieżący Clippy uruchomiony, końcowy MCP i Chromium nadal do wykonania.
- **Końcowy Clippy all-targets -D warnings PASS1m58s**, źródło6fdc1c18
  zgodne z backend846; OOM0/peak2312773632B/max0.
  [Log](evidence/2026-11-13-mcp-write-receipt/clippy-final.log).
  Niezależne snapshoty potwierdziły Rust/frontend/typecheck/tooling. Frontend
  bez zmian (hash63e45ef5) i TypeScript bez zmian (48e3c60c). Końcowy MCP
  uruchomiony na tym samym źródle; Chromium nadal do wykonania.
- **Końcowe MCP/HTTP/vault153 i ich Clippy all-targets PASS** na tym samym
  źródle6fdc1c18:116 MCP +6 HTTP +31 vault; Clippy26,20s,
  OOM0/peak1218838528B/max0. [Log](evidence/2026-11-13-mcp-write-receipt/mcp-final.log).
  Chromium/helper uruchomiony z CGU256+strip=symbols i bez zwiększenia budżetów.

- **Końcowy Chromium/runtime PASS7,91s**, ten sam source6fdc1c18,750 innych
  testów odfiltrowano. Obie rzeczywiste callback generacje mają claimed1/ack1
  obok writer_protected1. Przeszedł wcześniejszy flow reconcile/lost reply/AHP
  tool/revoke/cleanup. CGU256+strip dały OOM0/peak3699486720B/max0 i scratch64%,
  568372KiB wolnego. [Log](evidence/2026-11-13-mcp-write-receipt/runtime-final.log).
- [Manifest](evidence/2026-11-13-mcp-write-receipt/verified-source.json) wiąże
  backend846, MCP153, Clippy i Chromium1 z identycznym źródłem6fdc1c18.
  Tooling14+18 ma właściwy snapshot93ef6776. Niezależne bieżące snapshoty
  potwierdziły wszystkie cztery zakresy. Frontend153/TypeScript z poprzedniego
  odbioru użyto ponownie po potwierdzeniu identycznych węższych źródeł i hashów
  logów. Rustfmt wybranych plików, Python AST, git diff --check i122 dokumenty/
  linki PASS. Zmiana launchera nie jest deklaracją wykonania GitHub Actions.
- Wszystkie procesy tego przyrostu zakończone. To dowód zakończonego sukcesem
  wywołania set, nie odwołanie grantów AS ani usunięcie lokalnych danych.
  Backend memory fixture i SQL reopen nie zastępują realnego AS/keyringu,
  native macOS/tailnet acceptance ani pełnego runtime/conformance.
  Retencja/odzyskiwanie limitu256 i trwałe potwierdzenia AS przy referencjach
  pozostają następnym zakresem. Cel E0–E10 nadal aktywny, formalne20/68 bez zmiany.


## 2026-11-13 — E5.2/E5.3: odzyskiwanie pojemności i ograniczona retencja, odbiór w toku

- Migracja47 oddziela256 oczekujących referencji od historii:512 bezpiecznych,
  256 niepewnych i przejściowe256 podczas sweep; twardy limit1280. Potwierdzenie
  AS jest trwałe przy dokładnej referencji, niezależne od ostatniego journalu.
- Cleanup trzyma writer lease; pruning wymaga write claim/ACK, potwierdzenia AS
  i delete, chroni bieżące/staged pointery. Niepewna historia nie wygasa; po jej
  zapełnieniu kolejny niepewny wpis pozostaje obciążeniem aktywnej pojemności.
  Trwały licznik prób zapewnia sprawiedliwe partie do256 i postęp po restarcie.
- REST/UI rozróżniają available_slots, retained_history, uncertain_history,
  reclaimed i pruned. Historyczny cleanup nie wymaga ostatniego receipt AS.
- Nowe testy obejmują768 zapisów i restartów, utratę journalu/zmianę bindingu,
  ograniczenie uncertain, migrację/kolizje i REST ochrony nowej autoryzacji.
- Frontend155 PASS, TypeScript PASS i all-targets Clippy PASS przed korektą
  oczekiwanego JSON w jednym teście callbacku. Pełny backend nadal w toku;
  bieżący snapshot tego pierwszego uruchomienia nie zawiera korekty fixture.
  Nie jest to jeszcze końcowy odbiór. Logi:
  [frontend](evidence/2026-11-13-mcp-retention/frontend-final.log),
  [TypeScript](evidence/2026-11-13-mcp-retention/typecheck-final.log),
  [Clippy](evidence/2026-11-13-mcp-retention/clippy-before-server.log),
  [backend w toku](evidence/2026-11-13-mcp-retention/server-first.log).
- Formalnie20/68; cały cel E0–E10 pozostaje aktywny. Brak automatycznego AS/GC
  aktywnych tokenów, brak realnych kont/usług/tailnetu/native macOS.

- Pierwszy pełny backend zakończył się kodem101:705 relay PASS,2 FAIL,49
  osobnych acceptance ignored, identity1 PASS; dalsze pakiety nie ruszyły.
  Oba błędy to oczekiwany kontrakt JSON: brak storage w callback oraz pending1
  zamiast2 po begin_refresh. Poprawiono wyłącznie te asercje. Wszystkie4 nowe
  testy Store i nowy REST PASS, w tym768 zapisów/restarty/limity historii.
  OOM0/max0, peak3545141248B. Końcowy pełny backend uruchomiony ponownie.

- Końcowy backend używa source895b6ab3. Oba skorygowane kontrakty callback/
  disconnect i nowy REST retencji już PASS; pełny przebieg nadal trwa.
  Niezależny snapshot potwierdził bieżący Rust oraz identyczne źródła155
  testów frontend391a58a9, TypeScript92381de2 i tooling93ef6776.

- **Końcowy backend851 PASS**:707 relay+1 identity+79 SDK+27 terminal+37 usage;
  49 osobnych acceptance ignored. Source895b6ab3; oba skorygowane kontrakty
  i wszystkie5 nowych testów retencji PASS. Peak3594330112B/max0/OOM0,
  scratch62%,605792KiB wolnego. [Log](evidence/2026-11-13-mcp-retention/server-final.log).
  Uruchomiono bieżący MCP/HTTP/vault z package Clippy; Chromium i końcowy
  root Clippy nadal przed nami. To nie zamyka całego E5 ani E0–E10.

- **MCP/HTTP/vault153 i package Clippy PASS**, bieżące źródło895b6ab3;
  116 MCP+6 HTTP+31 vault. Clippy27,84s, peak1237581824B/max0/OOM0.
  [Log](evidence/2026-11-13-mcp-retention/mcp-final.log).
  Uruchomiono wybrany Chromium/helper acceptance na tym samym źródle;
  końcowy root Clippy nadal do wykonania.

- **Chromium/runtime PASS7,99s**, source895b6ab3,755 pozostałych testów
  odfiltrowano. Produkcyjny helper i Linux sandbox, kontrolowany AS/ACP/backend
  sekretów. Flow callback/reconcile/lost reply/tool approval/revoke/cleanup
  zachowany; storage pokazuje256 wolnych miejsc i2 wpisy historii, uncertain0.
  Współczesny writer ma trwały write ACK i nie symuluje późnego zapisu po nim;
  legacy late-write pozostaje osobnym testem API, bez prawa do pruning.
  [Log](evidence/2026-11-13-mcp-retention/runtime-final.log).
  OOM0/max0/peak3612385280B, scratch64%,568192KiB wolnego. To wybrany odbiór,
  nie pełna regresja runtime, platform czy rzeczywisty GitHub Actions run.
- Obejrzano oryginalne PNG panelu cleanup:
  [desktop](evidence/2026-11-13-mcp-retention/mcp_cleanup_image_desktop.png) i
  [mobile](evidence/2026-11-13-mcp-retention/mcp_cleanup_image_mobile.png).
  Liczniki i akcje mieszczą się w kartach; focus skip-link w przygotowanym
  przebiegu jest widoczny. Nie deklarujemy pełnego a11y/cross-browser E10.
  Końcowy root Clippy pozostaje w toku.

- **Końcowy root all-targets Clippy -D warnings PASS**,2m03s, source895b6ab3;
  peak2349420544B/max0/OOM0. [Log](evidence/2026-11-13-mcp-retention/clippy-final.log).
- [Manifest](evidence/2026-11-13-mcp-retention/verified-source.json) wiąże
  backend851, MCP153, Clippy i Chromium1 z identycznym źródłem895b6ab3,
  frontend155 z391a58a9 i TypeScript z92381de2. Niezależne snapshoty wszystkich
  czterech zakresów i SHA logów/PNG potwierdzone. Tooling14+18 użyto ponownie
  po potwierdzeniu identycznego93ef6776 i hashów poprzedniego dowodu;
  launcher/CI nie zmieniono w tym przyroście. Nie wykonano GitHub Actions.
- Rustfmt wybranych zmienionych plików, git diff --check i122 Markdown/linków/
  anchors PASS. Wszystkie procesy tego przyrostu zakończone.
- Retencja odzyskuje pojemność przez jawne AS revoke→cleanup→reauthorization.
  Nie ma automatycznego GC aktywnych rotacji lub niepotwierdzonych AS tokenów.
  Niepewna historia nie wygasa; przy pełnym limicie nowe niepewne referencje
  pozostają obciążeniem256 miejsc. Legacy unfinished/write outcomes i ich jawna
  migracja, URL elicitation, ogólne scoped secrets, A2A i dalsze E6–E10 pozostają
  do realizacji. Kontrolowany backend nie zastępuje prawdziwego AS/keyringu,
  native macOS ani tailnetu. Formalnie20/68 i cały cel E0–E10 nadal aktywne.


## 2026-11-13 — E5.2/E5.3: URL elicitation przez AHP i runtime, odbiór w toku

- Poprzedni przyrost retencji jest odebrany; kontynuacja pełnego celu obejmuje
  URL elicitation. AHP już definiuje url, a SDK ma typowaną kontynuację. Dodano
  hostowe mapowanie, URL journal bez answers/content i standardowe UI decyzji.
- Runtime reklamuje oba tryby. URL nie jest automatycznie pobierany/otwierany;
  UI pokazuje pełny adres, ASCII host i osobny link nowej karty z noopener/
  noreferrer/no-referrer. Otwarcie i Kontynuuj to osobne gesty; odmowa/anulowanie
  nie wysyłają content. Nie zmienia to bearera ani OAuth do samego MCP.
- Journal używa istniejącego request+null constraints, bez migracji schematu;
  nowa zależność url2 korzysta z istniejącej wersji w lockfile. Wspólna polityka
  SDK/host/journal/UI odrzuca nie-HTTPS, userinfo, raw whitespace/control/backslash.
  Testy obejmują jednorazowość, brak answers, recovery i rozdział trybów.
- Dodano4 ignorowane osobne acceptance: URL accept (2 rundy), decline, cancel,
  disconnect podczas oczekiwania. Chromium testuje brak prefetch i nagłówków,
  popup bez opener/referrer, brak automatycznej akceptacji i ręczną drugą rundę.
  Launcher dopuszcza ich wąski filtr z browser; CI full runtime obejmie je
  bez zmiany workflow. To opis przygotowanych testów, nie wynik PASS.
- Frontend157 i TypeScript PASS. Launcher18 PASS, CI-contract w toku;
  root Clippy w toku. Logi w evidence/2026-11-13-mcp-url-input/.
  Bez rzeczywistych kont/usług/tailnetu; formalne20/68 i E0–E10 nadal aktywne.

- CI-contract14 PASS i launcher18 PASS; frontend157 i TypeScript PASS. Wstępny
  root all-targets Clippy -D warnings PASS2m02s, peak2291662848B/max0/OOM0.
  Końcowy kod różni się od jego snapshotu wyłącznie dodatkowymi asercjami
  ignorowanego URL-revoke fixture (brak private-url-state i jawne cancel).
  Pełny backend uruchomiono na zamrożonym końcowym źródle. Nie deklarujemy
  jeszcze zaliczenia nowych testów journalu ani runtime/Chromium.

- **Backend853 PASS**:708 relay+1 identity+80 SDK+27 terminal+37 usage,
  53 osobne acceptance ignored. Nowy journal URL i mapowanie hosta PASS.
  Source2583f2d0, peak3539693568B/max0/OOM0, scratch62%,605136KiB wolnego.
  [Log](evidence/2026-11-13-mcp-url-input/server-final.log).
- Przegląd wykazał różnicę między parsowaniem https:example.test bez bazy
  a href w HTML z bazą HTTPS. UI zachowuje pełny oryginalny adres jako tekst,
  lecz link używa absolutnego parserowego href i hosta z tego samego parsera.
  Dodano asercje tego przypadku. Tylko3 pliki frontend zmieniły się podczas
  backendu; porównano wszystkie wejścia snapshotu i odtworzono hash źródła
  testowanego backendu z zachowanych oryginałów:
  [dowód](evidence/2026-11-13-mcp-url-input/server-source-delta.json).
  Rust i backend fixtures są identyczne bajtowo. Frontend/TypeScript są
  ponawiane; runtime i końcowy root Clippy będą na końcowym źródle.

- **Końcowy frontend157 i TypeScript PASS** po normalizacji href. Uruchomiono
  szerszy filtr runtime_remote_ z Chromium (dotychczasowe formularze, URL i
  inne zdalne acceptance); nie tylko nowy pozytywny URL case. Końcowy MCP
  oraz root Clippy pozostają do wykonania po runtime. Heavy builds sekwencyjne.

- **Zdalny runtime23 acceptance PASS**,247,07s po build,738 innych testów
  odfiltrowano, sourcec9cb28a5. Obejmuje4 nowe URL (2-rundowy accept,
  decline, cancel, odłączenie podczas input), wcześniejsze form i ich negatywne
  cases, WSS/transport/restart/proxy/renewal i produkcyjny Chromium. URL browser
  potwierdza brak prefetch, bearer/cookie/referer, oddzielne gesty open/accept,
  noopener i2 ręczne rundy; decline/cancel bez otwierania adresu lub efektów.
  Odłączenie przed odpowiedzią zatrzymuje sesję, anuluje input i nie wysyła
  kontynuacji. [Log](evidence/2026-11-13-mcp-url-input/runtime-final.log).
  Peak3669221376B/max0/OOM0, scratch64%,568072KiB wolnego.
  To szerszy wybrany zestaw remote, nie pełna regresja runtime/E10.

- Obejrzano oryginalne zrzuty URL challenge:
  [desktop](evidence/2026-11-13-mcp-url-input/url_remote_mcp_image_desktop.png) i
  [mobile](evidence/2026-11-13-mcp-url-input/url_remote_mcp_image_mobile.png).
  Widać źródłowy serwer, pełny adres i ASCII host; trzy decyzje mieszczą się
  w panelu i zawijają na mobile. Chromium potwierdził brak poziomego overflow.
  To kontrola tego UI, nie pełny odbiór a11y/cross-browser E10.

- **MCP/HTTP/vault153 i package all-targets Clippy PASS** na sourcec9cb28a5:
  116 MCP+6 HTTP+31 vault, rozszerzone unsafe URL cases bez dodatkowego HTTP.
  Clippy27,26s, peak1245175808B/max0/OOM0.
  [Log](evidence/2026-11-13-mcp-url-input/mcp-final.log).
  Uruchomiono końcowy root all-targets Clippy na tym samym źródle.

- **Końcowy root all-targets Clippy -D warnings PASS**,2m01s, sourcec9cb28a5,
  peak2289344512B/max0/OOM0. [Log](evidence/2026-11-13-mcp-url-input/clippy-final.log).
- [Manifest](evidence/2026-11-13-mcp-url-input/verified-source.json) obejmuje
  backend853, runtime23, MCP153, frontend157, TypeScript, końcowy Clippy
  oraz hostowe CI-contract14 i launcher18. Niezależne snapshoty wszystkich4
  zakresów i hashe logów/PNG/original-inputs PASS. Odtworzono dokładny
  backend2583f2d0 z bieżącego c9cb28a5 przez nałożenie wyłącznie3 zachowanych
  plików frontend sprzed normalizacji linku. Rust i backend fixtures są
  identyczne; bieżący frontend i runtime sprawdziły poprawkę canonical href.
  Tooling821cd925 bez późniejszych zmian. To nie wykonanie GitHub Actions.
- Rustfmt zmienionych modułów, Node syntax, git diff --check i122 dokumenty/
  linki/anchors PASS. Wszystkie procesy tego przyrostu zakończone. Nie ma nowej
  migracji SQL ani zmian transportu runnera: używany jest standardowy URL AHP,
  a Pending/requestState i bearer pozostają na hoście.
- URL dotyczy zatwierdzonych tools/call. Interaktywne discovery/list i pełne
  UI resources/prompts/conformance nie są zaliczone tym odbiorem. Starsze
  unfinished/write outcomes i jawna migracja, ogólne scoped secrets, A2A,
  pozostałe E6–E10 i osobno autoryzowane realne konta/keyring/tailnet/native
  macOS pozostają otwarte. Formalnie20/68; cały cel E0–E10 nadal aktywny.


## 2026-11-13 — E5.2/E5.3: operatorowa migracja legacy Running, odbiór w toku

- Przegląd kodu potwierdził brak identyfikacji kernelowej workera w legacy;
  nie wolno wnioskować shutdown z nowego wolnego flock. Dodano jawny lokalny
  show/recover z deklaracją zatrzymania wszystkich starych instancji, a nie
  fałszywy automatyczny dowód. Polecenie nie jest dostępne przez HTTP/MCP/runner.
- Show otwiera wyłącznie istniejący current-schema plik readonly, bez CREATE/
  migracji; bezpieczny review digest przypina plik/aktora/membership revision/
  dokładny ledger. Recover wymaga expected revision, digest, change ID i ack.
- Recover ponownie sprawdza aktywnego owner/admin workspace, oba modern leases
  i SQL CAS. Zwiększa rewizję/nonce, kończy stare Running bez nowych AS/write
  potwierdzeń, zachowuje targets i referencje, nie przywraca członkostwa ani
  aktywnych credentials. Prywatne operatorowe pochodzenie pozostaje jawnie
  operator_attestation i jest przenoszone do kolejnych bounded ledgerów.
- Dodano7 testów parsera/Store/CLI oraz osobny ignorowany runtime acceptance
  rzeczywistego executable. Obejmują stale digest, membership ABA, live leases,
  brak nowych potwierdzeń, audit rollback/full capacity, pliki i old schema,
  nieaktywny owner i odcięcie starego intent. Clippy w toku, backend/runtime
  jeszcze wymagają wykonania. Bez realnych baz/kont/usług; formalnie20/68.

- Pierwszy backend: identity1 PASS, relay714 PASS/1 FAIL/54 acceptance ignored;
  dalsze pakiety nie ruszyły po błędzie. [Log](evidence/2026-11-13-mcp-legacy-recovery/server-first.log).
  Wadliwy krok testu ustawił attempted_revision na już istniejące2, więc digest
  słusznie pozostał aktualny. Zastąpiono krok prawdziwym confirm starego workera
  przez Journal; test weryfikuje odrzucenie digestu sprzed tego postępu. Produkcja
  bez zmian. Pozostałe6 nowych testów PASS; OOM0/max0/peak3549577216B.
  Pełny końcowy backend uruchomiono ponownie na poprawionym fixture.

- **Końcowy backend860 PASS**:715 relay+1 identity+80 SDK+27 terminal+37 usage;
  54 osobne acceptance ignored. Wszystkie7 nowych testów PASS, w tym realna
  zmiana journalu pomiędzy show i recover. Sourcec706bd99, peak3596922880B/
  max0/OOM0, scratch62%,604968KiB wolnego.
  [Log](evidence/2026-11-13-mcp-legacy-recovery/server-final.log).
  Uruchomiono osobny production CLI acceptance w runtime; nie jest Chromium.

- **Production CLI acceptance1 PASS**,0,10s po build,768 innych testów
  odfiltrowano. Rzeczywisty executable wykonuje show/recover na syntetycznej
  bazie, z pustym env; writer lease blokuje apply, po zwolnieniu operatorowa
  migracja zachowuje referencje/unknown, zwraca revision3 i odrzuca replay.
  To test CLI, nie przeglądarki, AS ani dowód zakończenia starego procesu.
  [Log](evidence/2026-11-13-mcp-legacy-recovery/runtime-final.log).
  Sourcec706bd99, peak3563261952B/max0/OOM0, scratch64%,570060KiB wolnego.
- Przegląd wymagań E5.2 potwierdził w oficjalnym MRTR Supported Requests,
  że input_required jest dozwolony wyłącznie na tools/call,prompts/get,
  resources/read. Discovery/list MUSI być nieinteraktywne; wcześniejsza notatka
  o braku odbioru interaktywnego discovery/list nie stanowi luki protokołu.
  Skorygowano nieaktualne opisy URL/revoke/retencji w mcp-client.md oraz dodano
  macierz konkretnych wymagań E5.2. Checkbox pozostaje bez zmian do final checks.

- **MCP/HTTP/vault153 i package Clippy PASS** na sourcec706bd99:116 MCP,
  6 HTTP,31 vault; Clippy37,74s, peak1284198400B/max0/OOM0.
  [Log](evidence/2026-11-13-mcp-legacy-recovery/mcp-final.log).
  Uruchomiono końcowy root all-targets Clippy na tym samym źródle.

- **Końcowy root all-targets Clippy -D warnings PASS**,2m34s, sourcec706bd99;
  peak2299285504B/max0/OOM0. [Log](evidence/2026-11-13-mcp-legacy-recovery/clippy-final.log).
- [Manifest](evidence/2026-11-13-mcp-legacy-recovery/verified-source.json) wiąże
  backend860,MCP153,production CLI1 i Clippy z identycznym c706bd99. Niezależne
  cztery snapshoty i hashe logów PASS. Frontend157/TypeScript/CI-contract14/
  launcher18 użyto ponownie po porównaniu identycznych86f046de/d800150d/821cd925
  i hashów poprzednich logów. Nie zmieniano launchera/CI ani UI; nie uruchomiono
  GitHub Actions. Poprzedni form/URL remote23 pozostaje osobnym odebranym
  przyrostem na c9cb28a5, nie jest deklarowany jako nowy przebieg.
- **Zamknięto E5.2 po audycie konkretnych wymagań punktu**:2026+adapter2025,
  discovery/metadata,MRTR,subscriptions,OAuth i schemas z limitami. Macierz jest
  w mcp-client.md, a manifest wskazuje testy/źródła dla każdego wymagania.
  Oficjalne MRTR dopuszcza wyłącznie tools/call,prompts/get,resources/read;
  interaktywne discovery/list jest zabronione, więc wcześniejsza notatka
  o jego nieodebraniu nie oznacza brakującego wymagania. Potwierdzono także
  oficjalne reguły fallback400/recognized modern errors zgodne z implementacją.
- Formalny postęp: **21/68 =30,9% pełnych punktów**. E5.3–E5.5 i pozostały
  plan pozostają otwarte. Niepewne dawne write outcomes zachowują unknown,
  bez dopisania write ACK lub wygaszenia historii. Ogólne scoped secrets,
  katalog/Builder/A2A i dalsze etapy wymagają pracy; realne konta/AS/keyring/
  tailnet/native platformy nadal osobno w E10, po zgodzie właściciela.
- Rustfmt zmienionych modułów, git diff --check i122 Markdown/linków/anchors
  PASS. Wszystkie procesy tego przyrostu zakończone. Cały cel E0–E10 pozostaje
  aktywny i nieukończony; nie wykonywano migracji rzeczywistej bazy ani usług.

## 2026-11-13 — E5.3: bieżąca authority lokalnego vaulta, odbiór w toku

- Poprzedni turn statusowy nie zmienił implementacji. Ponownie sprawdzono plan,
  a następny dostępny zakres to rzeczywista luka lokalnego vaulta: snapshot
  authority był pobierany tylko przed całym użyciem sekretu.
- Dodano hostowy, niedeserializowany LiveAccess z deadline i terminalnym
  odwołaniem współdzielonym przez klony. Relay przypina projekt, ścieżkę,
  identyfikator sesji i read-only oraz ponawia istniejący hostowy authorizer.
  Zatrzymanie vaulta odwołuje wszystkie zarejestrowane sesje.
- Guarded list/http/run są podłączone do produkcyjnego handlera Unix socket.
  Broker sprawdza authority i chronioną politykę po zgodzie, po odczycie wartości,
  przed wysłaniem HTTP/uruchomieniem przepisu oraz przed oddaniem wyniku.
  Refresh legacy OAuth ma osobne kontrole przed/po HTTP. Utrata uprawnień po
  dispatch usuwa odpowiedź/błąd dostawcy, zwraca OutcomeUnknown i zapisuje
  audyt unknown bez sekretu; nie wykonuje retry. Nie cofa już wysłanego HTTP.
- CommandExecutor kontroluje authority przed spawn/retry oraz w watchdogu
  co25ms i zabija uniksową grupę procesu po odwołaniu. Niestandardowy executor
  musi jawnie obsłużyć guarded execution; domyślnie odmawia, zamiast pomijać
  kontrolę. To przerwanie procesu, nie dowód cofnięcia jego wcześniejszych efektów.
- Dodano przypadki cofnięcia podczas zgody/odczytu/HTTP, zmiany protected policy,
  braku dalszego dispatch po refresh, zachowania redakcji i rzeczywistego
  zatrzymania procesu po sygnale rozpoczęcia, oraz pinów/shutdown RelayVault.
- Granice: ten przyrost dotyczy istniejącego lokalnego vaulta. LiveAccess nie
  jest jeszcze jednorazowym permittem związanym z aktorem/workspace/turą/profilem
  managed runtime ani protokołem runnera. Równość polityki i timestamp SQL nie
  dowodzą braku policy/value ABA; potrzebny jest osobny wersjonowany scoped
  rekord i grant. Watchdog sprawdza hostową authority, nie odczytuje keyringu
  w pętli; zmiana samej polityki podczas przepisu blokuje wynik po wykonaniu.
  Pełny kill switch (w tym odłączenie transportu) i ogólne scoped secrets
  nadal otwarte. E5.3 bez zmiany checkboxa; formalnie21/68=30,9%.
- Pierwszy snapshot MCP/HTTP/vault przeszedł testy; Clippy i końcowe przebiegi
  po uzupełnieniu audytu i testów wymagają potwierdzenia. Nie używano realnych
  kont, keyringu ani usług. Dowody: evidence/2026-11-13-vault-live-access/.

- Pierwszy przebieg:161 testów PASS (116 MCP+6 HTTP+39 vault), Clippy odmówił
  przez blocks_in_conditions w nowym ramieniu OAuth. Zmieniono wyłącznie układ
  let/match; równolegle uzupełniono audyt, shutdown/piny i dwa dalsze testy.
  [Log diagnostyczny](evidence/2026-11-13-vault-live-access/mcp-first.log).
- Końcowy MCP/HTTP/vault: **163 PASS** (116+6+41) oraz package Clippy PASS;
  source2cdb3178, peak1216901120B/max0/OOM0. Wszystkie10 nowych przypadków
  vaulta przeszły, w tym rzeczywisty proces/grupa procesu i audyt unknown.
  [Log](evidence/2026-11-13-vault-live-access/mcp-final.log).
  Niezależny snapshot potwierdził identyczne Rust inputs; frontend/TypeScript/
  tooling pozostały odpowiednio86f046de/d800150d/821cd925. Trwa pełny backend,
  po nim root Clippy; nie deklarujemy jeszcze ich wyników.
- **Pełny backend861 PASS**:716 relay+1 identity+80 SDK+27 terminal+37 usage;
  54 osobne acceptance ignored. Nowy test RelayVault pinów/read-only/shutdown
  PASS. Relay716 trwał201,21s; peak3531628544B/max0/OOM0, scratch62%,604836KiB
  wolnego. Source2cdb3178 jak końcowe MCP163.
  [Log](evidence/2026-11-13-vault-live-access/server-final.log).
  Trwa root all-targets Clippy; nie uruchamiano nowych testów przeglądarkowych
  ani acceptance produkcyjnego executable w tym przyroście.

- **Końcowy root all-targets Clippy -D warnings PASS**,1m58s, source2cdb3178;
  peak2321469440B/max0/OOM0.
  [Log](evidence/2026-11-13-vault-live-access/clippy-final.log).
- [Manifest](evidence/2026-11-13-vault-live-access/verified-source.json) wiąże
  MCP163,backend861 i Clippy z tym samym pełnym snapshotem Rust. Sprawdzono
  zakończenie wszystkich przebiegów kodem0, hashe logów, liczbę testów i brak
  błędów. Frontend157/TypeScript/CI-contract14/launcher18 pozostają wynikami
  poprzednich przebiegów, użytymi po porównaniu identycznych narrow snapshots
  i hashów oryginalnych logów; nie uruchamiano GitHub Actions.
- Odbiór tego przyrostu lokalnego vaulta ukończony. **E5.3 nadal otwarte**;
  formalnie21/68=30,9%. Następny zakres: osobny niezmienny scoped rekord
  sekretu/generacji i grant, następnie hostowy permit aktor/workspace/projekt/
  sesja/tura/profil/lease oraz integracja managed runtime i runnera. Powiązanie
  wartości z chronioną wersją ma zastąpić samą równość polityki/timestamp.
  Pełny kill switch wymaga też odbioru przerwania transportu i pozostałych
  ścieżek. Nie utożsamiać tego lokalnego odbioru z zamknięciem E5.3 ani E10.
- Rustfmt, diff check i122 Markdown/linków/anchors PASS. Wszystkie procesy
  testów tego przyrostu zakończone. Cały cel E0–E10 pozostaje aktywny.

## 2026-11-13 — E5.3: rdzeń niezmiennych scoped credentials, odbiór w toku

- Poprzedni turn przyniósł kod lokalnego live vaulta i potwierdzone testy, więc
  był postępem. Sprawdzono obecne Store/authority/OAuth i otwarty zakres E5.3.
- Dodano moduł biblioteczny vault/scoped: jeden chroniony rekord schema1 zawiera
  projekt/workspace/host, identyfikator poświadczenia, politykę i wartość.
  Każdy zapis ma nową losową referencję scoped-secret i SHA całego rekordu.
  Produkcyjna fabryka wybiera osobne namespaces macOS/Linux; legacy raw broker
  nie dostaje JSON envelope jako wartości do wstrzyknięcia. Polityka i wartość
  nie są zapisywane oddzielnie. Wartość i roboczy JSON są zeroizowane.
- Instalacja wymaga od hostowego Journal rezerwacji/CAS i jednorazowego claim
  przed keyring I/O, potwierdzenia zapisu, następnie świeżej aktywacji.
  Błąd zapisu, utrata ACK lub niepewny commit aktywacji zwraca WriteUnknown;
  rdzeń nie usuwa zarezerwowanej referencji ani nie powtarza zapisu. Generacja
  poprzednia nie jest nadpisywana. Journal nadal wymaga implementacji SQL.
- Permit nie ma Deserialize/Serialize/Clone. Hostowe Authority otrzymuje
  human/service actor, workspace/projekt/host, sesję/turę/request, profil,
  lease instance/epoch, grant revision, read-only oraz dokładną wersję/digest
  poświadczenia. Osobne consume ma trwale zużyć zgodę przypiętą do SHA pełnego
  HttpInput. Rdzeń przekazuje te piny i wymaga świeżych kontroli; rzeczywiste
  walidacje SQL/profilu/grantu są odpowiedzialnością jeszcze niewdrożonego
  adaptera Authority, nie wynikiem samego istnienia tych pól.
- Scoped HTTP porównuje cały envelope i jego binding, używa istniejącej
  walidacji host/path/method/headers, AWS signing i redakcji. Read-only blokuje
  zapis. Authority i expiry polityki są sprawdzane przed dispatch i po HTTP,
  także przed zwrotem po completion. Utrata odpowiedzi/authority po dispatch
  zwraca statyczne OutcomeUnknown bez provider data/retry. Completion jest
  odrębnym potwierdzeniem hostowym. OAuth2 refresh jest jawnie odrzucony w tym
  rekordzie: rotację obsługuje osobny już istniejący durable OAuth engine.
- Pierwszy przebieg zatrzymał kompilator testu: HttpPlan.headers jest Vec par,
  a fixture używał get(string). Poprawiono fixture na wyszukanie nagłówka.
  [Log](evidence/2026-11-13-scoped-credentials/mcp-first.log).
- Drugi snapshot:172 testy (116 MCP+6 HTTP+50 vault),2 compile-fail doctests i
  package Clippy PASS; peak1221783552B/max0/OOM0. Testy obejmują podmianę
  polityki/wartości/bindingu, zmianę input, read-only, revocation przy odczycie
  i HTTP, utratę write/completion ACK oraz brak ponownego consume. Test pinów
  sprawdza przekazanie ich mockowi Authority, nie produkcyjną autoryzację SQL.
  [Log](evidence/2026-11-13-scoped-credentials/mcp-second.log).
- Przegląd dodał kontrolę expiry po HTTP i niepewnego commit ACK aktywacji;
  trwa końcowy przebieg na uzupełnionym źródle. To rdzeń biblioteczny z
  kontrolowanymi MemoryStore/Journal/Authority fixtures. **Nie jest jeszcze
  podłączony do produkcyjnego server/runnera**. Produkcyjny SQL journal,
  provisioning/grants, profile, managed HTTP/recipes i pełny kill switch
  pozostają wymagane. Nie zamknięto E5.3; formalnie21/68=30,9%.

- **Końcowy MCP/HTTP/vault174 +2 compile-fail PASS**, package all-targets
  Clippy -D warnings PASS (23,70s);116 MCP+6 HTTP+52 vault, w tym11 nowych
  scoped testów i2 testy typu Permit. Source05fb6521, peak1217617920B/
  max0/OOM0. Potwierdzono także expiry po dispatch i niepewny commit ACK
  aktywacji bez usuwania istniejącego rekordu.
  [Log](evidence/2026-11-13-scoped-credentials/mcp-final.log).
- Końcowy rustfmt przestawił wyłącznie dwa sąsiednie pub mod scoped/store
  w lib.rs. [Porównanie](evidence/2026-11-13-scoped-credentials/source-equivalence.json)
  odtwarza pełny testowany05fb6521 po odwróceniu tylko tych dwóch deklaracji
  w snapshotcie aktualnego2ca93310. Żaden inny plik źródłowy nie różni się;
  deklaracje nie mają macro_use. To dowód różnicy formatowania, nie nowy
  przebieg testów na2ca93310. Rustfmt całego zmienionego zakresu PASS.
- [Manifest](evidence/2026-11-13-scoped-credentials/verified-source.json)
  wskazuje rzeczywisty testowany snapshot, aktualne źródło, powyższe porównanie,
  liczbę testów i hashe logów. Frontend/TypeScript/tooling mają identyczne narrow
  snapshots i wcześniejsze wyniki. Nie uruchamiano ponownie backendu861,
  root Clippy, przeglądarek ani produkcyjnego executable: aplikacja nie jest
  jeszcze podłączona do nowego modułu; odbiór managed integracji wymaga tych
  dalszych testów po jej wdrożeniu. Brak realnych kont/keyringu/usług.
- Odbiór rdzenia bibliotecznego ukończony. Następny zakres to **produkcyjny SQL
  Journal i Authority**, z transakcyjnym CAS/grantami, trwałymi references i
  jednorazowymi requestami, następnie provisioning/profile i managed transport.
  Nie przedstawiać mocków jako dowodu durability SQL/restartu ani działającej
  funkcji w UI. E5.3 nadal otwarte,21/68=30,9%; cały cel E0–E10 aktywny.
- Rustfmt, diff check i122 Markdown/linków/anchors PASS. Procesy tego przyrostu
  zakończone; pierwsza kompilacja nieudana, drugi i końcowy przebieg zakończone
  kodem0, bez OOM. Nie zamknięto żadnego dodatkowego punktu planu.

## 2026-11-13 — E5.3: produkcyjny journal scoped secrets i provisioning API, odbiór w toku

- Poprzedni turn dostarczył kod biblioteki i dowody testów, był więc postępem.
  Sprawdzono obecne Store/schema47, BrowserProof, grants, host registry i API.
- Schema48 dodaje scoped_credentials i scoped_secret_references: wyłącznie
  binding, monotoniczną revision, stan, niezmienne referencje/digest oraz
  pierwotny writer proof, write_ack i activated. Bez kolumn value/policy.
  Migrowanie oraz kolizja obcych tabel wymagają transakcyjnego odbioru.
- Produkcyjny BrowserJournal implementuje Journal używany przez Credentials.
  Rezerwacja i aktywacja używają IMMEDIATE transakcji: aktualny browser cookie/
  device/IdP/host-account policy, ManageProject i ManageHosts, dokładne rewizje
  membership/project grant/host oraz zatwierdzony host. Refaktoryzacja istniejącego
  BrowserProof wydziela wspólną validate_identity, zachowując kontrolę OAuth user.
  Operacje keyringu pozostają poza Store;4 sloty trzymane do końca blocking job,
  również po utracie waitera. To provisioning, nie wykonanie na hostowym checkout.
- Rezerwacja/CAS następuje przed I/O, aktywacja wymaga dokładnego write ACK.
  ACK po logout/revocation jest dopuszczalny tylko dla oryginalnego writer proof
  i identycznej referencji/wersji/digestu: zapisuje fakt, nie przywraca dostępu.
  Późny writer nie aktywuje slotu zmienionego przez disable/nową generację.
  Nie ma automatycznego ponowienia ani kasowania keyringu. Limity:128 credentials
  na projekt,256 references na credential i4096 references na workspace.
  Po osiągnięciu limitu odmawiamy kolejnej rezerwacji bez usuwania unknowns;
  disable nadal działa. Audyt rezerwacji/aktywacji/disable jest częścią CAS.
- API cookie/same-origin pod /v1/web/workspaces/{id}/projects/{project}/hosts/
  {host}/scoped-secrets: GET list, POST /{credential}, POST /{credential}/disable.
  Body limity80KiB/1KiB, deny_unknown_fields, opcjonalne expected-user i no-store
  także na błędach. Wartość żądania ma Zeroizing; odpowiedź zawiera wyłącznie
  status albo potwierdzony historyczny receipt aktywacji. Brak GET wartości.
  Instalacja używa odrębnego namespace keyringu. API status ma
  execution_supported:false, bo managed Authority/transport nie są podłączone.
- Dodano8 testów SQL i5 API. Obejmują reopen/pending, drugi proces/Store i CAS,
  grant/membership ABA, cofnięcie wymaganej permission, cudzy writer proof,
  późny ACK po disable/nowej wersji, audit rollback, capacity bez pruning,
  host revocation, migrację47→48/collision, pełny REST→SQL→MemoryStore,
  lost write ACK, logout/grant ABA podczas I/O i odmowy cookie/origin/actor/scope.
  Zaktualizowano9 istniejących fixture'ów downgrade, aby odtwarzały historyczny
  schema bez tabel48; produkcyjna migracja nie usuwa żadnych tabel.
- Pierwszy kontener zatrzymano świadomie przed testami (exit137), po znalezieniu
  dwóch błędów fixture: tag kind zamiast type w Injection i podwójny mutex lock
  w assert_ne. Poprawiono także fixture ABA tak, by aktor nadal mógł wykonać
  drugą zmianę grantu; odwołanie wymaganej permission ma osobny test.
  [Log diagnostyczny](evidence/2026-11-13-scoped-sql/server-first.log).
  Drugi pełny backend trwa; nie deklarujemy jeszcze wyniku.
- E5.3 nadal otwarte,21/68=30,9%. Po odbiorze provisioning pozostaje produkcyjne
  Authority check/consume/complete, definicje/granty użycia, managed runtime/
  runner i recipes, UI oraz pełny kill switch/retencja. Nie utożsamiać konfiguracji
  sekretu z uprawnieniem agenta do użycia ani syntetycznego hosta z realnym tailnet.
- Drugi backend zakończył się exit101: identity1 PASS, relay723 PASS/6 FAIL,
  54 acceptance ignored; dalsze pakiety nie zostały uruchomione. Wszystkie13
  nowych testów PASS. Pięć fixture'ów inline w Store/ahp/native_grants pozostawiało
  tabele48 przy downgrade (kolizja scoped_credentials); operator CLI miał stałe
  [46,48]/47. Poprawiono te testy bez osłabiania migracji: usuwanie tabel48
  dotyczy tylko odtwarzanych syntetycznych baz, CLI porównuje SCHEMA_VERSION±1.
  Łącznie12 plików historycznych fixture'ów oraz osobny test CLI są uaktualnione.
  Source6a81fb64, peak3571757056B/max0/OOM0, scratch62%,603892KiB wolnego.
  [Log](evidence/2026-11-13-scoped-sql/server-second.log).
- Przegląd dodał konieczną gwarancję negative cut: disable ma obowiązkowy,
  ograniczony last_disable receipt w credential row (aktor/device/stamps/rev/time).
  Pełny ogólny audit lub odrzucony pojedynczy event nie blokuje odwołania;
  transakcja nadal musi móc trwale zapisać receipt. Dołożono9. test SQL dla
  100000 eventów oraz triggerowego ABORT. To14 nowych testów łącznie z5 API.
  Aktywacja/rezerwacja nadal wymagają zwykłego audytu i nie działają po capacity.
- Uruchomiono końcową sekwencję root Clippy→pełny backend→MCP/HTTP/vault→
  pojedynczy production CLI acceptance legacy recovery. Źródło6136603d
  potwierdza niezależny snapshot; frontend/TypeScript/tooling bez zmian.
  Wyniki tej sekwencji wymagają potwierdzenia. Bez realnych kont/keyringu/usług.

- **Końcowy root all-targets Clippy -D warnings PASS**,2m08s, source6136603d;
  peak2388971520B/max0/OOM0. [Log](evidence/2026-11-13-scoped-sql/clippy-final.log).
  Rozpoczął się końcowy pełny backend z14 nowymi testami; po nim MCP i CLI.
- **Końcowy backend875 PASS**:730 relay+1 identity+80 SDK+27 terminal+37 usage;
  54 osobne acceptance ignored. Wszystkie14 nowych testów oraz6 poprawionych
  przypadków historycznych PASS, w tym disable przy100000 eventów i ABORT
  pojedynczego audytu. Relay730 trwał199,99s. Source6136603d,
  peak3575078912B/max0/OOM0, scratch62%,603864KiB wolnego.
  [Log](evidence/2026-11-13-scoped-sql/server-final.log).
  Trwają MCP/HTTP/vault i osobny production CLI acceptance na tym samym źródle.
- **Końcowy MCP/HTTP/vault174 +2 compile-fail PASS**, package all-targets
  Clippy -D warnings PASS (26,84s);116 MCP+6 HTTP+52 vault. Source6136603d,
  peak1289719808B/max0/OOM0.
  [Log](evidence/2026-11-13-scoped-sql/mcp-final.log).
  Rozpoczął się osobny production CLI acceptance; nie jest to browser test
  ani dowód realnego keyringu. Odbiór kontynuowany po północy14 listopada.

- **Production CLI acceptance1 PASS**,0,11s po build,783 innych testów
  odfiltrowano. Rzeczywisty executable poprawnie wykonuje operatorowe show/
  recover przy schema48, z zachowaniem kernel lease i unknown/reference proofs.
  To CLI, nie Chromium ani realny keyring. Source6136603d,
  peak3575861248B/max0/OOM0, scratch64%,568592KiB wolnego.
  [Log](evidence/2026-11-13-scoped-sql/runtime-final.log).
- [Manifest](evidence/2026-11-13-scoped-sql/verified-source.json) wiąże backend875,
  MCP174+2,root/package Clippy i CLI1 z identycznym6136603d. Zweryfikowano exit0
  wszystkich czterech procesów, liczby testów i hashe logów. Frontend157,
  TypeScript,CI-contract14 i launcher18 są wcześniejszymi wynikami po sprawdzeniu
  identycznych narrow snapshots86f046de/d800150d/821cd925 oraz oryginalnych
  logów. Nie uruchamiano GitHub Actions ani nowych testów przeglądarkowych.
- **Odbiór provisioning SQL/API ukończony14 listopada.** Obejmuje produkcyjny
  Journal/handler/Service z kontrolowanym keyringiem MemoryStore oraz trwałe
  bazy testowe. Produkcyjny namespace keyringu jest kompilowany, nie używany
  na realnym koncie. Nie wykonano migracji rzeczywistych danych ani usług.
  Source,24 pliki rustfmt,diff check i122 Markdown/linków/anchors PASS.
- E5.3 wciąż otwarte,21/68=30,9%. Następne: produkcyjny Authority/granty użycia
  i profile, następnie managed broker/runner/recipes oraz UI i pełny kill switch.
  Konfiguracja sekretu nie nadaje runtime uprawnienia; API status zachowuje
  execution_supported:false. Retencja/recovery ogólnych sekretów wymaga dalszej
  pracy; pojemność nie jest odzyskiwana przez usuwanie unknowns. Wszystkie
  procesy tego przyrostu zakończone. Cały cel E0–E10 pozostaje aktywny.

## 2026-11-14 — przygotowanie commita i push na prośbę właściciela

- Zweryfikowano repo/branch/HEAD/index i zgodność z origin. Snapshoty Rust,
  frontendu,TypeScript/toolingu były zgodne z odebranym6136603d oraz narrow
  hashes; nie uruchamiano ponownie ciężkich testów bez zmiany zachowania.
- Kontrola całego nowego indeksu ujawniła jeden zbędny odstęp w testowym SQL
  local_metadata_migration_tests.rs. Usunięto tylko ten bajt. [Porównanie pełnego
  źródła](evidence/2026-11-14-commit/source-equivalence.json) odtwarza dokładnie
  wcześniej testowany6136603d po jego przywróceniu; kod produkcyjny bez zmian.
  To porównanie formatowania, nie nowy przebieg backendu875/MCP174+2/CLI1.
- Surowe logi i proof patches zachowano byte-for-byte ze względu na SHA oraz
  znaczące spacje kontekstu patchy. Reguła
  .gitattributes wyłącza wyłącznie ich whitespace diagnostics; źródła nadal
  podlegają sprawdzeniu. Pliki kluczy to publiczne, syntetyczne fixtures OIDC/TLS,
  ładowane wyłącznie w testach; skan nie wskazał realnych credentials. Index
  obejmuje powiązany przyrost implementacji, fixture'y, narzędzia i dokumentację.
- Prośba o commit/push nie zamyka celu E0–E10 ani E5.3; formalnie21/68=30,9%.

## 2026-11-14 — E5.3: granty scoped secrets i katalog profilu, odbiór w toku

- Poprzedni turn zapisał i wypchnął commit3325e16 na prośbę właściciela, był
  postępem. Ponownie sprawdzono czyste drzewo, schema48 i istniejącą allowlistę
  MCP. Prace nad całym E0–E10 pozostają aktywne; brak nowego polecenia commita.
- Dodano stały schemat MCP narzędzia http i zarezerwowany namespace slotów
  dunlin-secret-*. Profil schema2 przypina provider/name/definition digest,
  tak jak pozostałe MCP. Definicja nie zawiera poświadczeń, hostów ani referencji
  keyringu. Argumenty nie mogą podać secret; przypisuje go host z prywatnego
  grantu. Są dwa rozłączne tryby: method/url albo zarejestrowana operation/params.
  Istniejący protected policy nadal ogranicza rzeczywisty HTTP. Zewnętrzna
  konfiguracja OAuth nie może użyć zarezerwowanego prefiksu.
- Schema49 dodaje scoped_secret_grants: dokładny principal kind/id,
  AgentProfileRef (ID/version/digest), slot MCP, credential revision, rewizje
  membership/project grant/host, read-only oraz monotoniczną rewizję grantu.
  Revoke przechowuje tombstone i bounded last_revoke, także przy pełnym audycie.
  Zmiana uprawnień nie odświeża starej wybranej rewizji. Dwa profile o identycznej
  treści/digest mają osobne granty; nie utożsamiamy ich ID i wersji.
- Konfiguracja grantu wymaga istniejącego activated/write-acknowledged credential,
  zatwierdzonego hosta, dokładnego opublikowanego profilu i zgodnego pinu http.
  Grantee human musi mieć ViewProject+CreateSession; service — bieżący, nieodwołany
  i niewygasły principal w tym projekcie z ViewProject+RunWorkflow. Rewizje źródła
  są przypięte, więc revocation/regrant/ABA wymaga jawnego regrantu sekretu.
- Manager API: GET/POST scoped-secrets/{credential}/grants oraz POST
  scoped-secrets/{credential}/grants/{grant}/revoke. Katalog scoped-secret-tool
  zwraca schema/digest/prefix. Recipient API secret-bindings z AgentProfileRef
  zwraca wyłącznie własne bieżące bindings, bez ManageHosts. Wszystkie pozostają
  cookie/same-origin/expected-user/no-store i nie czytają keyringu. Katalog
  filtruje wersje, host i źródłowe rewizje w SQL; bound1024 wynika z8 slotów i128
  credentials, a sentinel przekroczenia kończy zapytanie zamiast cichego ucięcia.
- Execution_supported pozostaje false: nowe granty i metadata resolver nie są
  same w sobie runtime permittem. Wciąż wymagane: actual session/turn/lease
  Authority, jednorazowa zgoda SDK, broker/runner i pełny kill switch/retencja/UI.
- Dodano testy grantów SQL (reopen,CAS,grantee ABA,service revocation,profile/
  version/slot,Credential/Host invalidation,profile identity,negative audit),3 API
  i test reserved OAuth namespace, oraz biblioteczny test argumentów http.
  Historyczne downgrade fixtures uwzględniają brak tabel49.
- Pierwszy MCP build exit101: użyty ToolValidation jest prywatny. Zastąpiono go
  publicznym bounded Schema bez poszerzania API SDK.
  [Log](evidence/2026-11-14-scoped-grants/mcp-first.log).
  Pierwszy root Clippy exit101: fixture przekazał extra revision do raw row lookup;
  poprawiono tylko wywołanie testu. [Log](evidence/2026-11-14-scoped-grants/clippy-first.log).
- Przegląd dodał terminalny negative cut przy revision ceiling: credential disable
  i grant revoke działają przy maksymalnej rewizji bez dalszej inkrementacji;
  positive install/regrant przy ceiling pozostaje zabronione. Jest osobny test
  obu przypadków. Trwa drugi root Clippy, po nim backend i MCP. Bez deklaracji
  wyników i bez realnych kont/keyringu/tailnet. Formalnie21/68=30,9%, E5.3 otwarte.

- **Root all-targets Clippy -D warnings PASS**,2m10s, sourcee5ce2f4a;
  peak2329710592B/max0/OOM0. [Log](evidence/2026-11-14-scoped-grants/clippy-final.log).
- **Pełny backend889 PASS**:744 relay+1 identity+80 SDK+27 terminal+37 usage;
  54 osobne acceptance ignored. Wszystkie14 nowych testów backendu PASS
  (10 grant SQL,3 API,1 reserved namespace), w tym terminalny limit obu
  negative cuts i izolacja katalogu odbiorcy. Relay744 trwał207,74s;
  peak3592929280B/max0/OOM0, scratch62%,603292KiB wolnego.
  [Log](evidence/2026-11-14-scoped-grants/server-final.log).
- **MCP/HTTP/vault175 +2 compile-fail PASS**, package all-targets Clippy PASS
  (36,60s);116 MCP+6 HTTP+53 vault. Nowy test przypina statyczny schemat i
  odrzuca wybór credential przez argumenty oraz mieszanie trybów. Sourcee5ce2f4a,
  peak1311780864B/max0/OOM0. [Log](evidence/2026-11-14-scoped-grants/mcp-final.log).
- Niezależny snapshot potwierdzae5ce2f4a dla całego Rust. Frontend/TypeScript/
  tooling pozostają86f046de/d800150d/821cd925. Końcowy production CLI acceptance
  ze schema49 trwa; nie deklarujemy jeszcze jego wyniku. Rustfmt23 zmienionych
  plików,diff check i122 Markdown/linków/anchors PASS.

- **Końcowy production CLI acceptance schema49 PASS**:1 test,797 filtered;
  peak3592929280B/max0/OOM0. [Log](evidence/2026-11-14-scoped-grants/runtime-final.log).
  Wszystkie4 suite zakończyły się exit0 i wskazują ten sam Rust source SHA;
  ponowny snapshot przed commitem potwierdził zgodność.
  [Manifest odbioru](evidence/2026-11-14-scoped-grants/verified-source.json).
  Odbiór dotyczy grantów i katalogu konfiguracji; E5.3 pozostaje otwarte,
  execution_supported=false. Formalnie nadal21/68=30,9%.
