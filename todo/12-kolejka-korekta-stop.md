# 12. Kolejkuj, skoryguj, zatrzymaj

## Cel i stan obecny

Rozróżnić trzy intencje nowej wiadomości w czasie pracy agenta: po zakończeniu,
korekta przy bezpiecznej granicy oraz anulowanie i nowy request. Inspiracja:
mechanika chat VS Code. Dunlin już ma edycję/reorder/remove kolejki i cancel turn;
nie implementować drugiej kolejki. Continuations z working tree koordynacji są
odroczonymi, zatwierdzanymi requestami, nie natywnym steeringiem ACP.

## Zakres v1 i miejsca integracji

- Jawne queue/stop oraz stop-and-send z potwierdzoną granicą zakończenia.
- Steering tylko dla adapterów z faktycznie potwierdzonym wsparciem.
- `apps/desktop/src/{harness.rs,harness/readiness.rs,views/chat.rs,coordination.rs}`.
- `apps/desktop/src/{workspace_panes.rs,task_budgets.rs,remote.rs}`.
- `crates/agent/src/{lib.rs,bin/mock_acp_agent.rs}` oraz relay ACP.

## Etap A — jedna maszyna stanów wysyłki

1. [ ] Spisać obecne przejścia idle/running/permission-wait/cancelling/disconnected
   i foreground/background. Ustalić własność promptu: session + runtime generation,
   nie aktywny projekt lub indeks karty w momencie nadejścia eventu.
2. [ ] Zdefiniować intencję delivery: queue, steer, stop-and-send. Stan promptu
   odróżnia draft/pending/reserved/submitted/finished/cancelled/delivery-unknown.
   Rezerwacja nie dowodzi przyjęcia przez adapter; brak odpowiedzi nie pozwala replay.
3. [ ] Współdzielić dispatch gates dla Enter, przycisku, Retry, continuation i API:
   trust/grant, model/mode readiness, context, permissions i durable budget.
4. [ ] Po stop-and-send poczekać na terminalny event starej tury lub jawne
   reconciliation. Timeout anulowania nie może uruchomić drugiej niepewnej tury.
5. [ ] Stare tool/permission eventy z anulowanego runtime nie mogą otrzymać
   zgody nowej tury. Nie zmieniać statusu zakończonego procesu na podstawie ikony Stop.

## Etap B — composer i przewidywalna kolejka

1. [ ] Podczas działania agenta pokazać jawny przycisk dodania tekstu do kolejki
   oraz osobny Stop. Aktualnie ikona wysyłki przechodzi w Stop: usunąć niejasność
   między wysłaniem wpisanego tekstu a anulowaniem odpowiedzi.
2. [ ] Dropdown udostępnia dostępne tryby z opisem skutków. Domyślnie zachować
   dotychczasowe Enter/Shift-Enter i queue, chyba że użytkownik jawnie wybierze inaczej.
3. [ ] Queue pokazuje odbiorcę, treść/kontekst i stan. Reorder/edit/remove
   działają tylko przed rezerwacją; zmiana już zarezerwowanego promptu wymaga nowego ID.
4. [ ] Stop nie kasuje draftu ani kolejki. Zaproponować osobne „usuń oczekujące”,
   bez ukrytego wznowienia po cancellation. Zachować obecne foreground guards.
5. [ ] Ustalić dostępność skrótów przez plan 16; goły Escape zamyka menu/dialog,
   nie anuluje pracy tylko dlatego, że użytkownik zamknął picker.

## Etap C — natywny steering i transport

1. [ ] Sprawdzić faktyczne capabilities i wersje każdego adaptera. Zbudować
   macierz supported/unsupported/unknown; nie wnioskować obsługi z nazwy modelu.
2. [ ] Dla supported przesłać steering właściwym mechanizmem, z potwierdzeniem
   przyjęcia i identyfikatorem tury. UI nie obiecuje natychmiastowego przerwania toola.
3. [ ] Jeśli jedyną opcją jest cancel + nowy prompt, nazwać ją stop-and-send.
   Nie reklamować fallbacku jako zachowania na bezpiecznej granicy.
4. [ ] Rejestrować rzeczywisty koszt/attempt zgodnie z planem 6. Jeśli granica
   billingu steeringu jest nieobserwowalna, pokazać unknown, nie dopisywać kosztu zero.
5. [ ] Relay obsługuje te same stany bez local fallback; disconnect i niepewna
   dostawa wymagają reconciliation. Persistent queue (15) po restarcie jest wstrzymana.

## Testy i kryteria odbioru

- [ ] Mock ACP: długi tool, permission-wait, delayed cancel, brak cancel ack,
  end-turn równoczesny z kliknięciem, duplicate event i nieobsługiwany steering.
- [ ] Jeden świadomy dispatch rezerwuje jeden client attempt; podwójny klik lub
  reconnect nie wysyła drugiego. Nie deklarować provider-level exactly-once.
- [ ] Edycja kolejki i zmiana aktywnej karty nie zmieniają odbiorcy ani kontekstu.
- [ ] Budget stop blokuje dalsze wysłanie, ale zachowuje treść pending/draft.
- [ ] Stop nie jest Undo: UI ujawnia pozostawione pliki/komendy i nie udaje,
  że proces zewnętrzny lub deployment został wycofany.
- [ ] Test restartu w każdym stanie i zgodności z odroczonymi continuations.
- [ ] Natywne testy klawiatury, IME, kolejki i VoiceOver oraz osobny odbiór relay.

## Zależności i granice

Kontekst (9), trwałość (15), akcje (16) i poprawność billingu (6).
Bez automatycznego cofania plików, przełączania uprawnień i restartowania adaptera.

Źródło: [VS Code chat](https://code.visualstudio.com/docs/chat/chat-overview).
