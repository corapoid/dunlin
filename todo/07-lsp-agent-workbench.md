# 7. Semantyka LSP w agent workbench

## Cel i stan obecny

Rozszerzyć impact map o rzeczywiste dane serwera językowego, zachowując obecny
Rust AST/lexical index jako bezpieczny fallback. Aktualna implementacja nie
rozwiązuje kompletnie typów, cfg, metod dynamicznych, makr i zewnętrznych crate'ów.
Nie ma obecnie hosta LSP w module workspace ani pełnej semantycznej impact map.

## Zakres pierwszego wydania

- Rust + lokalny, jawnie wybrany rust-analyzer.
- Definicje, referencje, symbole i diagnostyka; call hierarchy, jeśli wspierana.
- Wyniki z pochodzeniem, rewizją dokumentu i jawnymi ograniczeniami.
- Bez completion, rename, code actions, automatycznych edycji i executeCommand.
- Bez automatycznej instalacji serwera i bez obietnicy dowiedzionego test coverage.
- Relay i kolejne języki jako oddzielne, późniejsze etapy.

## Miejsca integracji

- `crates/workspace/src/impact.rs`: obecny indeks, verify i candidate tests.
- `apps/desktop/src/git_history/impact.rs`: budowa, prezentacja i evidence → draft.
- `apps/desktop/src/{workbench.rs,workbench/saving.rs}`: wersje buforów i zapisy.
- Watchery plików, zmiana Git/root/trust i projektowe ustawienia runtime.
- `crates/relay/src/api.rs` i desktopowy transport impact dla późniejszego remote.
- `docs/user-guide/agent-workbench.md`: wyłącznie aktualny zakres workbench,
  bez historycznego PR/Actions.

## Etap A — kontrakt i bezpieczny host procesu

1. [ ] Zdefiniować interfejs źródeł impact evidence: lexical i LSP. Wynik zawiera
   backend, root/scope, source revision/hash, pozycje, kompletność i ostrzeżenia.
   Nie mieszać semantycznych i heurystycznych krawędzi bez oznaczenia.
2. [ ] Wybrać minimalny transport JSON-RPC stdio i własność lifecycle procesu
   poza UI. Wydzielić moduł/serwis zamiast dopisywać host do `app.rs`.
   Dodać limity wiadomości, timeouty, cancellation, bounded queues i cleanup.
3. [ ] Dodać opt-in per projekt, jawny wybór istniejącego executable i preview
   konfiguracji. Zaufanie do repozytorium nie oznacza zgody na dowolny program.
   Nie wykonywać komend serwera wskazanych wyłącznie przez plik projektu.
4. [ ] Zdefiniować politykę środowiska i dostępu do plików. Serwer jest procesem
   wykonującym kod: ograniczyć przekazywane env i nie reklamować sandboxa bez
   egzekwowania ograniczeń systemowych. Przed startem pokazać realne uprawnienia.
5. [ ] Domyślnie wyłączyć build scripts i proc-macros zgodnie z wersją serwera;
   pełniejsza analiza wymaga osobnej zgody na wykonanie kodu projektu.
   Nie rozszerzać workspace roots ani nie akceptować `workspace/applyEdit`,
   executeCommand czy nieznanych żądań serwera jako ukrytych mutacji.

## Etap B — synchronizacja dokumentów i zapytania

1. [ ] Obsłużyć initialize/capabilities i negocjowane kodowanie pozycji.
   UTF-16/UTF-8, CRLF, Unicode i emoji muszą trafiać we właściwe zakresy.
2. [ ] Wdrożyć didOpen/didChange/didSave/didClose z monotonicznymi wersjami.
   Jawnie odróżnić wyniki dla niezapisanego bufora od snapshotu plików na dysku.
3. [ ] Dodać definition/references/document symbols/diagnostics. Call hierarchy
   wyłącznie po capability check. Brak metody to unavailable, nie pusta lista
   dowodząca braku zależności. Nie mylić braku diagnostyki z udanym buildem.
4. [ ] Unieważniać odpowiedzi po zmianie dokumentu, HEAD, konfiguracji, root,
   projektu, trust i restarcie serwera. Debounce i limit równoległości chronią UI.
5. [ ] Obsłużyć indeksowanie w toku, crash, stderr, brak binarki i timeout.
   Fallback do analizy leksykalnej musi być widoczny; nie udawać semantycznego wyniku.

## Etap C — impact map i dowody dla agenta

1. [ ] Dodać wybór backendu oraz status gotowości, świeżości i zakresu analizy.
   Umożliwić skok do definicji/referencji istniejącym edytorem.
2. [ ] Budować ograniczoną mapę wpływu z dostępnych referencji i call hierarchy.
   Zaznaczać brak danych o dynamic dispatch, cfg, makrach i innych targetach.
   LSP nie zapewnia pełnego grafu całego programu ani dowodu pokrycia testami.
3. [ ] Przy evidence → draft ponownie sprawdzić wersje źródeł i backendu.
   Nie przedstawiać niezapisanego snapshotu jako danych z commitu HEAD.
   Zachować preview i ręczny dispatch; żadne zapytanie nie wysyła promptu.
4. [ ] Odbierać Rust lokalnie, zanim dojdą dodatkowe języki i zdalne procesy.

## Etap D — relay i kolejne języki

1. [ ] Na relay uruchamiać serwer wyłącznie na hoście projektu, pod właściwą
   tożsamością i grantem. Lokalny rust-analyzer nie interpretuje ścieżek serwera.
2. [ ] Zdefiniować wersjonowane, ograniczone API semantycznych zapytań i wersji
   dokumentów. Preferować allowlistę operacji zamiast nieograniczonego proxy LSP.
   Dodać kwoty zasobów, zamykanie po cofnięciu dostępu i brak automatycznego replay.
3. [ ] Ustalić zgodność ze starym relay. Lokalny fallback dla remote jest zabroniony;
   jawny zdalny backend leksykalny może pozostać oddzielną dostępną funkcją.
4. [ ] Wybierać kolejne języki według potrzeb i testować każdy adapter osobno.
   Nie zakładać identycznych uprawnień i semantyki konfiguracji różnych serwerów.

## Testy i kryteria odbioru

- [ ] Fake LSP: capabilities, nieobsługiwane metody, zła ramka, oversize, timeout,
  cancel, crash i spóźniona odpowiedź po zmianie scope/wersji.
- [ ] Fixture Rust: import aliases, typy, metody, cfg, makra, rename pliku i Unicode;
  porównać faktyczne różnice względem indeksu leksykalnego.
- [ ] Niezapisany bufor, dwa panele i zmiana backendu nie publikują starych pozycji.
- [ ] Próba applyEdit/executeCommand lub rozszerzenia roots jest odrzucana.
- [ ] Build scripts/proc-macros nie uruchamiają się bez odpowiedniej zgody.
- [ ] Brak procesu po zamknięciu zakresu, cofnięciu trust i zakończeniu aplikacji.
- [ ] UI pozostaje responsywne dla dużego repo; indeksowanie i wyniki są ograniczone.
- [ ] Semantyczny wynik, fallback, incomplete i unavailable są rozróżnialne w UI
  oraz tekście przekazywanym agentowi.
- [ ] Odbiór lokalny, rzeczywisty relay i każdy kolejny język mają osobne statusy.

Zakończenie lokalnego Rust LSP v1 nie zamyka pełnego multi-language impact map
ani dalszych funkcji edytora. Nie łączyć tego planu z rozszerzaniem Git/blame.
