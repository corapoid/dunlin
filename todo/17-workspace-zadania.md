# 17. Workspace zadania jak w Orce

## Cel i stan obecny

Zadanie łączy cel, sesje, kontekst, opcjonalny worktree, budżet i dowody w jednym
przepływie. Nie każda wiadomość staje się formularzem lub nowym checkoutem.
Dunlin ma subtask worktrees i A/B comparison, Spec tasks `T###`, sesje i budgets.
W working tree `coordination::Task` grupuje runy właściciela, lecz nie jest jeszcze
pełnym workspace. Najpierw sprawdzić integrację, a nie tworzyć kolejny model Task.

## Zakres v1 i miejsca integracji

- Lekkie lokalne zadanie: cel + istniejąca sesja + opcjonalne kryteria/linki.
- Osobny etap: kreator przygotowania i jawnego uruchomienia w wybranym worktree.
- `apps/desktop/src/{coordination.rs,subtask_worktrees.rs,task_budgets.rs,spec.rs}`.
- `apps/desktop/src/git_history/{subtasks.rs,compare.rs,review.rs,checks.rs}`.
- `crates/coordination/src/{lib.rs,store.rs}` i `crates/workspace/src/subtask.rs`.
- `apps/desktop/src/{project_runtime_settings.rs,workspace_panes.rs,remote.rs}`.

## Etap A — tożsamość i relacje zadania

1. [ ] Uzgodnić jeden domenowy task ID i owner authority/project. Spisać mapowanie:
   task ↔ spec document hash + `T###`, sessions/runs, worktree, checkpoint,
   budget ledger, plan revisions i review. Nie używać samego `T###` globalnie.
2. [ ] Zdecydować, czy rozszerzyć `coordination::Task`, czy utrzymać jawne
   powiązanie z agregatem workspace; zapisać jednego właściciela każdego faktu.
   Zachować rozróżnienie agent-reported success, turn finished i human verified.
3. [ ] Migracja istniejących rejestrów jest addytywna. Nie zmieniać właściciela
   runów ani nie przypisywać kosztu/historycznych sesji do zadania heurystyką.
4. [ ] V1: jeden owner session, kolejne jego runy i jawne evidence links.
   Wielu wykonawców/role/DAG wymagają późniejszego rozszerzenia kontraktu, nie
   poluzowania walidacji scope w istniejącym coordinatorze.
5. [ ] Tworzenie zadania i przypięcie istniejącej sesji nie wysyła promptu,
   nie uruchamia procesu i nie wymaga spec/governance pluginu.

## Etap B — lekki workflow i wybór miejsca pracy

1. [ ] Dodać „Utwórz zadanie” z draftu/rozmowy i „Dodaj do zadania” dla kontekstu.
   Minimalny formularz: cel; reszta to opcjonalne rozszerzenie/progressive disclosure.
2. [ ] Widok zadania łączy istniejące Context, Plan, Review, Checks i Budget,
   a nie duplikuje tych paneli. Launcher 8 i inbox 11 prowadzą do jego stabilnego ID.
3. [ ] Wybór execution: bieżący katalog, istniejący worktree lub nowy worktree
   z dokładnego commitu. Pokazać równoległych writerów i dirty baseline; nie tworzyć
   worktree dla zwykłego wyjaśnienia kodu i nie czyścić istniejącej pracy.
4. [ ] Wskazać harness/model/mode, skuteczny env provenance i budżet. Nie
   zapisywać wartości sekretów w task record. Niewspierane opcje są unavailable.
5. [ ] Przeniesienie zadania na inny host/root wymaga nowego przygotowania,
   potwierdzenia i sprawdzenia dostępu; samo przepięcie linku nie przenosi sesji.

## Etap C — prepare, confirm i jawny start

1. [ ] Zbudować niezmienny preview: cel/prompt, scope, commit/root, branch/path,
   konfiguracja, budżet i skutki. Osobno wybrać „Przygotuj” oraz „Uruchom”.
2. [ ] Tworzenie worktree idzie przez istniejący Git plan, lock i guards.
   Rejestr operacji przechowuje przygotowane/utworzone/częściowe/failed/unknown.
   Operacje DB + filesystem nie są jedną transakcją; nie obiecywać rollbacku.
3. [ ] Po częściowym błędzie zachować pliki i czytelny stan do reconciliation.
   Nie usuwać automatycznie katalogu, projektu lub gałęzi, które ktoś mógł zmienić.
4. [ ] Start przechodzi przez wspólny dispatch (12), świeżą konfigurację i budżet.
   Zmiana preview unieważnia zgodę. Restart nie wznawia startu ani płatnego promptu.
5. [ ] Opcjonalny baseline korzysta z planu 14. Worktree izoluje checkout,
   ale nie jest sandboxem dla shella, sieci, sekretów i katalogów poza projektem.

## Etap D — zakończenie, cleanup i relay

1. [ ] Wynik zadania prowadzi do review 13 i kryteriów; agent zgłasza claim,
   człowiek zatwierdza dokładną wersję dowodów. Bez automatycznego merge/push.
2. [ ] Archiwizacja metadanych jest oddzielna od usuwania worktree/branch.
   Cleanup wymaga fresh Git preview, ochrony dirty/untracked/buffers i potwierdzenia.
3. [ ] Relay dostaje tę samą semantykę po osobnym audycie grants i wersji API.
   Nie przenosić lokalnych env/credentials ani nie robić lokalnego fallbacku.

## Testy i kryteria odbioru

- [ ] Zadanie bez spec/worktree działa z istniejącą sesją i nie uruchamia agenta.
- [ ] ID i linki nie kolidują między projektami/hostami oraz identycznymi `T###`.
- [ ] Dirty repo, branch collision, blokada w innym worktree i wielu writerów.
- [ ] Crash po utworzeniu katalogu, przed zapisem registry i przed/po dispatch.
- [ ] Zmiana modelu/root/HEAD/permissions po preview wymusza ponowne przygotowanie.
- [ ] Agent claim, zakończenie tury i zweryfikowany wynik mają różne statusy.
- [ ] Usunięcie/archiwizacja nie kasuje repo ani nie unieważnia dowodów bez informacji.
- [ ] Lokalny native smoke z mock ACP; relay i prawdziwy cleanup odbierane osobno.

## Zależności i granice

Minimum akcji (16) i istniejące task/worktree/budget. Schemat A jest fundamentem
planów 13/14/18/19, ale ich wersje sesyjne nie czekają na cały kreator.
Bez koordynatora DAG, fan-out do N agentów, środowisk VM/container i auto-merge.

Źródło: [Orca worktrees](https://www.onorca.dev/docs/model/worktrees).
