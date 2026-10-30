# Import sesji, historia Git i integracja macOS

- [x] Import plików sesji w ustawieniach harnessa: adaptery, podgląd, dopasowanie projektów, deduplikacja i atomowy zapis.
- [x] Graficzna historia repozytorium i tworzenie brancha z zaznaczonego commita bez checkoutu.
- [x] Własna ikona w Project settings, z możliwością powrotu do wykrywania automatycznego.
- [x] Powitanie na podstawie lokalnego czasu systemowego.
- [x] Natywne menu macOS: Settings, pliki, nawigacja i Git.
- [x] Testy, Clippy, formatowanie, nowa paczka macOS i weryfikacja podpisu.
- [x] Przegląd ROADMAP/PLAN i aktualna lista niezrealizowanych elementów.

Import nie uruchamia agentów ani nie modyfikuje źródeł. Niejednoznaczne lub
niedopasowane ścieżki wymagają jawnego wyboru projektu. Istniejące rozmowy nie
są nadpisywane. Historia Git jest lokalna, ograniczona i nie wykonuje fetch.

Walidacja: 301 testów zakończonych powodzeniem, 4 opcjonalne integracyjne
pominięte; Clippy `-D warnings`, fmt i diff check poprawne. Zbudowano
`dist/Dunlin.app`; `codesign --verify --deep --strict` poprawny (podpis ad-hoc).
Brak wizualnego testu GUI z powodu niedostępnego sterowania oknem. Nie używano
płatnych promptów ani nie modyfikowano roboczego repozytorium testowymi komendami
Git; operacje mutujące były testowane wyłącznie na izolowanych fixture'ach.
Zmiany pozostają bez commit/push. Pozostały backlog: `REMAINING-WORK.md`.
