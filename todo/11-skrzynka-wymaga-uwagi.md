# 11. Skrzynka „Wymaga uwagi”

## Cel i stan obecny

Pokazywać decyzje człowieka z wielu sesji zamiast wymagać sprawdzania każdej karty.
Inspiracje: Orca Inbox i unread activity JetBrains Air. Dashboard zna aktywność,
a permission cards są częścią rozmowy. W aktualnym working tree istnieją też
`coordination.rs` oraz `crates/coordination/`: wiadomości między agentami, run/task,
continuations i podstawowy widok. Najpierw sprawdzić ich aktualne podłączenie;
nie utożsamiać inboxu agent–agent ze skrzynką decyzji użytkownika.

## Zakres v1 i miejsca integracji

- Agregacja permission requests, pytań strukturalnych, błędów/stopów budżetu,
  planów do zatwierdzenia i wyników gotowych do review, gdy źródło je wystawia.
- Skok do dokładnego celu, odczyt/nieprzeczytane i ciche oznaczenia aktywności.
- `apps/desktop/src/{coordination.rs,harness.rs,dashboard.rs,views/sidebar.rs}`.
- `apps/desktop/src/{task_budgets.rs,projection.rs,platform.rs}` i storage.
- `crates/coordination/src/{lib.rs,store.rs,service.rs}`; adaptery relay osobno.

## Etap A — kontrakt uwagi i źródła prawdy

1. [ ] Wprowadzić AttentionItem: ID źródła, typ, scope/host, sesja/task,
   source revision, blocking/informational, created/resolved i read state.
   Odczyt nie oznacza zgody ani rozwiązania; koniec tury nie oznacza sukcesu zadania.
2. [ ] Agregować istniejące źródła, nie kopiować ich mutowalnych uprawnień.
   Run/task/continuation zachowują model koordynacji; globalna lista jest projekcją.
3. [ ] Ustalić deduplikację eventów i aktualizacji po session rekey, reconnect
   i restart. Zgoda dotyczy aktualnego requestu, nie starej karty zapisanej na dysku.
4. [ ] Pytania z dowolnego tekstu AI nie stają się automatycznie blocking request.
   Używać eventu adaptera albo jawnego ręcznego „do odpowiedzi”. Unknown pozostaje jawne.

## Etap B — inbox i nawigacja

1. [ ] Dodać listę globalną z filtrami projektu/hosta, typu i blocking/unread.
   Priorytet: zablokowana decyzja, błąd/budżet, plan, gotowe review, informacja.
2. [ ] Wiersz prowadzi do dokładnej sesji, tool requestu lub planu; nie otwiera
   pustej nowej rozmowy. Usunięty/niedostępny cel ma wyjaśnienie, nie fallback.
3. [ ] Dodać „Następna wymagająca uwagi” w rejestrze 16 i launcherze 8.
   Background event nie przenosi fokusu ani nie przestawia aktualnego zaznaczenia.
4. [ ] Pozostawić zatwierdzanie w docelowej powierzchni v1. Skrzynka nie dodaje
   „Zatwierdź wszystko”, nie uruchamia continuations i nie ukrywa per-use approval.
5. [ ] Zachować stan przeczytane/nieprzeczytane oddzielnie od resolved. Snooze
   informacyjny można dodać później, ale nie może ukryć krytycznej blokady bez śladu.

## Etap C — powiadomienia i odtwarzanie

1. [ ] Użyć istniejącej obsługi macOS notifications i ustawień TCC. Dodać politykę
   per kategoria/host, batching i rate limit; denial nie blokuje pracy w aplikacji.
2. [ ] Domyślnie powiadamiać o potrzebnej decyzji, nie o każdym tool callu.
   Zwykłe unread może pozostać badge; treść na lock screen ma bezpieczny preview.
3. [ ] Po reconnect pobrać fakty aktualnego runtime; nie odtwarzać wygasłej zgody.
   Utrata obserwacji pokazuje unknown/disconnected, a nie „agent nadal pracuje”.
4. [ ] Relay wymaga tożsamości/grantu i wersji API. Nie agregować danych innego
   członka ani nie wysyłać lokalnych powiadomień z nieautoryzowanego projektu.
5. [ ] Wiadomość agenta pozostaje niezaufanymi danymi. Oznaczenie przeczytania
   lub kliknięcie powiadomienia nigdy nie jest prawem do kolejnego requestu.

## Testy i kryteria odbioru

- [ ] Kilka agentów tworzy jedną kartę na aktualne żądanie, bez duplikatów po reconnect.
- [ ] Otwarcie itemu nie zatwierdza narzędzia, planu ani wysłania promptu.
- [ ] Zakończony/odwołany request znika z blocking, zachowując poprawny read state.
- [ ] Background event nie kradnie fokusu; klik kieruje do poprawnego host/session.
- [ ] Restart, usunięta sesja, cofnięty trust/grant i wygaśnięta continuation.
- [ ] Notification denied, wiele zdarzeń naraz i brak danych runtime są czytelne.
- [ ] Lokalny odbiór mock ACP i rzeczywisty relay mają oddzielne wyniki.
- [ ] VoiceOver oraz pełna nawigacja klawiaturą; zmierzyć kroki od badge do decyzji.

## Zależności i granice

Akcje (16); plany (18) i review (13) dostarczają później dodatkowe źródła.
Bez nowego message bus, koordynatora DAG, mobilnej aplikacji i auto-approval.

Źródła: [Orca notifications](https://www.onorca.dev/docs/notifications),
[JetBrains Air](https://www.jetbrains.com/air).
