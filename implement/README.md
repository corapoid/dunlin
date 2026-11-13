# Dunlin Web — plan implementacji

Aktualizacja: 2026-11-13. Status całości: **w realizacji, nieukończone**.

Ten katalog jest planem wykonawczym nowej aplikacji. Istniejący desktop i jego
backlog w `docs/project/roadmap.md` pozostają odrębne. Obecność kodu nie oznacza
odbioru. `[x]` oznacza wykonanie i podaną weryfikację; `[ ]` pozostaje otwarte.
Nie oznaczamy etapu jako ukończonego po samym utworzeniu szkieletu.

## Uzgodnione wymagania

- Self-hosted, zespół od początku, wiele hostów macOS/Linux.
- Ruch między klientami/serwerami Dunlin na różnych komputerach **wyłącznie
  przez prywatny Tailscale Serve HTTPS/WSS**. Na jednej maszynie dozwolony
  numeric loopback lub localhost HTTP/WS. Brak publicznego HTTPS, bezpośredniego
  LAN, Funnel, tuneli SSH i konektora omijającego Tailscale.
- Przeglądarka używa jednego originu. Serwer pozostaje na loopback za Serve.
  Koordynator kontaktuje się z innymi hostami również przez Tailscale.
- WebSocket jest podstawowym transportem live. REST służy bootstrapowi,
  logowaniu, plikom i konfiguracji, nie cyklicznemu odświeżaniu UI.
- AHP synchronizuje sesje, ACP steruje agentami, MCP udostępnia narzędzia,
  A2A obsługuje niezależnych agentów. Własny bootstrap WebSocket nie jest AHP.
- Wyłącznie darmowe ReUI i shadcn; Echo/Flowly/Mesh są inspiracją kompozycji,
  nie źródłem płatnego kodu. Marketing `web/` zostaje; aplikacja: `apps/web/`.
- Nie przebudowujemy teraz UI macOS. Nie osłabiamy istniejących grantów,
  sandboxów, keyringu, zasad hostowych kont Git ani kompatybilności API v1.
- Zewnętrzne API Linear/GitHub/modeli/MCP mogą wymagać publicznego HTTPS;
  ograniczenie Tailscale dotyczy połączeń infrastruktury Dunlin, nie zamiany
  publicznych usług dostawców w usługi dostępne wyłącznie w tailnecie.
- Żadnych zmian w realnym tailnecie, poświadczeniach ani płatnych usługach bez
  odrębnej zgody. Akceptacja na produkcyjnym koncie pozostaje osobną bramką.

## Architektura

```text
browser ── WSS przez Tailscale Serve / lokalne WS ── Dunlin Server
                                                       ├─ identity / grants
                                                       ├─ projekty i hosty
                                                       ├─ AHP / runtime ACP
                                                       ├─ workflow / scheduler
                                                       ├─ integracje / vault
                                                       └─ HTTPS/WSS Tailscale
                                                               └─ inne hosty
```

Role `standalone`, `controller`, `worker` są odpowiedzialnościami jednego produktu,
nie obowiązkowym zestawem mikroserwisów. Serwer jest autorytetem stanu; browser
przechowuje tylko preferencje/szkice. Zamknięcie browsera nie kończy zatwierdzonej
pracy. Restart bez możliwości wznowienia harnessu daje `interrupted/unknown`,
nie automatyczne ponowienie promptu. SQLite na lokalnym dysku i jeden koordynator
na pierwsze wydanie; HA/PostgreSQL poza tym zakresem.

## Plan i zadania

### E0 — decyzje i badanie integracji — zrealizowano
- [x] E0.1 Zapisać kontrakt Tailscale-only + loopback i role protokołów.
- [x] E0.2 Zweryfikować oficjalne Tailscale MCP i granice tworzenia sieci.
- [x] E0.3 Opisać transport live, odtwarzanie stanu i granice zaufania.
- [x] E0.4 Zapisać macierz ekranów, model domenowy i dwa workflow.

Odbiór: dokumenty `architecture.md`, `tailscale.md`, `product.md` są zgodne z
wymaganiami. Bez twierdzenia o zakończeniu prototypu AHP.

### E1 — fundament aplikacji i kontrakt live — w realizacji
- [x] E1.1 Utworzyć niezależny pakiet `apps/web` z React/TypeScript/Vite.
  Metadane, locked install, typy, testy TS i produkcyjny build zweryfikowane.
- [x] E1.2a Dodać publiczny registry ReUI i pierwszy darmowy Frame na podstawie
  rzeczywistego źródła/API oraz zachować licencję MIT. Adaptacja bez nowych dependencies.
