# 4. Menu kontekstowe wiadomości i tool calli

## Cel i stan obecny

Udostępnić pod prawym przyciskiem te same bezpieczne akcje, które są dostępne
w kontrolkach rozmowy. Wiadomości użytkownika mają Copy/Edit/Retry, odpowiedzi
agenta Copy, a diff narzędzia Copy diff. Nie ma wspólnego menu aplikacyjnego.
Istniejące menu zaznaczonego tekstu nie jest jego zamiennikiem.

## Zakres akcji

| Element | Akcje v1 |
| --- | --- |
| Wiadomość użytkownika | Kopiuj prompt, Edytuj w composerze, Ponów prompt |
| Odpowiedź agenta | Kopiuj odpowiedź |
| Tool call | Kopiuj szczegóły, Kopiuj diff jeśli istnieje, Rozwiń/Zwiń |
| Wiadomość z prawidłową granicą | Branch od wiadomości dopiero po planie 2 |

Retry oznacza nową turę w bieżącej sesji, nie historyczny fork. Edit wypełnia
draft i nie wysyła. Nie dodawać ponownego wykonania tool calla, automatycznego
restore plików ani innych nowych mutacji pod pozorem menu.

## Miejsca w kodzie

- `apps/desktop/src/views/chat.rs`: user/agent/tool cards i widoczne przyciski.
- `apps/desktop/src/harness.rs`: `edit_prompt`, `retry_prompt` i dispatch guards.
- `apps/desktop/src/workspace_panes.rs`: aktywna sesja i panel.
- `apps/desktop/src/git_history/graph.rs`: istniejący wzorzec `ContextMenuExt`.
- `crates/ui/src/controls.rs`: wspólne kontrolki i dostępność.

## Etapy

1. [ ] Wydzielić identyfikator celu i wspólną obsługę akcji wiadomości/narzędzia.
   Zarówno przyciski, jak i menu wywołują ten sam kod oraz warunki dostępności.
2. [ ] Powiązać cel z sesją, panelem i stabilnym ID elementu, nie samym indeksem
   wiersza. Przy braku jeszcze ID z planu 2 użyć zweryfikowanego uchwytu snapshotu
   i unieważniać menu po zmianie rewizji; nie zgadywać celu na podstawie tekstu.
3. [ ] Dodać menu na kartach, zachowując zaznaczanie tekstu, linki i zwykły click.
   Menu nie może powodować rozwinięcia karty lub zmiany aktywnej sesji ubocznie.
4. [ ] Przed wykonaniem ponownie sprawdzić istnienie celu i aktualny stan runtime.
   Stosować te same blokady budget/model/loading/provider i zachowanie kolejki
   co przy istniejących przyciskach; nie tworzyć alternatywnej ścieżki dispatchu.
5. [ ] Rozwiązać konflikt istniejącego draftu przy Edit/Retry zgodnie ze wspólną
   polityką przycisków. Nie naprawiać zachowania tylko w menu i nie tracić draftu
   cicho. Ewentualną zmianę polityki objąć osobnym testem obu wejść.
6. [ ] Zdefiniować kopiowany format: tekst promptu/odpowiedzi, widoczne szczegóły,
   istniejący unified diff. Brak diffu nie generuje sztucznego pustego patcha.
   Nie dodawać odczytu ukrytych payloadów, sekretów ani nowych zapytań do harnessa.
7. [ ] Zapewnić klawiaturowe otwieranie menu, nawigację i Escape z przywróceniem
   fokusu. Dodać etykiety AX i wyjaśnienia dla akcji niedostępnych.
8. [ ] Po planie 2 podłączyć branchowanie do tego samego systemu akcji, wyłącznie
   dla poprawnej granicy tury i obsługiwanego trybu harnessa/relay.

## Testy

- [ ] Równość zachowania i blokad przycisk vs menu dla Copy/Edit/Retry.
- [ ] Zmiana sesji, projektu, panelu i rewizji podczas otwartego menu.
- [ ] Streaming, wcześniejsze wiadomości, identyczne teksty i aktualizacja tool calla.
- [ ] Brak nowego requestu po samym otwarciu menu; jeden request po Retry.
- [ ] Read-only/importowana historia nie omija zasad zwykłego dispatchu.
- [ ] Zaznaczony tekst i linki pozostają używalne; click nie wywołuje dwóch akcji.
- [ ] VoiceOver, Tab, klawiatura menu, Escape, dark/light i wąskie dwa panele.

## Kryteria odbioru

- [ ] Każda widoczna pozycja działa na właściwym elemencie albo odmawia po jego zmianie.
- [ ] Menu i kontrolki nie mają rozbieżnych zasad bezpieczeństwa.
- [ ] Nie ma akcji replay tool call ani automatycznych mutacji plików/Git.
- [ ] Branch nie jest reklamowany przed implementacją planu 2.

Plan może być wykonany przed zmianą storage; zależność od stabilnych ID należy
obsłużyć jawnie, bez blokowania małego zakresu kopiowania i istniejących akcji.
