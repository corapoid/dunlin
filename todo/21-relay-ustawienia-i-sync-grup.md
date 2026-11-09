# 21. Relay — aktualność ustawień i synchronizacja grup

## Cel i stan obecny

Audyt working tree: 2026-11-09. Oddzielić odświeżanie istniejących ustawień
projektu od nowej synchronizacji organizacji między urządzeniami. Priorytet:
P1 poprawność/odbiór runtime, P2 pełny sync i zdalne Apply saved model.
Status i odbiór prowadzić w roadmapzie #3/#25/#54, nie w drugim rejestrze ukończenia.

Już istnieją: lokalne dziedziczenie globalne → grupy → projekt, ręczna kolejność
grup, kolory/ikony, zapis model/env projektu na relay (admin), odczyt dla członka
z grantem, model dla nowej sesji i env następnego procesu na serwerze.
Lokalne Apply saved model używa ACP bez promptu/restartu; bieżący model można też
wybrać w composerze. Nie implementować ponownie tych funkcji.

W ramach audytu dodano odświeżanie model/env przy wejściu w projekt i otwarciu
ustawień zamiast tylko pierwszego dodania; przy nawigacji odczyt kończy się przed
importem listy sesji/automatycznym połączeniem. Odpowiedź nie zastępuje lokalnego
wyglądu/harness override/uprawnień, otwartego formularza ani nowszych wartości.
To nie jest wersjonowany sync: obecne GET/PUT nie mają CAS/revision, a odświeżenie
może zawieść lub zostać pominięte podczas edycji. Rzeczywisty host wymaga odbioru.

## Etap A — aktualność i kontrakt własności ustawień

1. [ ] Wydzielić jawny DTO runtime na relay od lokalnego `ProjectSettings`.
   Lokalny wygląd, membership, client approval i trust nie podróżują przy zapisie
   model/env. Serwerowa polityka wykonania jest osobną, egzekwowaną granicą.
2. [ ] Dodać revision/ETag i warunkowy zapis; brak rewizji/stary serwer oznacza
   ograniczoną dostępność, nie bezwarunkowy overwrite. Błąd konfliktu zostawia
   draft i pozwala porównać/zaktualizować baseline bez ujawniania env w logach.
3. [ ] Zdefiniować stany loading/fresh/stale/unavailable oraz generation ID dla
   odczytów i zapisów. Przed edycją poczekać na świeży baseline; spóźniony odczyt
   nie nadpisuje zapisu, nawet gdy wartości zmieniły się A → B → A.
4. [ ] Przed nową zdalną sesją/reconnect zweryfikować świeżość defaults. Przy
   błędzie nie wysyłać promptu z po cichu wybranym starym/fallback modelem.
   Samo wznowienie starej sesji nie stosuje domyślnego modelu.
5. [ ] Odświeżać po ponownym połączeniu i zmianie na drugim urządzeniu; użyć
   ograniczonego mechanizmu invalidation/pull, nie stałego pełnego pollingu.
   Usunięcie hosta, zmiana grantu/tokena lub retarget projektu unieważniają operacje.

Miejsca: `apps/desktop/src/{app.rs,project_runtime_settings.rs,remote.rs,harness.rs}`,
`crates/relay/src/{api.rs,acp.rs,store.rs}`. Zweryfikować faktyczny podział store
przed zmianą schematu; nazwy nowego DTO/rewizji/API wymagają osobnego kontraktu.

## Etap B — synchronizowana organizacja v1

1. [ ] Ustalić opt-in workspace katalogu **użytkownik + relay**, bez automatycznej
   publikacji całej lokalnej bazy. Pierwsze włączenie pokazuje preview import/merge;
   nie zastępuje lokalnych grup ani nie scala ich po samej nazwie.
2. [ ] Wprowadzić stabilne ID grup, parent, nazwę, kolor/portable icon, membership
   i jawny porządek rodzeństwa. Mapować projekty po tożsamości relay, nie lokalnym
   UUID, nazwie lub ścieżce. Obce/brakujące projekty nie stają się nowymi grantami.
3. [ ] V1 synchronizuje grupy wyłącznie tego relay. Lokalne/mieszane grupy i grupy
   innych hostów pozostają lokalne; konwersja wymaga preview. Nie wysyłać lokalnych
   ścieżek, custom command, credentials, trust, historii ani env jako metadanych UI.
4. [ ] Przechowywać lokalne mapowanie/revision i trwałą kolejkę zmian w transakcji.
   Operacje muszą być idempotentne; restart/offline/retry nie tworzy duplikatów.
