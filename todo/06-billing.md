# 6. Billing i kompletność rozliczeń

## Cel i stan obecny

Rozliczać dostępne dane requestów, retry i pracy dzieci bez podwójnego liczenia,
z jawnym źródłem, jakością i brakami. Istnieją skanery Claude/Codex/OpenCode,
`Recorded / Priced / Unpriced`, kategorie pracy, raport QA, agregaty relay oraz
trwałe budżety dispatchu, czasu i obserwowanych USD. Nie budować ich od nowa.

„Pełny” oznacza kompletny opis dostępnych obserwacji, nie gwarancję zgodności
z fakturą, odtworzenie nieujawnionych opłat ani twardy provider billing cap.
Koszt wyceniony z tokenów nie musi być kwotą faktycznie pobraną z subskrypcji.

## Miejsca w kodzie

- `crates/usage/src/{billing.rs,billing/}`: skanery, ceny, dedupe, agregaty.
- `apps/desktop/src/{usage.rs,task_budgets.rs,harness.rs}`.
- `crates/domain/src/{lib.rs,task_budget.rs}`.
- `crates/agent/src/lib.rs`: obserwacje ACP i tokeny.
- `crates/relay/src/{api.rs,store.rs}`: dowody i dostęp per user/device.
- `docs/standalone-chat-worktree-usage.md`, `docs/workspace-polish-and-usage.md`.

## Etap A — poprawność obecnej implementacji (P1)

1. [ ] Zastąpić szerokie `contains` w cenniku kontrolowanym rejestrem dokładnych
   modeli i udokumentowanych aliasów. Klucz uwzględnia provider/model, wersję
   cennika, datę obowiązywania i istotne warianty taryfy. Nieznany wariant
   pozostaje `Unpriced`; nie pobiera ceny podobnie nazwanego modelu.
2. [ ] Zweryfikować oficjalne stawki oraz semantykę cache/context tiers na dzień
   implementacji. Zapisać źródło i datę w danych cennika; nie przepisywać kwot
   z historycznych notatek jako aktualnych bez sprawdzenia.
3. [ ] Poprawić tożsamość zdarzeń Codex: sama ścieżka + numer linii nie deduplikuje
   kopii logu. Preferować stabilne ID źródłowe; fallback musi być wersjonowany,
   uwzględniać sesję i odróżniać identyczne, lecz rzeczywiście osobne requesty.
4. [ ] Rozdzielić fork rozmowy od subagenta. `forked_from_id` nie jest samodzielnym
   dowodem pracy dziecka. Zachować pochodzenie i pewność przypisania.
5. [ ] Zweryfikować per adapter, czy output zawiera reasoning i czy liczniki są
   przyrostowe czy kumulatywne. Nie sumować podzbiorów jako osobnych tokenów.
   Brakujące wymagane pola usage oznaczać jako częściowe/nieznane, nie zero.
6. [ ] Dodać jawny stan nieznanego kosztu sesji. Dla starych `cost_usd = 0` nie
   zakładać, że praca była darmowa, jeśli nie ma dowodu. Oddzielić stan danych
   od liczbowej kwoty i zachować kompatybilność migracji.

## Etap B — trwały rejestr i uzgadnianie (P2)

1. [ ] Zdefiniować wersjonowany model obserwacji: źródło, provider/account scope,
   model, session/turn/request/attempt, parent/agent/task, timestamp, jednostka,
   tokeny, kwota, waluta, jakość i zakres kosztu własny/inclusive/unknown.
   Kwoty rozliczać w ustalonej precyzji całkowitej, z jawnym zaokrąglaniem.
2. [ ] Dodać storage zdarzeń, checkpointów skanowania i korekt. Import oraz
   checkpoint zatwierdzać atomowo. Zachować ograniczenia czasu/pamięci,
   anulowanie, obsługę rotacji/truncation i przyrostowego odczytu dużych logów.
