# 5. Composer w wąskim oknie

## Cel i stan obecny

Composer zawija treść i kontrolki bez poszerzania kolumny, obcinania Send/Stop
oraz utraty tekstu i fokusu. Obecnie textarea ma domyślny soft-wrap, a górny
i dolny pasek używają `flex_wrap`. Najpierw odtworzyć problem; nie dodawać
ponownie istniejącego mechanizmu zawijania.

## Miejsca w kodzie

- `apps/desktop/src/views/chat.rs`: `render` i `composer`.
- `apps/desktop/src/views/chat_controls.rs`: etykiety, ograniczenia szerokości.
- `crates/ui/src/controls.rs`: `composer_input`.
- `apps/desktop/src/{app.rs,workspace_panes.rs}`: obie ścieżki tworzenia textarea.
- `apps/desktop/src/workspace_ui_smoke.rs`: pomiary natywnego layoutu.
- `vendor/gpui-base-0.8.0/src/input/base/{state.rs,element.rs}`: tylko diagnostyka
  zachowania komponentu; zmiana vendora wyłącznie po wykazaniu błędu biblioteki.

## Etapy

1. [ ] Ustalić aktualne minimum szerokości okna i osobno minimum panelu rozmowy.
   Odtworzyć przypadek w pustej i niepustej rozmowie, z sidebarami i dwoma panelami.
2. [ ] Przygotować treści: długie akapity, słowo/URL bez spacji, kod, Unicode,
   emoji, wielowierszowy paste oraz długie nazwy modelu, harnessa i providera.
   Sprawdzić kolejkę i jednoczesne Stop check/Stop response.
3. [ ] Zmierzyć bounds od kolumny do textarea i pasków. Zweryfikować `min_w(0)`,
   `w_full`, shrink i padding; szczególnie nierozciągliwą grupę model + Send.
   Odróżnić overflow tekstu od overflow kontrolki i całego kontenera.
4. [ ] Naprawić najwęższe źródło problemu. Pozwolić kurczyć/truncate etykiety,
   zachowując ich pełną treść w tooltipie/AX. Kontrolki mogą przechodzić do nowego
   wiersza; Send/Stop zachowuje rozmiar celu i nie znika poza maską.
5. [ ] Sprawdzić wysokość textarea i scroll po zawinięciu. Zachować obecny kontrakt
   Enter/Shift-Enter i draftów. Jeżeli potrzebna jest adaptacyjna wysokość,
   ograniczyć ją tak, aby nie wypychała całej rozmowy i przycisków poza viewport.
6. [ ] Ujednolicić konfigurację composera głównego i tworzonego dla panelu.
   Nie wiązać wrap composera z opcją word-wrap edytora plików.
7. [ ] Dodać natywny test regresji oparty na geometrii: bounds textarea i wszystkich
   obowiązkowych przycisków mieszczą się w panelu po resize i zmianie etykiet.
   Samo znalezienie `flex_wrap` w źródle nie jest testem zachowania.

## Macierz odbioru

- [ ] Minimum okna, zwykłe okno i dwa panele; sidebar otwarty/zamknięty.
- [ ] Dark/light i wszystkie wspierane poziomy zoomu tekstu.
- [ ] Pusta historia, długa historia, kolejka i trwająca odpowiedź.
- [ ] Wklejanie, selekcja, Undo/Redo, IME i bidi w rzeczywistym GUI.
- [ ] Resize podczas pisania i streamingu bez utraty kursora, draftu i fokusu.
- [ ] Tab/Shift-Tab nadal dociera do wszystkich kontrolek w logicznej kolejności.

## Kryteria odbioru i granice

- [ ] Długi tekst nie zwiększa szerokości kolumny i nie wymaga poziomego scrollu
  do przeczytania zwykłego promptu.
- [ ] Send/Stop i pozostałe wymagane kontrolki pozostają widoczne i klikalne.
- [ ] Nie zmieniają się semantyka wysyłania, kolejki, draftów i podziału paneli.
- [ ] Brak reprodukcji jest opisany jako brak potwierdzonego błędu, nie jako
  wykonana naprawa. Testy i ręczny odbiór mogą zamknąć ten punkt bez zmiany kodu.

Nie rozszerzać tego zakresu o nowy edytor promptów lub przebudowę całego chatu.