5. [ ] Obsłużyć konkurencyjne rename/move/reorder/delete przez CAS i jawny konflikt,
   nie ślepy last-write-wins. Walidować cykle, brak rodzica, limity i poprawną
   kolejność przy równoczesnej zmianie na dwóch urządzeniach.
6. [ ] Dodać tombstones i reguły retencji. Delete/Undo grupy korzysta z istniejącej
   semantyki promocji członków i zachowuje późniejsze przeniesienia; usunięcie grupy
   nigdy nie usuwa repozytorium/sesji. Stary klient nie wskrzesza skasowanej grupy.
7. [ ] Dane organizacji są per-user; serwer ponownie sprawdza urządzenie/granty
   przy każdej operacji. Snapshot dla jednego użytkownika nie ujawnia katalogu
   innego. Ikony mają limity formatu/rozmiaru i nie pobierają zewnętrznych URL.

Miejsca: desktop `project_tree.rs`, `project_groups.rs`, `group_settings.rs`,
`app.rs`, `remote.rs`; storage oraz relay API/store. Nie używać formatu backupu
jako gotowego protokołu sync — backup celowo wyklucza runtime i dane wrażliwe.

## Etap C — współdzielone defaults grup, oddzielnie od organizacji

1. [ ] Dopiero po B zdefiniować serwerowe grupy runtime i dziedziczenie. Edycja
   model/env wymaga administratora; zmiana osobistego układu grup nie może sama
   zmieniać serwerowej polityki lub env innych użytkowników.
2. [ ] Jawnie oddzielić grupę UI od grupy polityki/runtime albo wymagać osobnego
   potwierdzenia ich powiązania. Nie materializować inherited wartości w projektach.
3. [ ] Resolver i walidacja scalonego env działają autorytatywnie na relay;
   lokalne env grup nigdy nie jest automatycznie wysyłane. Zachować blokady
   reserved env, brak sekretów w podsumowaniach oraz limity całego dziedziczenia.
4. [ ] Move/delete/Undo ponownie wylicza przyszłe defaults, lecz nie restartuje
   procesów, nie zmienia bieżącego modelu i nie poszerza zamrożonych group contexts.
   Zmiany server permissions odbierać oddzielnie od client approvals.

## Etap D — Apply saved model na relay

1. [ ] Udostępnić istniejącą jawną akcję również dla aktywnej sesji zdalnej,
   po świeżym odczycie efektywnego default dla faktycznego harnessu serwera.
2. [ ] Współdzielić walidację z lokalnym `live_model_target`: idle/ready,
   advertised model, aktywne filtry, oczekiwanie na ACK i brak fallback.
3. [ ] Pinować host/project/session/generation do ACK; przełączenie zakładki,
   odmowa modelu lub zerwanie połączenia nie zmienia innej sesji ani defaults.
   Nie wysyłać promptu i nie restartować procesu. Hot-reload env poza zakresem.

## Testy i odbiór

- [ ] Runtime: dwa urządzenia, odczyt po cudzym zapisie, równoległe PUT, stary
  serwer, offline, grant revoked, member/admin i brak lokalnego fallback.
- [ ] Opóźnione GET/PUT, A → B → A, zmiana projektu/hosta, kasowanie projektu,
  otwarty draft i lokalny wybór wyglądu/Read-only nie tracą danych ani ochrony.
- [ ] Sync: pierwsze parowanie z istniejącymi grupami, ponowne podłączenie,
  retry po restarcie, rename/move/reorder konflikt, cykl, delete/Undo i tombstone.
- [ ] Migracja zachowuje lokalne grupy/kolejność, sessions/transcripts i defaults;
  nie podnosi automatycznie istniejących lokalnych grup do współdzielonego scope.
- [ ] Model: dozwolony, filtered, unsupported, rejected, busy i zmiana generacji;
  dowód braku promptu/restartu i zachowania parked sessions.
- [ ] Fizyczny odbiór #3: klawiatura/fokus, Save/Cancel, walidacja bez wycieku env,
  minimum okna i oba motywy; następnie test dwóch klientów na disposable relay.
  Zapisać build klienta/serwera/protokołu, PASS/FAIL/DEFERRED i obserwacje.

## Kolejność i zależności

Najpierw A i odbiór istniejących defaults, potem mały D, następnie B i osobno C.
B nie czeka na serwerowe dziedziczenie runtime. Odbudowa schematu synchronizacji
nie blokuje lokalnego FTS z [planu 08](08-search-everywhere.md) ani skills z
[planu 20](20-skills-frontmatter-i-odbior.md). Powiązać retencję z
[Undo](03-undo-usuniecia.md); nie budować kolejnej kopii historii/backupów.
Referencja obecnego kontraktu: [runtime defaults](../docs/reference/project-runtime-settings.md).
