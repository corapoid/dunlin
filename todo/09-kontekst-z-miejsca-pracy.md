# 9. Kontekst dodawany z miejsca pracy

## Cel i stan obecny

Przekazywać zaznaczenie, błąd, wynik komendy lub diff bez kopiowania ręcznego.
Inspiracje: kontekst VS Code/JetBrains i drag-and-drop Orki. Obecne Attach wybiera
pliki i dopisuje ścieżki do promptu; remote może przesłać plik na host projektu.
Context inspector zna klientowy envelope, ale nie cały kontekst harnessa.
Checks/Review/Impact już potrafią dodać wybrane dowody do draftu.

## Zakres v1 i miejsca integracji

- Typowane, usuwalne karty kontekstu dla pliku, zaznaczenia, diffu i raportu Checks.
- Podgląd dokładnie wysyłanej treści, rewizji i ograniczeń przed dispatch.
- `apps/desktop/src/{app.rs,harness.rs,views/chat.rs,workbench.rs,workspace_panes.rs}`.
- `apps/desktop/src/git_history/{context.rs,review.rs,checks.rs,impact.rs}`.
- `apps/desktop/src/{terminal/mod.rs,knowledge.rs,remote.rs}`; storage szkiców (15).

## Etap A — jeden kontrakt kontekstu

1. [ ] Zdefiniować attachment ID, owner session/draft, kind, scope, source ID,
   range, content hash/version, timestamp, mime i transport. Rozróżnić content
   snapshot, reference/path i plik faktycznie przesłany na relay.
2. [ ] Ustalić politykę snapshot vs live: domyślnie zachować wybrany fragment,
   a zmianę źródła oznaczać stale. Refresh jest jawny i pokazuje różnicę.
3. [ ] Wspólny builder buduje preview i rzeczywisty payload tuż przed wysłaniem,
   także dla kolejki, Retry i API. Harness-owned context pozostaje unknown.
4. [ ] Ustalić limity liczby kart, bajtów, zakresów i MIME; nie ucinać danych bez
   oznaczenia. Tokeny szacowane oznaczać jako estimate, nie model-reported usage.
5. [ ] Capability check adaptera: obrazy/zasoby tylko przy obsłudze odpowiedniego
   transportu. Tekstowy fallback wymaga widocznego formatu; sama ścieżka nie oznacza,
   że model otrzymał zawartość albo prawo odczytu poza dotychczasowym scope.

## Etap B — wejścia z powierzchni pracy

1. [ ] Dodać „Dodaj do rozmowy” w edytorze, diffie i Checks przez akcje planu 16.
   Cel to jawnie wybrany draft; zmiana aktywnego panelu nie przekierowuje odbiorcy.
2. [ ] Dodać picker plików i autocomplete odwołań w composerze; wybór znaku
   `@`/`#` nie może kolidować z komendami adaptera. Obsłużyć drag z Explorera
   i obrazy z Finder bez przenoszenia plików projektu ani cichego uploadu.
3. [ ] Pokazać karty: nazwa, pochodzenie, range, snapshot/reference, stale,
   usunięcie i podgląd. Żadna z tych czynności nie wysyła wiadomości.
4. [ ] Dodać terminalowe bloki command/output/exit/cwd z istniejącego OSC.
   Brak markerów daje ręczne zaznaczenie, nie zgadywaną granicę polecenia.
5. [ ] Dodać aktualne decyzje i artefakty z Knowledge. Stare decyzje nie są
   automatycznie podmieniane; karta wskazuje brak aktualnego zatwierdzenia.

## Etap C — bezpieczeństwo, relay i trwałość

1. [ ] Domyślnie bez automatycznego kontekstu aktywnego pliku. Ewentualny opt-in
   pokazuje stale widoczny wskaźnik i rzeczywisty zakres dołączany do żądania.
2. [ ] Rozróżnić lokalny plik, zdalne odwołanie i upload. Przed transferem pokazać
   host/docelowy projekt i limity; anulowanie oraz częściowy upload nie dodają
   karty udającej sukces. Ochronić symlinki, sekrety i ścieżki poza grantem.
3. [ ] Oznaczać logi i treść zewnętrzną jako dane niezaufane; nie wyciągać z nich
   automatycznych komend, grantów ani instrukcji zmiany uprawnień.
4. [ ] Zachować karty w draftach/kolejce zgodnie z planem 15. Wysyłka sprawdza
   istnienie źródła, adapter i dostęp; przy braku zgodności pozostawia draft.
5. [ ] Ustalić retencję snapshotów, usuwanie uploadów i ostrzeżenie o lokalnym
   przechowywaniu treści. Nie duplikować sekretów w logach diagnostycznych.

## Testy i kryteria odbioru

- [ ] Preview i wysłany client envelope mają identyczne dane dla każdego typu karty.
- [ ] Unicode/CRLF i niezapisane zaznaczenie zachowują tekst oraz właściwy range.
- [ ] Zmiana źródła, HEAD, projektu, panelu lub modelu nie podmienia kontekstu cicho.
- [ ] Adapter bez vision odmawia obrazu; timeout uploadu nie daje fałszywego sukcesu.
- [ ] Kolejka, Retry, restore draftu i dwa panele zachowują właściwego odbiorcę.
- [ ] Oversize, binarny plik, symlink, cofnięty grant i usunięty artefakt są obsłużone.
- [ ] Klawiatura/VoiceOver pozwalają dodać, podejrzeć i usunąć każdą kartę.
- [ ] Żadne otwarcie pliku/podglądu nie wysyła kontekstu do modelu automatycznie.

## Zależności i granice

Rejestr akcji (16), wspólny draft (15), wspólna semantyka wysyłki (12).
Migracja może zachować stare tekstowe Attached files; nie przepisywać historii.
Bez Design Mode, zewnętrznej przeglądarki ani automatycznej ekstrakcji sekretów.

Źródło: [VS Code — context](https://code.visualstudio.com/docs/chat/copilot-chat-context).