- [ ] E1.2b Rozwinąć komponenty przez ReUI MCP/skill i przeprowadzić ich kontrolę
  na renderowanym interfejsie. Nie zastępować tego atrapą premium UI.
- [x] E1.3 Dodać testowany walidator endpointów: Tailscale HTTPS/WSS lub loopback.
- [x] E1.4 Zdefiniować wersjonowany bootstrap live, limity, heartbeat i resync.
- [x] E1.5 Bezpieczne browser pairing, HttpOnly cookie, expiration, logout,
  kontrola Origin/Host, revocation. Bez sekretów w URL/localStorage/logach.
- [x] E1.6 Pierwszy rzeczywisty przekrój: pairing → WebSocket → projekty i sesje
  bieżącego użytkownika; brak fikcyjnych projektów i liczników.
- [x] E1.7 Serwowanie statycznego builda z Rust, same-origin dev proxy,
  dokumentacja uruchomienia i CI (typy, testy, build).

### E2 — tożsamości i zespoły
- [x] E2.1 Workspace, członkostwa, role owner/admin/member/reviewer/viewer.
  Migracje/API, CAS ról, last-owner i testy izolacji; OIDC/granty hostowe nadal otwarte.
- [ ] E2.2 OIDC, bezpieczny bootstrap administratora, zaproszenia i sesje.
  Trwałe browser credentials (hash/expiry/revocation/restart) zweryfikowane;
  Zaproszenia dla istniejących użytkowników mają migrację/API/UI. OIDC code
  flow z PKCE/nonce, jawnym bindingiem i bezpiecznym bootstrapem lokalnego admina
  przeszedł testy fixture i Chromium. Odbiór rzeczywistego IdP oraz provisioning
  nowych tożsamości przez zaproszenia nadal otwarte.
- [ ] E2.3 Projektowe i hostowe granty; osobne service principals.
  Granty projektowe oraz dedykowane service credentials z expiry/revocation
  są zaimplementowane. Natywne granty mają tombstones/revision i fencing
  actor/ACP, FS/Git/PTY i source/derived jobs. Prepared restore/comparison/checks,
  prywatne artefakty i publikacja administracyjnych clone/copy mają świeże,
  przypięte uprawnienia. Legacy sesje/transkrypty i konfiguracja projektu mają
  wspólny Store cut z walidacją proof; settings PUT wymaga świeżej roli admina.
  Obsługiwane zadania administracyjne, ich output/cancel i usage/billing mają
  świeżą authority urządzenia; guardian kończy cofnięte procesy. Testy lokalnych
  grup procesów, backendu i Chromium PASS. Remote host grants oraz service
  authority wykonania workflow nadal otwarte; odbiór realnych kont osobno.
- [x] E2.4 Prywatne i współdzielone sesje, audyt, odwoływanie dostępu.
  Jawne ACL sesji interaktywnych: pełna historia tylko do odczytu, bieżące granty
  obu stron, CAS/audyt, trwałe visibility fencing i revoke/regrant snapshot.
  HTTP/runtime/real WS i Chromium PASS; legacy historie pozostają prywatne.
- [ ] E2.5 Izolacja tożsamości Git/agentów; nie odblokowywać hostowego `gh`
  dla członków przez usunięcie obecnego ograniczenia.

### E3 — serwerowy runtime i AHP
- [ ] E3.1 Oddzielić proces ACP od lifecycle połączenia WebSocket.
  Eksperymentalny actor serwera uruchamia ACP niezależnie od obserwatorów;
  odczyt AHP jest podłączony; odbiór rzeczywistego harnessu nadal otwarty.
- [ ] E3.2 Serwerowe transkrypty, kolejka promptów, polityki i budżety.
  Wdrożono dziennik/kolejkę/fencing/recovery jako wewnętrzną warstwę persistence;
  actor ACP z guardianem grantów/checkoutu i limitami czasu/wyjścia jest wdrożony
  eksperymentalnie. Transport AHP i jednorazowe zgody narzędzi wdrożone; budżety kosztowe otwarte.
- [ ] E3.3 Implementacja wydanej wersji AHP, projekcja ACP → AHP i conformance.
  Journal zapisuje typed AHP chat actions/snapshot przez oficjalny SDK/reducer
  1.0.0, atomowo z ACP; root/session/chat, prywatny transport WebSocket,
  listowanie i React są wdrożone. Queued text dispatch ma trwały origin/dedupe
  i optimistic reconciliation. Inne operacje i pełny conformance nadal otwarte
  (w tym agregacja statusu SDK).
  Adapter browsera porównuje pięć traces krok po kroku z rzeczywistym pinned
  reducerem SDK; metadata, terminal gates i timestamp/flags poprawione.
  Znana rozbieżność surowego SDK dla partial reorder jest jawnie testowana;
  backend już emituje pełny canonical order zgodny z SQL/live/snapshot/retry.
