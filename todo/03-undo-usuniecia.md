# 3. Undo usunięcia projektu i sesji

## Cel i stan obecny

Przywracać usunięty projekt lub sesję z toastu bez dotykania plików repozytorium.
Istniejący `ToastAction` obsługuje Undo grupy. Projekty i sesje są usuwane trwale;
projekt usuwa również transkrypty i część powiązanych danych. Relay także wykonuje
trwały DELETE sesji. Sama kopia metadanych w toaście nie wystarczy.

## Miejsca w kodzie

- `apps/desktop/src/app.rs`: `ToastAction`, delete/confirm, runtime i wybór sesji.
- `crates/storage/src/lib.rs`: delete, kaskady i migracje.
- `apps/desktop/src/{workbench.rs,workspace_panes.rs}`: drafty i panele.
- `apps/desktop/src/views/sidebar.rs`, `app/overlays.rs`.
- `crates/relay/src/{api.rs,store.rs}` i desktopowe operacje remote session.

## Decyzje projektowe

- Preferować trwałe soft-delete/tombstone zamiast trzymania całej historii w RAM.
- Przed implementacją ustalić czas dostępności Undo i retencję przed purge;
  schowanie toastu nie może przypadkiem uruchamiać nieodwracalnego usuwania.
- Usunięcie lokalnego wpisu projektu relay nie usuwa serwerowego repozytorium
  ani nie zmienia grantów. Zdalne usunięcie sesji wymaga serwerowego Undo.
- Undo przywraca dane, nie działający proces, kolejkę dispatchu ani dawne grants.
- Zachować dotychczasowe granice Clear history i factory reset; nie rozszerzać
  ich o Undo bez osobnej decyzji.

## Etapy

1. [ ] Zidentyfikować pełny zbiór danych projektu/sesji i kaskady: transkrypty,
   ustawienia, grupy, evidence, approval, drafty, panele, checkpoint/budget links.
   Określić, co jest przywracane, zachowywane osobno i czyszczone przy purge.
2. [ ] Uzgodnić schemat z planem 2. Dodać znaczniki usunięcia i identyfikator
   operacji oraz migrację. Listy, startup, eksporty i wyszukiwanie muszą jawnie
   uwzględniać stan usunięcia, bez przypadkowego wskrzeszania wpisów.
3. [ ] Dodać transakcyjne delete/restore/purge. Oznaczenie usunięcia projektu
   obejmuje dokładnie jego dane; restore nie przywraca sesji usuniętej wcześniej
   niezależnie. Purge nie usuwa historii współdzielonej przez odgałęzienia.
4. [ ] Zachować blokady dirty/saving. Zatrzymać runtime i unieważnić generacje
   odczytów/zapisów przed ukryciem; opóźnione odpowiedzi i zapisy nie mogą
   odtworzyć usuniętej sesji ani dopisać do nowej sesji o podobnym tytule.
5. [ ] Rozszerzyć toast o token przywracania; obsłużyć kilka kolejnych usunięć
   przez kolejkę lub jawnie zdefiniowaną politykę. Stary toast nie może cofnąć
   nowszej operacji. Błąd restore pozostawia dane w stanie odzyskiwalnym.
6. [ ] Przywracać te same ID, historię i odpowiednie ustawienia. Jeśli grupa już
   nie istnieje, użyć jawnego bezpiecznego miejsca; nie cofać późniejszych zmian
   organizacji i uprawnień. Nie uruchamiać agenta automatycznie.
7. [ ] Obsłużyć konflikt ponownego dodania projektu o tej samej ścieżce lub ID.
   Bez cichego nadpisania i łączenia historii. Retencję i cleanup wykonywać
   również po restarcie; określić sposób ponownego dostępu do aktywnego Undo.
8. [ ] Dodać serwerowe soft-delete/restore sesji z autoryzacją właściciela,
   aktualnym grantem, ochroną lease i idempotencją. Brak funkcji w starym relay
   ma być jawny; nie obiecywać Undo na podstawie lokalnego cache.
9. [ ] Uaktualnić treści potwierdzeń: usuwane są dane aplikacji, nie katalog
   projektu. Wyjaśnić okres odzyskiwania i moment trwałego usunięcia danych.

## Testy

- [ ] Migracja i zgodność starych baz; transakcyjność przy błędzie storage.
- [ ] Sesja aktywna, nieaktywna, ostatnia w projekcie; projekt ostatni i nieaktywny.
- [ ] Dirty/saving buffer, spóźniony event, usunięcie podczas ładowania i restart.
- [ ] Usunięcie sesji, potem projektu; Undo nie wskrzesza niezależnie usuniętych danych.
- [ ] Wiele toastów, wygaśnięcie, dismiss, ponowne Undo i powtórzony restore.
- [ ] Brak folderu/grupy, ponowne dodanie projektu i zmienione uprawnienia.
- [ ] Rodzic z odgałęzieniami: delete/restore/purge bez uszkodzenia współdzielonej historii.
- [ ] Relay: owner/member, cofnięty grant, utrata odpowiedzi i aktywna sesja drugiego device.
- [ ] Porównanie plików i stanu Git przed/po: brak zmian w repozytorium.

## Kryteria odbioru

- [ ] Undo odtwarza dane i tożsamość, nie tylko pusty wpis w sidebarze.
- [ ] Utrata procesu aplikacji nie usuwa danych przed ustalonym końcem retencji.
- [ ] Przywrócenie nie wysyła promptów, nie wznawia agentów i nie rozszerza dostępu.
- [ ] Cleanup jest ograniczony i nie blokuje UI; nie ma nieograniczonej retencji w RAM.
- [ ] Lokalny i serwerowy zakres mają osobne testy oraz odbiór ręczny.

Wycena zakresu powinna uwzględniać storage i relay; nie traktować całości jako
samego małego rozszerzenia przycisku Undo.
