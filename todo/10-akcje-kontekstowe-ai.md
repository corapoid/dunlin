# 10. Akcje kontekstowe AI i małe interakcje

## Cel i stan obecny

Uruchamiać właściwą pomoc przy kodzie lub błędzie, zamiast każdorazowo pisać prompt.
Inspiracje: JetBrains intentions i VS Code inline chat. Dunlin ma zwykłe akcje
rozmowy oraz dowody → draft, ale nie jednolity katalog intencji dla zaznaczenia.
Nie zakładać dostępności hosta LSP ani deterministycznych code actions: plan 7
obejmuje początkowo odczyt semantyki, nie wykonywanie edycji.

## Zakres v1 i miejsca integracji

- Wyjaśnij fragment; przygotuj minimalną poprawkę; przygotuj test regresji;
  sprawdź zgodność z zatwierdzonymi decyzjami; dodaj kontekst do aktualnego zadania.
- Pierwsza wersja przygotowuje jawny draft. Krótka odpowiedź AI to oddzielny,
  capability-gated etap; menu samo nie uruchamia agenta ani płatnego wywołania.
- `apps/desktop/src/{workbench.rs,views/chat.rs,harness.rs,workspace_panes.rs}`.
- `apps/desktop/src/git_history/{checks.rs,review.rs,context.rs}` i wspólne UI.

## Etap A — katalog intencji i spójne cele

1. [ ] Na rejestrze planu [16](16-akcje-skroty-i-fokus.md) zdefiniować action ID,
   opis, wymagany target, rodzaj skutków i warunki dostępności. Menu wiadomości
   z planu [4](04-menu-kontekstowe.md) pozostaje wejściem do istniejących akcji.
2. [ ] Target wiązać z canonical buffer/session, zaznaczeniem, diagnostyką
   i rewizją. Obsłużyć kursor bez zaznaczenia przez jawny zakres, nie cały plik.
3. [ ] Tworzyć instrukcję + karty kontekstu z planu 9; pokazać odbiorcę,
   model/tryb i zakres przed wysłaniem. Nie nadpisywać istniejącego draftu.
4. [ ] Udostępnić te same intencje w menu, palecie, toolbarze zaznaczenia
   i konfigurowalnym `⌥Enter`, wyłącznie w odpowiednim kontekście fokusu.

## Etap B — małe pytanie vs zadanie agentowe

1. [ ] Rozdzielić „przygotuj pytanie”, „wyjaśnij” i „wykonaj zmiany”. Zwykłe
   wyjaśnienie nie może niejawnie przełączyć sesji z Plan/read-only do wykonania.
2. [ ] Zbadać transport krótkiej interakcji dla obsługiwanych adapterów. Wariant
   quick answer wymaga rzeczywiście ograniczonych narzędzi/permissions; sam prompt
   „nie używaj narzędzi” nie gwarantuje bezpieczeństwa ani małej liczby wywołań.
3. [ ] Jeśli nie da się potwierdzić ograniczeń, proponować normalny draft sesji
   z jawnym ostrzeżeniem zamiast reklamować szybki, read-only tryb.
4. [ ] Wynik wyjaśnienia pokazać przy źródle z możliwością zachowania w historii
   lub otwarcia pełnej rozmowy. Powrót do źródła nie gubi zaznaczenia ani Undo.
5. [ ] Proponowana poprawka trafia do zwykłego review i kontroli uprawnień;
   nie stosować fragmentu wygenerowanego tekstu jako patcha bez walidacji.

## Etap C — deterministyczne akcje i rozszerzenia

1. [ ] Przy przyszłej integracji formattera/LSP rozdzielić akcje deterministyczne
   od AI w menu. Format i bezpieczny rename nie potrzebują modelu ani tokenów.
2. [ ] Code actions/rename/applyEdit wymagają własnego rozszerzenia po planie 7:
   capability check, preview wielu plików, wersje buforów i walidacja ścieżek.
   Nie odblokowywać ogólnego executeCommand jako skutku dodania intentions.
3. [ ] Powtarzalne prompty mogą korzystać z istniejącej biblioteki skills, ale
   ich użycie nie instaluje plików globalnie ani nie synchronizuje ich automatycznie.
4. [ ] Dodać decyzje projektu wyłącznie jako wybrane, aktualne źródła. Wynik AI
   jest oceną, nie dowodem zgodności lub gotowym zatwierdzeniem review.

## Testy i kryteria odbioru

- [ ] Menu/paleta/skrót mają ten sam zakres, dostępność i wynik działania.
- [ ] Otwarcie menu nie uruchamia narzędzi, promptu ani automatycznego odczytu repo.
- [ ] Draft nie ginie; puste zaznaczenie i stale diagnostic nie wysyłają złego pliku.
- [ ] Quick answer nie obiecuje read-only, jeśli nie ma egzekwowanego ograniczenia.
- [ ] Plan/read-only, permission ceiling i budżet pozostają zachowane.
- [ ] Korekta wielu plików, zmiana źródła podczas odpowiedzi i cancellation są jawne.
- [ ] Zmierzyć liczbę kroków od zaznaczenia do gotowego pytania na lokalnym fixture;
  bez telemetrii i bez obowiązkowego płatnego benchmarku.
- [ ] Klawiatura, VoiceOver, dark/light, IME i dwa panele mają osobny odbiór GUI.

## Zależności i granice

Akcje (16), kontekst (9), review (13); LSP (7) tylko dla późniejszych funkcji.
Nie zastępować zwykłej edycji generowaniem AI i nie budować nowego harnessa/model API.

Źródła: [JetBrains intentions](https://www.jetbrains.com/help/idea/intention-actions.html),
[VS Code inline chat](https://code.visualstudio.com/docs/chat/inline-chat).