- [x] E3.4 Reconnect/replay/snapshot, sekwencje i idempotency; limity buforów.
  Wewnętrzny źródłowy stream dzienników ma trwały wspólny porządek, spójny
  snapshot wielu prywatnych sesji i bounded replay. AHP ma wspólny porządek
  akcji, reconnect/replay/snapshot i trwałą negocjację klienta. Client action
  dedupe dla wspieranych akcji wdrożone. Jawny SubscribeView.turns/fetchTurns,
  ograniczone strony, trwały requested view i świeży paged reconnect są testowane
  SDK/WS/Chromium. REST create z Idempotency-Key ma atomowy ledger/session/lease,
  immutable receipt i retry bez nowego actora; legacy unkeyed to osobny zamiar.
  Realny Linux SIGKILL/relaunch tej samej DB, retry REST i promptu bez powtórzeń
  oraz Clippy PASS. Profile/formularz ukończone w E5.1; pełny conformance, zasoby
  dużej treści oraz odbiór platform poza Linux pozostają w E3.3/E3.7/E10; dowody i granice w progress.md.
- [ ] E3.5 Wielu obserwatorów; arbitraż tur, anulowań i jednorazowych zgód.
  Dwóch obserwatorów zweryfikowano prawdziwym WS/SDK; serial queued text
  ma trwałe potwierdzenia, retry i mapping ID. Edycja/usuwanie/reorder kolejki
  oraz fenced ACP cancel wdrożone. Jednorazowe zgody mają nonce, first-wins,
  trwały outcome, delivery fencing i recovery unknown; testy prywatnych sesji
  właściciela PASS. Jawne współdzielenie odczytu ma ACL/revocation; pozostały
  conformance i współdzielona kontrola wykonania nadal otwarte.
- [x] E3.6 Restart/recovery bez automatycznego powtarzania niepewnych promptów.
  File-reopen/recovery/dedupe oraz rzeczywisty SIGKILL/relaunch binarnego relay
  z bubblewrap i tą samą DB: Unknown/Queued, brak powtórzonego efektu, stary nonce
  odrzucony, świeża jawna sesja działa. Dowód Linux i ograniczenia opisano
  w progress.md i runtime-checks.md; odbiór macOS/harnessów i operator resume otwarte.
- [ ] E3.7 UI rozmowy: streaming, narzędzia, zgody, kontekst, diff i artefakty.
  Panel pokazuje text/reasoning/narzędzia/błędy; composer wysyła queued text
  do istniejącego actora; obsługuje kolejkę, anulowanie i jednorazowe zgody. Steering,
  kontekst, diff i artefakty otwarte.

### E4 — wiele hostów przez Tailscale
- [x] E4.1 Katalog hostów z enrollmentem, odwołaniem i capabilities.
  Prowizoryczny rejestr aplikacyjny, jednorazowy bilet, osobne poświadczenie
  workera, CAS/epoch, trwałe odwołanie nawet przy pełnym audycie i raporty
  capabilities. Store/HTTP, produkcyjny panel, push revocation i Chromium PASS;
  domena, frontend/typecheck oraz Clippy PASS. Claim dowodzi posiadania biletu;
  wszystkie wpisy pozostają unverified/execution_ready:false. WSS i rzeczywista
  tożsamość sieciowa są osobnymi E4.3/E4.7. Kontrakt: [hosts.md](hosts.md).
- [ ] E4.2 ProjectBinding: logiczny projekt → host/repo/checkout.
  Lokalny adapter ma jawne native grants, CAS, odwołanie i pin katalogu/ustawień.
  Worker ma osobny trwały katalog exportów/grantów operatora, z CAS/tombstones,
  pinami zwykłego Git i jawnie zatwierdzonych linked worktree roots. Store/FS,
  dwie SQLite connections i binarny CLI PASS; nie są to jeszcze zdalne bindingi.
  Wdrożono metadata DescribeExport przez negocjowany WSSv2, controller CAS
  bindings i panel. Store/HTTP/WSS, Chromium, runtime/bwrap i Clippy PASS.
  V1 zachowuje diagnostykę.
  Typed BindingRef/resolver i linked export/reopen→WSS→binding→Member resolve
  mają backend/Chromium, runtime i końcowy Clippy PASS.
  Lokalny katalog metadata approval/grant i jawny operator-local CLI/check
  oraz migracja33 mają pełny backend, actual binary CLI/bwrap i Clippy PASS.
  NativeCheckout/NativeGit authority, historia/prepared Git i legacy ACP mają
  backend405, workspace189, actual Linux bwrap/guardian i Clippy PASS.
  Metadata roots pozostają oddzielne od workspace FS/vault/attachments.
  Jobs/PTY/interactive i local linked binding mają authority i migrację34:
  backend433, workspace189, cztery actual Linux bwrap acceptance (również
  lingering held metadata FD/blocked stdin), frontend121 i typecheck PASS.
  Chromium12 i rzeczywisty Mac kernel proof pełnego wygenerowanego profilu
  metadata RO/RW/denials PASS (bez hostowego buildu Rust). Native Mac Rust test
  nadal niewykonany; końcowy tree/Clippy po integracji E4.4 otwarte. Zakres
  profile-only proof oraz nieudane próby zapisano w progress.md.
  Wykonanie zdalne należy do E4.4.
  Kontrakt operatora: [worker-exports.md](worker-exports.md). Kontrakt rozwoju: [remote-bindings.md](remote-bindings.md).
