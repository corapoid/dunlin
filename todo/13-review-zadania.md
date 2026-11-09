# 13. Review zadania i aktualność dowodów

## Cel i stan obecny

Ocenić wynik względem celu, zmian i rzeczywistych kontroli, a nie podsumowania AI.
Inspiracje: review VS Code, komentarze do diffów Orki i JetBrains Air.
Dunlin ma jawne saved-file baselines, obserwowane zmiany, komentarz → draft,
selective one-file restore oraz raporty Checks. Brakuje spójnego review postępu,
zbiorczego feedbacku i powiązania dowodów z aktualną wersją zadania.

## Zakres v1 i miejsca integracji

- Cel, lista zmian, stan przejrzenia, raporty kontroli i nierozwiązane uwagi.
- Praca na istniejącym checkpoint/session scope; pełny task/spec dochodzi później.
- `apps/desktop/src/git_history/{review.rs,checks.rs,context.rs,impact.rs}`.
- `crates/workspace/src/{task_review.rs,task_review/,checks.rs}`.
- `apps/desktop/src/{spec.rs,knowledge.rs,remote.rs}` i trwałe dane review.

## Etap A — stan review i tożsamość zmian

1. [ ] Zdefiniować review ID powiązane z host/project/session, checkpointem
   i opcjonalnym task ID planu 17. Zmiana ma before/after hash, path i metadata;
   nie identyfikować jej wyłącznie numerem linii.
2. [ ] Oznaczenie „Przejrzane” wiązać z wersją zmiany. Każda późniejsza edycja,
   rename lub zmiana bazowego porównania unieważnia odpowiedni stan, nie cały audyt.
3. [ ] Przechowywać komentarz z hunk/range i source hash. Po drift pokazywać
   outdated/unanchored; nie przypinać komentarza do innego kodu po przesunięciu.
4. [ ] Rozróżnić observed change od autorstwa AI. Wspólny worktree i preexisting
   dirty/untracked files nie pozwalają przypisać każdej zmiany jednemu agentowi.

## Etap B — powierzchnia review i feedback

1. [ ] Po zakończeniu pracy pokazać skrót: cel, zmienione pliki, wykonane checks,
   nieweryfikowane obszary i otwarte uwagi. Zakończenie tury nie ustawia „zweryfikowane”.
2. [ ] Dodać następny/poprzedni nieprzejrzany fragment i keyboard-only workflow
   przez rejestr 16. Wąski panel używa czytelnego inline diff zamiast ścisku kolumn.
3. [ ] Zbierać kilka uwag lokalnie, z możliwością edycji/usunięcia. „Przekaż
   uwagi” przygotowuje jeden draft z kartami kontekstu (9); wysyłka jest oddzielna.
4. [ ] Po odpowiedzi agenta komentarz nie znika automatycznie. Agent może zgłosić
   naprawę; człowiek albo zdefiniowana kontrola potwierdza rozwiązanie.
5. [ ] Używać nazw „Przejrzane”, „Zapisane”, „Staged”, „Commit” zgodnie z efektem.
   Oznaczenie review nie stosuje patcha, nie zapisuje bufora i nie wykonuje Git.

## Etap C — świeżość Checks i spec-aware review

1. [ ] Raport Checks zawiera exact argv/cwd, env names, backend, czas, exit/cancel
   i tożsamość wejścia: HEAD + hash/inventory overlay, nie tylko timestamp.
2. [ ] Zmiana istotnych plików/config/dependencies oznacza raport jako stale.
   Gdy nie da się ustalić zakresu, pokazać unverifiable zamiast „testy aktualne”.
   Niezapisany bufor nie jest dowodem na zawartość przetestowaną z dysku.
3. [ ] Osobno pokazywać passed command, zakres testów i brak coverage. Brak
   parsera test runnera nie zamienia exit zero w potwierdzenie wszystkich kryteriów.
4. [ ] Po planach 17/18 dodać kryteria akceptacji i powiązane dowody. Link do
   testu lub deklaracja agenta nie dowodzi spełnienia kryterium.
5. [ ] Zatwierdzenie końcowe wiązać z dokładnym changesetem/commitem i rewizją
   planu/spec. Zmiana źródła odbiera aktualność, pozostawiając historyczny wpis.
6. [ ] Relay zachowuje serwerowe identity i freshness; klient nie oblicza
   „zdalnego” wyniku na lokalnych plikach. Stare API pokazuje brak możliwości.

## Testy i kryteria odbioru

- [ ] Oznaczenie reviewed znika po zmianie odpowiedniego pliku/hunka, a nie
  pozostaje zielone wskutek zachowania tej samej ścieżki.
- [ ] Komentarze przeżywają restart i uczciwie oznaczają utraconą kotwicę.
- [ ] Testy sprzed późniejszych edycji i testy dysku wobec dirty buffer są stale.
- [ ] Fail/timeout/cancel/output failure nie wyglądają jak poprawne sprawdzenie.
- [ ] Batch feedback nie gubi istniejącego draftu i nie wysyła się automatycznie.
- [ ] Binary, rename/mode-only i limity diffu są widoczne jako ograniczenia review.
- [ ] Klawiatura/VoiceOver, dwa panele, dark/light i wdrożony relay osobno.
- [ ] Brak automatycznego stage/commit/merge, udawanej atrybucji i self-approval AI.

## Zależności i granice

Akcje (16), kontekst (9), opcjonalnie task (17), plan (18), timeline (14).
Restore pozostaje kontraktem planu 14, bez duplikowania własnej ścieżki zapisu.
Nie przywracać PR/Actions; podpisy zespołowe i automatyczny reviewer to dalszy zakres.

Źródła: [VS Code review](https://code.visualstudio.com/docs/agents/run/review-code-edits),
[Orca annotations](https://www.onorca.dev/docs/review/annotate-ai-diff).
