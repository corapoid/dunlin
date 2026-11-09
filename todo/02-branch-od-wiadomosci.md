# 2. Branch rozmowy od wiadomości

## Cel i stan obecny

Tworzyć niezależne odgałęzienie rozmowy od wskazanego miejsca bez kopiowania
całego transkryptu i bez zmiany oryginalnej sesji. Nie jest to branch Git.
Obecne Retry/Edit ponawia prompt w tej samej sesji; nie odtwarza wcześniejszego
stanu agenta. `HarnessCommand` nie udostępnia obecnie operacji fork.

## Miejsca w kodzie

- `apps/desktop/src/harness.rs`: `ChatItem`, transkrypt, Retry/Edit, dispatch.
- `crates/domain/src/lib.rs`: `SessionSummary`.
- `crates/storage/src/lib.rs`: migracje i przechowywanie transkryptów.
- `crates/agent/src/lib.rs`: komendy, możliwości i zdarzenia harnessa.
- `apps/desktop/src/views/chat.rs`, `workspace_panes.rs`, `workbench.rs`.
- `crates/relay/src/{api.rs,store.rs}` i desktopowy transport sesji.

## Kontrakt do zatwierdzenia przed kodowaniem

- Branch z wiadomości użytkownika: historia do miejsca przed tą wiadomością,
  a wybrany prompt trafia do nowego draftu. Nic nie wysyła się automatycznie.
- Branch po odpowiedzi agenta: historia obejmuje całą zakończoną turę.
- Nie rozcinać trwającego tool calla, nierozwiązanej permission ani streamingu.
- Oddzielić relację odgałęzienia rozmowy od hierarchii agent/subagent.
- Dokładny fork stanu harnessa i nowa rozmowa z odtworzonym kontekstem to dwa
  różne tryby; UI musi pokazywać, który rzeczywiście jest dostępny.

## Etapy

1. [ ] Sprawdzić możliwości aktualnych adapterów i używanej wersji ACP.
   Sam fork sesji na jej końcu nie dowodzi wsparcia dowolnej wiadomości.
   Zapisać macierz: dokładny checkpoint / kontekst odtworzony / brak wsparcia.
2. [ ] Dodać stabilne ID elementów i tur oraz wersjonowanie historii.
   Przeprowadzić zgodną migrację starych transkryptów, również importowanych.
   Nie używać indeksów UI lub samego tekstu jako identyfikatorów granicy.
3. [ ] Zaprojektować niezmienny współdzielony prefiks historii i osobny ogon
   każdej sesji, np. przez segmenty z referencjami. Punkt odgałęzienia musi
   wskazywać konkretną rewizję; dalszy streaming rodzica nie zmienia dziecka.
   Zachować zgodny odczyt i zapis dotychczasowych snapshotów.
4. [ ] Dodać transakcyjne tworzenie odgałęzienia: nowe ID sesji, relacja źródłowa,
   granica, nazwa, ustawienia oraz niezależny draft. Błąd zapisu nie może zostawić
   częściowo utworzonej sesji. Koszt/czas/requesty nowej pracy zaczynają się osobno.
5. [ ] Zintegrować dokładny fork wyłącznie tam, gdzie adapter go potwierdza.
   Nie współdzielić mutowalnego ACP session ID między niezależnymi rozmowami.
   Nie ładować starej sesji jako pozornego odgałęzienia historycznego.
6. [ ] Dla braku dokładnego forka zaprojektować jawny tryb odtworzenia kontekstu:
   preview zakresu, limit rozmiaru, oznaczenie treści jako historycznych danych,
   opis brakujących stanów harnessa i osobna zgoda na pierwszy dispatch.
   Nigdy nie replayować historycznych tool calli ani permission grants.
7. [ ] Dodać akcję do wiadomości i menu z planu 4, informację o pochodzeniu
   oraz nawigację do źródła. Zachować oryginalny draft, scroll i aktywny runtime.
8. [ ] Uzgodnić z planem 3 retencję: usunięcie rodzica nie usuwa segmentów używanych
   przez dzieci; purge sprząta tylko nieużywane dane. Eksport dziecka zawiera
   wystarczający kontekst bez zależności od istnienia rodzica u odbiorcy.
9. [ ] Dodać wsparcie relay z kontrolą właściciela, grantu, rewizji i idempotencji.
   Timeout nie powoduje ponownego utworzenia odgałęzienia ani promptu.
   Starszy relay zwraca jawny brak funkcji, bez lokalnego fallbacku.

## Testy

- [ ] Migracja starej historii, duże transkrypty i wiele odgałęzień tego samego prefiksu.
- [ ] Równe teksty wiadomości mają różne ID; refresh/streaming nie przestawia granicy.
- [ ] Zmiany w rodzicu i dziecku nie przeciekają; restart zachowuje granicę i draft.
- [ ] Usunięcie/Undo/purge rodzica nie uszkadza dziecka; osierocone dane są sprzątane.
- [ ] Mock ACP potwierdza izolację sesji i brak automatycznego prompt/tool replay.
- [ ] Błąd forka, brak wsparcia, zerwane połączenie i ponowienie żądania relay.
- [ ] Historyczny prefiks nie zwiększa rachunku dziecka ani licznika requestów.
- [ ] Dwa panele, klawiatura i odgałęzienie od dawnej, zwirtualizowanej wiadomości.

## Kryteria odbioru i granice

- [ ] Oryginalna sesja pozostaje bez zmian; zapis nie duplikuje całej historii.
- [ ] UI nie przedstawia odtworzonego kontekstu jako dokładnego stanu harnessa.
- [ ] Można utworzyć odgałęzienie bez uruchamiania agenta i ponoszenia nowego kosztu.
- [ ] Nie ma ukrytych zmian repozytorium, checkpoint restore ani branchowania Git.
- [ ] Dokładne wsparcie poszczególnych adapterów i relay ma osobny wynik odbioru.

Zależności: kontrakt retencji z planem 3, akcje z planu 4 i rozróżnienie
fork/subagent oraz kosztów odziedziczonych z planu 6.