- [x] E4.3 Uwierzytelniony kanał controller–worker tylko przez Serve WSS.
  Worker CLI, prywatny Unix LocalAPI dial bez fallbacku, TLS, reverse nonce proof
  oraz aktualne credentials/epochs zweryfikowano przez rzeczywiste TLS/WSS fixture.
  Adapter dotyczy Unix systemowego daemona; Mac GUI port/password oraz odbiór
  platform pozostają otwarte. Dwie realne maszyny/policy/Funnel należą do E4.7/E10.
  Kontrakt i granice: [worker-channel.md](worker-channel.md).
- [x] E4.4 Heartbeat, leases/fencing, kolejki i limity równoległości.
  Działają oddzielne v3 execution i opt-in v4 persistent sessions: prywatne WSS,
  trwałe admission/turn/stream ledgery, świeże Describe/authority cuts, niezależne
  guards, bounded serial queue, wspólne host reservations i canonical stop/ACK.
  Remote/Profile REST i AHP zachowują kontekst jednego procesu ACP między turami.
  Worker Linux ma rzeczywisty guardian/sandbox oraz kernel lock przed recovery.
  Pełny snapshot `b528fbb4`:548 relay/67 Journal/189 workspace/15 kernel checks PASS; osobny
  Chromium `cf346502` PASS17,43s: Remote pin, utrata202/identyczny retry, reload
  i kontekst, rzeczywisty packet loss/reorder, edycja/reorder kolejki i aktywne
  anulowanie. Worker sessions2/turns5/reservation0/ack12 oraz martwy potomek;
  dwie queued tury nie ruszyły. UI rozróżnia potwierdzone stop od Unknown wyniku.
  Domyślnie wyłączony opt-in zachowuje Remote501. Remote cancellation kończy
  sesję, a remote tool approvals są unsupported. Snapshot `d3f24933` ma oba
  dokładne Journal/ACK crash cuts z SIGKILL/relaunch obu service i SIGSTOP/CONT
  kontrolera PASS42,14s oraz all-targets Clippy `-D warnings` PASS1m53s.
  Unknown/Queued bez replay, nowe boots i worker ACK9/reservation0 w obu
  wariantach; source stop jest wymagany do zwolnienia rezerwacji. Dowody i
  granice: [progress.md](progress.md), [remote-session-plan.md](remote-session-plan.md)
  oraz [runtime-checks.md](runtime-checks.md). Realne platformy/konta/tailnet to
  osobne otwarte bramki E4.7/E10, nie wynik fixtures.
- [x] E4.5 Panel hostów: kondycja, wersje, agenci, drain, diagnostyka.
  Podgląd live zaakceptowanych kanałów i runtime counts jest zaimplementowany;
  frontend128, osobny tsc, Vite, runtime i rzeczywisty Chromium v4 heartbeat PASS.
  All-targets Clippy snapshotu health PASS. Końcowa regresja registry browser
  PASS7,89s: enrollment, worker claim/report, CAS/drain/revoke, scope i sekrety.
  Odbiór dotyczy panelu i izolowanych fixtures; realny tailnet jest w E10.7.
  Zakres i dowody: [hosts.md](hosts.md), [progress.md](progress.md).
- [ ] E4.6 Opcjonalny onboarding przez oficjalny Aperture MCP po zgodzie człowieka.
- [ ] E4.7 Izolowane tagi/granty; walidacja polityki i brak Funnel; nie używać
  `.ts.net` jako dowodu prywatności. Akceptacja na dwóch realnych maszynach.

