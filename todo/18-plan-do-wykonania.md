# 18. Przepływ plan → wykonanie z VS Code/Junie

## Cel i stan obecny

Zatwierdzać konkretny plan i świadomie przechodzić do wykonania bez kopiowania
tekstu między rozmowami. Dunlin ma ACP modes, Spec workspace i hash-bound decisions.
Spec gate jest advisory na granicy composera, nie sandboxem całego harnessa.
W koordynacji istnieją zatwierdzane continuations; nie traktować ich automatycznie
jako modelu zatwierdzania dowolnych planów lub zgody na każdą przyszłą operację.

## Zakres v1 i miejsca integracji

- Plan z rewizjami, kryteriami, ryzykami, feedbackiem i decyzją człowieka.
- Akcje: popraw plan, odrzuć, zatwierdź bez wykonania, przygotuj wykonanie.
- `apps/desktop/src/{spec.rs,views/chat_controls.rs,harness.rs,coordination.rs}`.
- `apps/desktop/src/{knowledge.rs,markdown_editor.rs,project_runtime_settings.rs}`.
- Task z planu 17, governance file index i istniejące decision/evidence storage.
- Nie wymagać Spec Kit; Markdown pozostaje kanoniczny dla planu zapisanego w pliku.

## Etap A — rewizja planu i kontrakt zatwierdzenia

1. [ ] Zdefiniować PlanRef: task/session, dokument/artifact ID, hash exact bytes,
   scope, source version, HEAD/baseline, kryteria i status. SQLite przechowuje
   indeks oraz decyzje, nie konkurencyjną kanoniczną kopię pliku Markdown.
2. [ ] Obsłużyć plan zewnętrzny, utworzony ręcznie i dostarczony przez adapter.
   Gdy agent zwraca tylko tekst, jawne „Zachowaj jako plan” tworzy wersjonowany
   artefakt; nie zgadywać pełnej struktury na podstawie dowolnej listy w odpowiedzi.
3. [ ] Stany: draft → in review → approved/rejected → prepared execution.
   Każda zmiana treści unieważnia approval; historyczna decyzja pozostaje widoczna.
4. [ ] Zgoda ma aktora, timestamp, hash, scope i konkretne ograniczenia. Nie
   może pochodzić z tekstu agenta, jego toola ani zapisu pola status przez API.
5. [ ] Zmiana HEAD/root lub kontraktu wymaga jawnego przeglądu drift przed startem.
   Nie odświeżać automatycznie hash-bound approval na nową rewizję.

## Etap B — planowanie i feedback

1. [ ] Pokazać kartę planu i otwarcie pełnej treści w istniejącym Markdown editorze.
   Plan dla małego zadania może być krótki; nie narzucać dziesięciu faz Spec Kit.
2. [ ] Feedback ogólny i uwagi do sekcji wiązać z rewizją. Niewysłany feedback
   blokuje zatwierdzenie albo wymaga jawnego odrzucenia uwag, nie ich utraty.
3. [ ] Rozdzielić „Zapisz”, „Poproś o poprawę”, „Zatwierdź bez wykonania”
   i „Przygotuj wykonanie”. Żadna akcja podglądu/zapisu nie przełącza trybu agenta.
4. [ ] Pokazać realną ochronę Plan/read-only adaptera. Jeśli narzędzia/opaque
   shell nie są egzekwowane przez klienta lub sandbox, nie twierdzić, że prompt
   „nie zmieniaj plików” zapewnia read-only. Przy unknown zablokować taki claim.
5. [ ] Reuse istniejących spec decisions/gates i permission ceiling; nowa karta
   nie może omijać niezatwierdzonych reviewer checklists lub otwartych pytań.

## Etap C — bezpieczne przejście do wykonania

1. [ ] Przygotować exact prompt + approved plan revision, kontekst 9, task/root,
   model/harness/mode i budget. Pokazać podsumowanie skutków i zakres poleceń.
2. [ ] Approval planu nie jest approval toola, auto-approve, Git ani deploymentu.
   Ewentualna zmiana trybu wymaga osobnego świadomego wyboru i potwierdzenia ACP.
3. [ ] Po acknowledged mode/config jeszcze raz sprawdzić hash, unsaved editor,
   HEAD/root/grants i guards, a dopiero potem dispatch przez plan 12. Wcześniejsza
   zgoda nie może uruchomić nowej treści po zmianie pliku w tle.
4. [ ] „Zatwierdź bez wykonania” zapisuje tylko decyzję. „Wykonaj” może złożyć
   jawne przygotowanie i potwierdzenie startu w jeden czytelny dialog, ale nie
   dodaje ukrytej kontynuacji ani kolejki po restarcie.
5. [ ] Handoff do innego modelu/sesji przekazuje zweryfikowaną rewizję i wybrane
   dowody, nie udaje pełnego fork kontekstu. Nowy scope sprawdza aktualność/grants;
   zmiana uprawnień nie wynika z faktu użycia „modelu wykonawczego”.
6. [ ] Delivery-unknown wymaga reconciliation; approval nie jest biletem do
   nieograniczonych retry. Zakończenie wraca do kryteriów i review 13.

## Testy i kryteria odbioru

- [ ] Edycja planu w Dunlinie lub zewnętrznie unieważnia zatwierdzenie.
- [ ] Unsaved plan, pending feedback i stale spec gate blokują start.
- [ ] Zatwierdzenie bez wykonania nie wysyła promptu i nie zmienia mode/permissions.
- [ ] Adapter odrzuca mode change: brak startu i zachowany draft/plan.
- [ ] Podwójny klik, restart między approval a dispatch i niepewne dostarczenie.
- [ ] Agent/API nie wystawia human approval; brak capability nie daje cichego fallbacku.
- [ ] Wykonanie innej rewizji/root/modelu wymaga poprawnego recheck/preflight.
- [ ] Plan ad hoc bez Spec Kit i plan plikowy mają spójną semantykę.
- [ ] Native mock ACP, VoiceOver, narrow/dark/light; realny relay osobnym etapem.

## Zależności i granice

Task identity (17), akcje (16), kontekst (9), dispatch (12), inbox (11).
Bez autopilota, autonomicznego podnoszenia uprawnień, self-approval i nowego formatu
spec zastępującego Markdown. Odbiór lokalny nie zamyka remote authoring governance.

Źródła: [VS Code planning](https://code.visualstudio.com/docs/agents/run/planning),
[Junie modes](https://www.jetbrains.com/help/ai-assistant/junie-agent.html).