3. [ ] Zdefiniować korelację ACP i logów. Obserwacje tego samego requestu mają
   jedno rozliczenie, ale zachowane dowody; nie dodawać ich do siebie.
   Bez pewnej korelacji prezentować osobne źródła i ostrzeżenie, nie fałszywy total.
4. [ ] Dodać hierarchię pracy i retry. Koszt inclusive rodzica nie sumuje się
   ponownie z dziećmi. Nieujawnione próby lub nieznany zakres pozostają widoczne.
   Fork dziedziczy kontekst, nie nowy rachunek za historyczny prefiks.
5. [ ] Wyprowadzać Usage, podział per projekt/sesja/task i porównania A/B z jednego
   uzgodnionego modelu. Rozróżniać kwoty raportowane, wycenione i nieznane oraz
   niepełne archiwa; nie przedstawiać estymaty jako dokładnego dolnego limitu.
6. [ ] Podłączyć budżety tylko do zweryfikowanych obserwacji o odpowiednim zakresie
   i świeżości. Historyczny scanner nie zastępuje live guardu. Zachować blokadę
   przy opaque child cost, błędach storage i nieznanej kwocie; nie osłabiać
   mechanizmu bezpieczeństwa, aby pozornie uzyskać kompletne rozliczenie.

## Etap C — relay, adaptery i alerty (P2)

1. [ ] Zachować istniejący ręczny upload agregatów jako jawny tryb. Same agregaty
   per device nie dowodzą rozłączności danych; nie sumować ich bez deduplikacji.
   Dla globalnych sum zaprojektować osobną wersję minimalnych danych rozliczenia
   z tożsamością źródła i zgodą użytkownika, bez treści promptów i surowych logów.
2. [ ] Odbierać dane relay w odpowiednim account/user/device scope; sprawdzać granty,
   limity payloadów i aktualność. Brak uprawnień nie uruchamia lokalnego fallbacku.
3. [ ] Dodawać adaptery pojedynczo, według rzeczywistej potrzeby i dostępnych danych.
   Nie odczytywać nowych credential stores ani automatycznie łączyć kont.
4. [ ] Dodać account/global alerty po zdefiniowaniu zakresu, waluty i okna czasu.
   Alert oparty na niepełnych danych musi to komunikować; nie jest blokadą faktury.

## Testy i odbiór

- [ ] Cennik: nieznany podobny model, alias, data, progi context i cache TTL.
- [ ] Dedupe: kopia/rename logu, append, rotacja, restart, korekta i dwa podobne requesty.
- [ ] Usage: brak pola vs prawdziwe zero, reasoning jako podzbiór i liczniki kumulatywne.
- [ ] Rodzic inclusive + dziecko, retry, fork, unknown i wiele źródeł tego samego requestu.
- [ ] Awaria między zapisem zdarzeń a checkpointem i wielokrotny identyczny import.
- [ ] Przekroczenie capów skanowania daje jawny stan częściowy i pozostawia UI responsywne.
- [ ] Budżet nie wysyła bez trwałej rezerwacji i nie przepuszcza nieweryfikowalnego kosztu.
- [ ] Relay: dwa urządzenia z tymi samymi danymi, powtórny sync, member/admin i offline.
- [ ] Ręczna kontrola zanonimizowanych rzeczywistych logów wszystkich wspieranych adapterów;
  żadnych dodatkowych płatnych wywołań bez osobnej zgody.

## Kryteria odbioru

- [ ] Każdą pokazywaną kwotę da się powiązać ze źródłem i regułą rozliczenia.
- [ ] Kopie logów, dzieci i ACP/log overlap nie zawyżają totalu.
- [ ] Nieznane i częściowe dane nie są ukryte jako zero lub pełne rozliczenie.
- [ ] Zaktualizowana dokumentacja osobno opisuje stan A, B i C; wykonanie etapu A
  nie oznacza zakończenia całego billingu.