### E5 — agenci, MCP i A2A
- [x] E5.1 Wersjonowane definicje agentów, harness/model, skills/instrukcje.
  Szkice CAS, niezmienne publikacje/digest, ograniczone literalne skills i audyt;
  atomowe przypięcie wersji do sesji oraz weryfikacja model/mode przez ACP.
  Produkcyjny Agent Builder i formularz sesji zweryfikowane w Chromium, także
  utrata HTTP202 i jawny identyczny retry bez drugiego actora. Domena, Store/REST,
  runtime, frontend, build/typecheck oraz Clippy PASS. Kontrakt i granice:
  [agent-profiles.md](agent-profiles.md); raport capabilities i realny harness
  pozostają w E5.5/E10, MCP/A2A/scoped secrets w kolejnych punktach E5.
- [x] E5.2 MCP 2026-07-28 i adapter zgodności 2025; discovery, metadata,
  MRTR, subscriptions, OAuth i schematy z limitami.
  Metadane i modern-first transport z jawnym adapterem2025 mają testy/Clippy PASS.
  MRTR, subskrypcje, prywatny cache TTL, discovery i PKCE/code/refresh/revoke
  mają testy/Clippy PASS. Dodano trwały dziennik OAuth w SQLite z generacjami,
  CAS i blokadą refresh przed siecią. Adapter SecretStore i browser status/disconnect
  mają testy:113 MCP +6 HTTP +31 vault PASS. Dodano operatorowy katalog providerów,
  start/callback powiązany z sesją przeglądarki i migrację40: pełny584 relay
  oraz Clippy PASS. Odbiór i snapshoty: progress.md.
  Panel nazwanych prywatnych połączeń i lokalny pełny OAuth w produkcyjnym UI
  mają odbiór Chromium na dwóch sites, także utratę odpowiedzi i spóźniony start
  po opuszczeniu panelu. Jawne sprawdzenie katalogu działa przez broker OAuth,
  z odświeżaniem lokalnej expiry i kontrolą authority/generacji przed/po HTTP/cache.
  Input/output JSON Schema mają lokalny profil referencji i limity grafu/regex;
  oba transporty wymagają pinu katalogu i walidują argumenty oraz wyniki.
  SDK obsługuje typowane form MRTR z accept/decline/cancel; biblioteka domyślnie
  nie reklamuje elicitation. Runtime jawnie włącza form. Migracja41 przypina
  jednorazową decyzję AHP do oryginalnego MCP permit, a publiczny dispatch
  używa dokładnego FormSchema::validate. Runtime19/Chromium desktop/mobile
  obejmuje wszystkie płaskie typy,2 rundy, korektę błędnej odpowiedzi, dedupe,
  decline/cancel bez content/efektów i odłączenie konta podczas formularza.
  Client/Pending i requestState pozostają host-only, z niezmiennym deadline.
  Publiczny profil CIMD ma jawną konfigurację i eksport dokumentu do osobnego
  hostingu HTTPS. Broker wykonania jest podłączony do lokalnego i zdalnego
  runtime; pełny form odbiór dotyczy zdalnego Linux fixture. Trwałe odwołanie
  zapisanych tokenów ma owner-only REST/UI, receipt i jawne ponowienie z zachowaniem
  potwierdzeń oraz postępu. Odbiór używa kontrolowanego AS i produkcyjnego helpera.
  Lokalne sprzątanie potwierdzonych tokenów ma osobny REST/UI i trwały dziennik;
  chroni bieżącą generację i pozwala ponowić delete po spóźnionym zapisie.
  Jawne rozstrzyganie nowych unfinished ma owner-only REST/UI i dowód zakończenia
  lokalnego workera przez kernel lease; bez powtarzania HTTP lub odczytu sekretów.
  Retencja/odzyskiwanie limitu i URL elicitation zatwierdzonych tools/call mają
  odbiory opisane poniżej. Legacy unfinished ma jawne operatorowe show/recover
  po deklaracji shutdown, bez nowych potwierdzeń AS/write. Prawdziwi dostawcy/
  platformy pozostają odbiorem E10. Historyczny odbiór formularzy: backend810/runtime19/Clippy, frontend147 i TypeScript
  PASS. Odwołanie tokenów: backend822, MCP/HTTP/vault151, frontend150, jeden
  Chromium/helper case oraz Clippy/TypeScript PASS. Dokładne źródła i granice
  obu odbiorów są w progress.md.
  Cleanup: backend829, MCP151, frontend152, Chromium/helper1 i końcowy
  Clippy/TypeScript PASS; szczegóły i granice również w progress.md.
  Reconcile: backend835, frontend153, Clippy/TypeScript oraz Chromium/helper1
  PASS. Procesowy SIGKILL i przygotowany browser cut są osobnymi dowodami.
  Migracja45 oznacza nowe zapisy chronione writer lease. Callback i oba brokery
  trzymają go przez zapis/refresh, zwalniają przed MCP HTTP; utrata waitera go
  nie zwalnia, a disconnect nadal blokuje stary commit. Backend840, bieżący
  Clippy/Chromium i tooling14+18 PASS; runtime CGU128 bez zwiększenia RAM.
  Sam lease nie potwierdza zakończenia backendu sekretów. Migracja46 dodaje
  jednorazowy claim i trwały wynik set, również po utracie dodatniej authority,
  bez reaktywacji połączenia. Backend846, MCP153, Clippy, Chromium/helper1
  i tooling14+18 PASS na bieżącym źródle; frontend153/TypeScript bez zmian.
  Server/runtime CGU256+strip mieszczą się w dotychczasowych RAM/scratch.
  Migracja47 implementuje odzyskiwanie limitu256, trwałe potwierdzenia AS przy
  referencjach i ograniczoną retencję512 bezpiecznych/256 niepewnych wpisów.
  Odbiór: backend851, MCP153, frontend155, TypeScript, końcowy Clippy
  i wybrany Chromium/helper1 PASS. Manifest wiąże źródła oraz zakres;
  nie jest to pełny odbiór całego E5. Niepewne
  wpisy nie wygasają automatycznie. Szczegóły/zakres: progress.md.
  URL elicitation tools/call: backend853, MCP153, frontend157, TypeScript,
  końcowy Clippy i23 zdalne acceptance z Chromium PASS, w tym4 nowe URL cases
  i dotychczasowe formularze. UI pokazuje pełny adres/host, otwiera po kliknięciu
  bez opener/referrer; osobna decyzja nie zawiera content. Odmowa, anulowanie
  i odłączenie podczas input nie tworzą efektu narzędzia w fixture. To nie
  pełny conformance MCP.
  Końcowy odbiór E5.2: backend860, MCP/HTTP/vault153, produkcyjny CLI1 i
  root all-targets Clippy PASS; frontend157/TypeScript bez zmian. Macierz wymagań
  i manifest wiążą te wyniki z wcześniejszymi odbiorami form/URL/OAuth. MRTR
  dotyczy tylko tools/call,prompts/get,resources/read; discovery/list ma zgodnie
  ze specyfikacją nieinteraktywny wynik. Zewnętrzne konta/keyring/tailnet/native
  platform pozostają E10; ogólne scoped secrets i Builder/A2A w E5.3–E5.5.
  Aktualny zakres: [mcp-client.md](mcp-client.md), wyniki w [progress.md](progress.md).
