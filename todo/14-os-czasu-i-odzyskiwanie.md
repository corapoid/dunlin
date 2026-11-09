# 14. Oś czasu zadania i bezpieczne odzyskiwanie

## Cel i stan obecny

Łączyć plan, zmiany, kontrole i review w czytelną historię z bezpiecznym restore.
Inspiracje: JetBrains Local History i checkpointy VS Code. Dunlin ma Graph/Trace
z transkryptu, jawne saved-file checkpointy i selektywne przywrócenie pliku.
To nie jest pełna Local History: nie obejmuje dowolnego czasu, wszystkich plików,
niezapisanych buforów, indeksu Git ani skutków zewnętrznych.

## Zakres v1 i miejsca integracji

- Oś zdarzeń sesji/zadania i odwołania do istniejących dowodów/checkpointów.
- Nazwany baseline przed pracą i istniejący bezpieczny restore całego pliku.
- `apps/desktop/src/{projection.rs,activity.rs,coordination.rs,knowledge.rs}`.
- `apps/desktop/src/git_history/{review.rs,checks.rs}` i `task_budgets.rs`.
- `crates/workspace/src/{task_review.rs,task_review/}` oraz storage/relay review.
- Opcjonalny auto-baseline i bardziej granularny restore dopiero po v1.

## Etap A — dziennik domenowy, nie drugi transcript

1. [ ] Zdefiniować stabilne event ID, scope, session/task, causation/source ID,
   revision/hash i source timestamp. Porządek lokalny zapewnia sekwencja, nie
   synchronizacja zegarów różnych hostów. Nie obiecywać total order bez danych.
2. [ ] Projektować: start, plan revision/approval, checkpoint, observed changes,
   check start/result, feedback, budget stop i koniec tury. AI summary to claim,
   a nie źródło prawdy o wykonaniu komendy czy poprawności rozwiązania.
3. [ ] Wykorzystać istniejące storage/coordination i odwołania do artefaktów;
   nie przechowywać drugi raz pełnych payloadów, sekretów i strumienia reasoning.
4. [ ] Ustalić retencję, limity i reguły dla brakującego/usuniętego źródła.
   Przebudowa projekcji nie tworzy fałszywych historycznych checkpointów.

## Etap B — nawigacja i baseline

1. [ ] Dodać timeline w kontekście istniejącej sesji/review, bez kolejnego
   obowiązkowego globalnego panelu. Linki otwierają właściwy plan, diff lub raport.
2. [ ] Pozwolić nazwać i porównać jawnie zapisane punkty. Pokazać, co capture
   pomija i kiedy snapshot jest partial/failed; nie nazywać go pełnym backupem.
3. [ ] Zaprojektować opt-in „baseline przed zadaniem” po planie 17. Capture ma
   limit i wynik przed dispatch; przy błędzie użytkownik wybiera kontynuację bez
   ochrony albo rezygnuje. Nie przekształcać każdego promptu w kosztowny pełny scan.
4. [ ] Utrwalać checkpoint i referencję przed potwierdzeniem sukcesu capture.
   Po crash sieroty są raportowane/porządkowane, nie automatycznie odtwarzane.

## Etap C — restore z kontrolą konfliktów

1. [ ] Wykorzystać istniejący prepare/confirm/execute restore i Git-dir lock.
   Przed zapisem ponownie sprawdzić root, file identity, bytes/mode, wiek planu,
   trust/grant oraz aktywnych writerów i wszystkie canonical buffers.
2. [ ] Zachować dzisiejszą odmowę, gdy otwarte bufory lub aktywni agenci nie
   pozwalają bezpiecznie odtworzyć pliku. Zatrzymanie agenta nie dowodzi, że jego
   proces potomny zakończył wszystkie zapisy; pokazać zakres gwarancji.
3. [ ] Jeśli plik zmienił się po prepare, odmówić i wygenerować nowy preview.
   Nie robić force/reset ani nadpisywania nowszej pracy jako „automatycznej naprawy”.
4. [ ] Zapisać wynik próby restore i stan uncertain przy utracie odpowiedzi.
   Nie ponawiać zapisu bez uzgodnienia rzeczywistego stanu.
5. [ ] Granularne hunki i częściowe odzyskiwanie wymagają osobnego etapu:
   three-way base/current/desired, jawne konflikty i potwierdzenie całego wyniku.
   Nie obiecywać transakcji wielu plików, których system nie potrafi zapewnić.

## Testy i kryteria odbioru

- [ ] Timeline prowadzi do rzeczywistych źródeł; duplicate/reordered eventy nie
  tworzą dodatkowych capture, promptów lub mutacji.
- [ ] Brak HEAD object, usunięte źródło, limit plików/bytes i partial capture.
- [ ] Preexisting dirty/untracked data nie jest błędnie przypisane agentowi.
- [ ] Stale preview, symlink/path replacement, mode-only/binary i external writer.
- [ ] Dirty bufor w drugim panelu lub innej sesji chroni przed nadpisaniem.
- [ ] Utrata odpowiedzi relay nie uruchamia retry restore; revoked grant blokuje zapis.
- [ ] Retencja nie usuwa checkpointu używanego przez aktualny review bez ostrzeżenia.
- [ ] UI mówi wprost: restore nie cofa komend, deploymentów, sieci i baz danych.
- [ ] Ręczny odbiór klawiatury, VoiceOver, dark/light i prawdziwego relay osobno.

## Zależności i granice

Tożsamość zadania (17) dla wariantu task; review (13) i plany (18) są źródłami
zdarzeń. Wczesna wersja może działać per sesja. Bez pełnej historii wszystkich
edycji, ukrytych kopii sekretów, git reset, rewind rozmowy i automatycznego rollbacku.

Źródła: [JetBrains Local History](https://www.jetbrains.com/help/idea/local-history.html),
[VS Code checkpoints](https://code.visualstudio.com/docs/agents/run/review-code-edits).
