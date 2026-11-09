# 15. Przywracanie workspace po restarcie

## Cel i stan obecny

Po ponownym otwarciu odzyskać stanowisko pracy, nie odtwarzać automatycznie skutków.
Inspiracja: Orca session restore. Dunlin ma trwałą historię i geometrię okna,
ale kompozycja paneli, część kart, szkice i kolejka żyją w pamięci. Lokalne PTY
kończą pracę przy zamknięciu aplikacji. Historia rozmowy nie jest tożsama z żywym
runtime, a obecność rekordu sesji nie dowodzi możliwości wznowienia przez ACP.

## Zakres v1 i miejsca integracji

- Trwałe szkice z kontekstem, kolejność kart, układ paneli, fokus i pozycja czytania.
- Oczekujące prompty odzyskiwane w stanie wstrzymanym, bez replay.
- `apps/desktop/src/{workspace_panes.rs,window_state.rs,workbench.rs,chat_scroll.rs}`.
- `apps/desktop/src/{harness.rs,coordination.rs,terminal/mod.rs,app/navigation.rs}`.
- `crates/storage/src/lib.rs`, factory reset/backup i model usuwania z planu 3.
- Bez daemona, serializacji procesów/PTY i automatycznego odtwarzania komend.

## Etap A — schemat trwałego stanu

1. [ ] Zdefiniować wersjonowany WorkspaceSnapshot: host/project, panel ID,
   ordered tabs z canonical IDs, selected tab, reading anchor i layout bounds.
   Session ID jest kluczem draftu; projekt nie może scalić szkiców różnych sesji.
2. [ ] Przechowywać draft text + attachment refs (9) i pending queue (12).
   Nie zapisywać Entity/InputState, focus handles, tokenów, środowiska lub runtime.
3. [ ] Ustalić własność zapisu, debounce i atomowy commit. Kilka okien/instancji
   nie nadpisuje nowszego snapshotu bez revision/ownership check.
4. [ ] Wersje schematu migrują addytywnie; uszkodzony rekord daje recovery report
   i bezpieczny pusty widok, nie reset całej historii/projektów.
5. [ ] Zdefiniować limity/retencję i poufność draftów. Backup eksportuje je tylko
   w jawnym zakresie; factory reset/purge obejmuje nowe dane według preview.

## Etap B — zapis i odbudowa interfejsu

1. [ ] Utrwalać szkic po zmianie i flush przy przełączeniu/zamknięciu; nie
   polegać tylko na clean quit. Zmierzyć i udokumentować okno możliwej utraty po crash.
2. [ ] Po starcie najpierw załadować projekty/sesje, potem zweryfikować snapshot.
   Brakujący plik/host lub usunięta sesja nie blokuje odtworzenia pozostałych kart.
3. [ ] Canonical buffer otwierać raz, także w dwóch panelach. Pozycje i layout
   przycinać do nowej treści i bieżącego monitora; fokus przywracać po gotowości UI.
4. [ ] Reading anchor wiązać ze stabilnym ID wiadomości z planu 2 lub jawnie
   wersjonowanym uchwytem, nie indeksem okna 120 pozycji. Nie wymuszać live edge.
5. [ ] Stan review (13) odtworzyć z jego domenowego storage. Workspace trzyma
   wyłącznie wybór/powiązanie, nie kopię zatwierdzeń o innej aktualności.

## Etap C — odtworzenie bez ponownego wykonania

1. [ ] Pending queue po restarcie ma status „odzyskane — sprawdź i wyślij”.
   Reserved/submitted bez potwierdzenia pozostają delivery-unknown; użytkownik
   uzgadnia stan z adapterem, nie dostaje przycisku maskującego duplikat.
2. [ ] Nie wznawiać continuations, zgód, testów, Git i porównań. Użyć reguł
   recover z coordination zamiast ożywiać wygasłe capabilities przez snapshot UI.
3. [ ] Relay reattach tylko do potwierdzonej sesji w ramach istniejącego lifecycle
   i grantu. Brak kompatybilnego serwera daje historię/offline, bez local fallback.
4. [ ] Terminalowe placeholdery informują, że lokalny proces nie żyje.
   Odtworzenie scrollbacku, jeśli dodane później, nie odtwarza shella ani komendy.
5. [ ] Niezapisane bufory: zachować obecny quit/save protection. Opcjonalne
   crash-recovery tekstu to osobny etap z base hash, limitem i jawnym porównaniem
   z dyskiem; nigdy automatyczne save ani zastąpienie zewnętrznej nowszej edycji.

## Testy i kryteria odbioru

- [ ] Restart z kilkoma projektami, sesjami, różnymi draftami i dwoma panelami.
- [ ] Clean quit i wymuszony crash dają raportowane, kontrolowane granice odzyskania.
- [ ] Skasowany/renamed plik, sesja tombstoned, cofnięty trust i niedostępny relay.
- [ ] Migracja starszego snapshotu, ucięty zapis, pełny dysk i dwa równoległe okna.
- [ ] Kolejka/draft zachowują treść, karty i odbiorcę; żaden prompt nie wysyła się sam.
- [ ] Po restarcie nie wykonuje się test, shell, worktree creation ani Git mutation.
- [ ] Przywrócone pozycje pasują do nowych bounds i nie wymuszają powrotu na dół chatu.
- [ ] Backup/purge/factory reset uwzględniają nowy zakres bez usuwania plików repo.
- [ ] Osobny native smoke na izolowanej bazie i odbiór rzeczywistego relay.

## Zależności i granice

Kontekst (9), delivery state (12), reviewed state (13), task scope (17) opcjonalny.
V1 nie wymaga pełnego task workspace. Nie kopiować daemon-based background execution
Orki: to odrębna zmiana lifecycle wymagająca osobnej zgody i projektu bezpieczeństwa.

Źródło: [Orca session restore](https://www.onorca.dev/docs/model/session-restore).