- [ ] E5.3 Katalog narzędzi MCP, allowlisty, scoped secrets i kill switch.
  Dziennik runtime ma oddzielne jednorazowe zgody MCP z przypięciem całego
  wywołania, blokadą replay i hostową przestrzenią tool IDs chronioną przed ACP.
  SDK zgód ma74 testy PASS. Schema2 profilu przechowuje dokładne allowlisty
  provider/name/definition digest; prywatny katalog publikuje piny narzędzi, a
  Agent Builder pozwala zapisać/opublikować politykę. Lokalny runtime Unix ma
  broker HTTP, stdio proxy i jawne prywatne powiązania połączeń sesji; odbiór
  poszczególnych scenariuszy jest w progress.md. Ogólne scoped secrets
  i pełny odbiór kill switch pozostają otwarte.
  Lokalny vault otrzymał bieżące sprawdzanie authority po blokujących krokach,
  odrzucenie wyniku po odwołaniu oraz watchdog grupy procesu receptury;
  odbiór MCP/HTTP/vault163, backend861 i Clippy PASS. Nie stanowi to jeszcze
  scoped permitów managed runtime/runnera; szczegółowe granice w progress.md.
  Rdzeń biblioteczny scoped credentials ma osobny namespace, niezmienny
  envelope policy/value, jednorazowy permit HTTP i kontrolowane testy174+2
  compile-fail oraz package Clippy PASS. Journal SQL schema48 i cookie API
  provisioning/status/disable mają odbiór backend875, MCP174+2, CLI1 i Clippy
  PASS. Authority SQL użycia, profilowe grants i integracja managed runtime/runnera pozostają
  otwarte; API konfiguracji jawnie zgłasza execution_supported:false.
  Wspólny rdzeń ma hostowy adapter authority i testy niezmienności pinów oraz
  zachowania pojemności po anulowaniu waitera. Proponowany kontrakt v5 wiąże
  MCP z sesją/turą i dzieli odpowiedzi na ograniczone fragmenty. Negocjacja Hello5
  wymaga osobnych flag obu stron, realnego runtime i producenta sesji;
  produkcyjny zdalny przepływ MCP ma kontrolowany odbiór REST/AHP/HTTP/runtime.
  Osobne kodeki Hello5/open i pasywne przyjęcie sesji przypinają ofertę limitów
  oraz wymagają własnej polityki operatora schema2/mcp_broker:true. Runner ma
  bounded dispatcher z korelacją, timerem i zachowaniem slotu wysłanej operacji
  po anulowaniu waitera. Jest podłączony do zweryfikowanego WSS v5 i opaque
  connection runtime. Natywny odbiorca open5, Unix proxy i konfiguracja MCP
  procesu agenta mają10 produkcyjnych scenariuszy runtime PASS, w tym cofnięcie
  Execute, limit odpowiedzi i utrata lokalnego odbiorcy. Testy używają
  kontrolowanego hostowego respondera, bez konta OAuth ani zgody SDK w tej ścieżce.
  Controller SQL ma oddzielny zapis open v5 i prywatnych pinów kont, rollback,
  retry bez nowego lease oraz recovery bez dispatch. Publiczny endpoint
  przyjmuje prywatne selections, a aktor instaluje broker przed open5.
  Odczyt pierwotnego receipt nie wymaga żywego transportu ani nowego lease.
  Wewnętrzna fabryka hosta łączy już prywatne konto, jednorazową zgodę SDK
  i rzeczywisty HTTP MCP; ma odbiór siedmiu przypadków oraz regresję backendu.
  Typowana kolejka kontrolera zachowuje open v5 i koreluje jego odpowiedzi;
  Actor po instalacji brokera korzysta ze scoped SQL i typed open5. Import
  potwierdzeń, renew, kolejka i projekcja wymagają żywego dowodu. Pending
  queue nie instaluje prawa dispatch; AHP approval wymaga udanej fabryki.
  Runtime11/11 PASS obejmuje pełny REST/AHP→checked OAuth/vault→HTTP MCP→runner
  z dwiema turami i utratą ACK, bez powtórzenia Submit. Formularz zdalnego MCP
  ma jawny wybór/reset kont, przypięty retry oraz odbiór Chromium desktop/mobile.
  Backend803, runtime11 z oboma browser branches i końcowy Clippy PASS.
  Reconcile-before-stream kończy ACK do zweryfikowanego stopu bez późniejszej
  projekcji tekstu. Rozszerzona regresja runtime15/15 z oboma Chromium branches
  obejmuje odmowę zgody (dalsze dwie tury, zero HTTP calls), utratę odpowiedzi
  po przyjęciu HTTP i disconnect konta podczas HTTP (Unknown, jedno wywołanie,
  stop/ACK i reservation0 bez replay). Dodatkowy wymuszony terminal-vs-renew
  potwierdza uzgodnienie odrzuconego RenewTurn wyłącznie po canonical terminal
  i świeżej authority, bez ponowienia operacji ani wydłużenia budżetu.
  Formularze MRTR mają odbiór runtime19/Chromium opisany w E5.2. Odwołanie
  zapisanych tokenów ma trwały journal/receipt, REST i UI. Osobny lokalny cleanup
  zachowuje niepewne tombstones i ochronę nowej autoryzacji. Retencja odzyskuje
  pojemność z ograniczoną historią; URL tools/call ma standardowy AHP i jawne
  decyzje UI. Legacy unfinished ma operatorowy odbiór opisany w E5.2.
  Ogólne scoped secrets i realny dostawca pozostają otwarte; niepewne dawne
  write outcomes zachowują unknown bez dopisywania potwierdzeń. Nowe unfinished można jawnie rozstrzygnąć
  dopiero po uzyskaniu lease wykluczającego żywego workera.
  Dowody i granice odbioru zapisano w progress.md.
- [ ] E5.4 A2A: discovery, zadanie, stan, dodatkowe wejście, cancel i artefakty.
- [ ] E5.5 Agent Builder, test definicji i czytelne unsupported capabilities.

### E6 — Linear i GitHub
- [ ] E6.1 Linear OAuth/refresh/revoke; osobiste i zespołowe połączenia.
- [ ] E6.2 Jeden model sync; mapowania zespołów, statusów, labeli i projektów.
- [ ] E6.3 Komentarze, dokumenty, załączniki i dwukierunkowe aktualizacje.
- [ ] E6.4 Prywatne wdrożenia używają pollingu integracyjnego; webhook wymaga
  osobnej decyzji architektonicznej, nie wystawiamy publicznie Dunlin/Funnel.
- [ ] E6.5 GitHub App, krótkotrwałe credentials, PR/reviews/checks/merge.
- [ ] E6.6 Deduplikacja, naprawcza synchronizacja, outbox i konflikty zmian.

### E7 — trwały silnik workflow
- [ ] E7.1 Definicje, wersje, typed ports/schemas, walidacja grafu.
  Model, walidator, CAS drafts i immutable versions są testowane; pełna
  walidacja provider JSON Schema i integracja z wykonaniem pozostają otwarte.
- [ ] E7.2 Triggery ręczne/Linear/harmonogram i trwałe uruchomienia/kroki/próby.
- [ ] E7.3 Kroki ACP/MCP/A2A/workspace/condition/parallel/join/wait/review.
- [ ] E7.4 Ograniczony Repeat dla pętli poprawek, budżet i timeout.
- [ ] E7.5 Scheduler, leases, dedupe cross-host, rezerwacje i retry per typ operacji.
- [ ] E7.6 Recovery, unknown outcomes i ręczne uzgodnienie; brak obietnicy exactly-once.
- [ ] E7.7 Akceptacje związane z wersją artefaktu/SHA i opcją separation of duties.
- [ ] E7.8 Dry-run na fixtures, historia i wyjaśnienie blokad.

### E8 — interfejs projektów i edytor workflow
- [ ] E8.1 Projekty: karty/tabela/tablica, filtrowanie i aktywność.
- [ ] E8.2 Projekt: przegląd/zadania/sesje/workflow/runs/artefakty/ustawienia.
- [ ] E8.3 React Flow OSS + darmowe ReUI; typed connections i panel konfiguracji.
- [ ] E8.4 Draft/publish/version diff; konflikt edycji bez cichego nadpisania.
- [ ] E8.5 Graf wykonania live, logi, błędy, approvals, cancel/retry/reconcile.
- [ ] E8.6 Wszystkie empty/loading/error/offline/forbidden states, keyboard,
  light/dark, reduced motion i dostępność.

### E9 — oba scenariusze z diagramu
- [ ] E9.1 Publisher: Linear → Writer → wersjonowany artykuł → review/poprawki.
- [ ] E9.2 Publisher: Developer → repo → zatwierdzona publikacja → potwierdzony URL.
- [ ] E9.3 Product: issue → izolowany workspace → Developer → checks → PR.
- [ ] E9.4 Product: komentarze → poprawki → zgoda na SHA → merge → Linear Done.
- [ ] E9.5 Weryfikować rezultat; `end_turn` i sam merge nie dowodzą wdrożenia.

### E10 — wydanie i odbiór
- [ ] E10.1 Testy kontraktów, domeny i browser end-to-end z fixtures ACP/MCP/A2A.
- [ ] E10.2 Testy dwóch użytkowników i hostów, restartu, duplikatów, odwołań,
  braku uprawnień, stale approvals, lag/backpressure i unknown mutations.
- [ ] E10.3 Safari/Chrome/Firefox, a11y, duże transkrypty i grafy.
- [ ] E10.4 Installer i upgrade bez utraty API v1/danych desktopu.
- [ ] E10.5 Backup/restore bazy + artefaktów + keyringu i test odtworzenia.
- [ ] E10.6 Dokumentacja operatora, ograniczeń izolacji i diagnostyki Tailscale.
- [ ] E10.7 Realny tailnet/Serve/keyring/OAuth/harness acceptance po zgodzie właściciela.

## Zależności i zasady ukończenia

E0 → E1 → E2/E3 → E4 → E5/E6 → E7 → E8 → E9 → E10.
Proste UI powstaje z działającym backendem; pełny graf nie wyprzedza runtime'u.
Każdy ukończony punkt musi mieć wpis w `progress.md`: zmiany, komenda testu,
wynik i nieweryfikowane granice. Zmiana statusu etapu wymaga ukończenia wszystkich
jego punktów i odbioru. Nie oznaczamy testów manualnych jako wykonanych przez CI.

## Otwarte bramki zewnętrzne

- Uwierzytelnione konto ReUI MCP/skill do wyszukiwania i walidacji komponentów.
- Własny testowy tailnet i jawna zgoda na provisioning/policy mutations.
- Testowe integracje Linear/GitHub/OIDC oraz odblokowany keyring.
- Sandbox Docker został uruchomiony i zweryfikowany; odbiór realnych integracji
  nie wynika z zielonych testów izolowanych.

Szczegóły architektury: [architecture.md](architecture.md).
Badanie Tailscale: [tailscale.md](tailscale.md).
Zakres produktu: [product.md](product.md).
Rzeczywisty postęp: [progress.md](progress.md).

Odtwarzanie testów rzeczywistego runtime: [runtime-checks.md](runtime-checks.md).
