# Subreaper bootstrap: zweryfikowana hipoteza kernela

2026-11-12. **PASS dotyczy małego probe Python na rzeczywistym jądrze Linux,
nie integracji usługi Rust ani pełnej odporności na awarie.** E4.4 pozostaje otwarte.
Nie uruchamiano hostowego buildu Rust, modelu, tailnetu ani usług zewnętrznych.

## Dowód i jego zakres

Uruchomiono dwa probe w dedykowanym `dunlin-checks`: 128 MiB RAM/swap, 1 CPU,
64 procesy, 16 MiB tmpfs, 35 s limit, UID 65534, read-only root/toolchain,
no-network, pusty env (`HOME`, `PATH`, `TMPDIR`), przypięte istniejące
AppArmor/seccomp. Brak mountów źródeł, host HOME i credentiali.
Bubblewrap: `ff5add553bea12a45c18ebe6c67d3ed637052b869a1ca2fc5d74a6a2e8c5cce6`.
Oba uruchomienia zakończyły się kodem 0.

Preferowany drugi dowód:

- [probe.py](../2026-11-12-worker-bootstrap/subreaper-accepted-hypothesis/probe.py)
- [result.json](../2026-11-12-worker-bootstrap/subreaper-accepted-hypothesis/result.json)

Pierwszy, eksploracyjny, zachowano bez zmian w
[pierwszym probe](../2026-11-12-worker-bootstrap/subreaper-exploratory/result.json).
Wyniki zawierają hash źródła, image ID, limity oraz bounded stdout/stderr.
Koordynator zachowuje kopie dowodów; probe nie zmieniał runtime produkcyjnego.

Trzy rzeczywiste SIGKILL zabiły guardian: przed przekazaniem init pidfd do Main,
po przekazaniu, lecz przy zablokowanej bramce, oraz po uruchomieniu lokalnego
payloadu z cichym potomkiem trzymającym stdout i zapisywalny FD. Main jako subreaper
przejął outer bwrap i namespace-init. Każdy cel uzyskał pidfd, potwierdzenie
własnego dziecka po uchwycie, PPid/UID, SIGKILL przez FD, obserwację zakończenia
i reap. Przed ACK payload nie ruszył; po starcie stdout osiągnął EOF, a zapis
zatrzymał się. Drugi probe wymagał końcowego `P_ALL/WNOWAIT -> ECHILD` oraz
sprawdził, że uchwyt procesu rodzica nie przechodzi kontroli własnego dziecka.

## Kontrakt do integracji Rust

1. Subreaper wyłącznie w dedykowanym worker CLI, przed uruchomieniem runtime
   i przed forkami. Nie w wielofunkcyjnym relay API. Na fatal cleanup bramka
   nowych spawnów musi być zamknięta; nie może istnieć niezależny spawner.
   [PR_SET_CHILD_SUBREAPER](https://man7.org/linux/man-pages/man2/PR_SET_CHILD_SUBREAPER.2const.html)
   określa adopcję osieroconych potomków przez najbliższego żywego subreapera;
   flaga wymaga osobnego ustawienia w guardianie, jeżeli ten również jej używa.
2. Nigdy nie sygnalizować PID odczytanego z JSON lub `/proc`. Dla kandydata
   najpierw `pidfd_open`, następnie `waitid(P_PIDFD, ..., WEXITED|WNOHANG|WNOWAIT)`.
   `ECHILD` oznacza brak uprawnienia do sygnalizowania tego uchwytu. Potwierdzić
   PPid i UID; nie reapywać kandydata przed otwarciem uchwytu. Gwarancja obejmuje
   żywe/zombie własne dzieci bez `SIGCHLD=SIG_IGN`, `SA_NOCLDWAIT` i konkurencyjnego
   reapera. [pidfd_open](https://man7.org/linux/man-pages/man2/pidfd_open.2.html),
   [waitid](https://man7.org/linux/man-pages/man2/waitpid.2.html).
3. `/proc/self/task/*/children` jest tylko źródłem kandydatów. **Pusta lista nie
   dowodzi zatrzymania**: man-pages opisuje pomijanie żywych dzieci przy wyjściu
   innych. Końcowe `waitid(P_ALL,0,WEXITED|WNOHANG|WNOWAIT)` musi zwrócić `ECHILD`.
   Brak gotowego statusu oznacza nadal żywe dzieci; zombie zachować do zdobycia
   pidfd. Ponawiać bounded cleanup/adopcję, a po deadline raportować Unknown.
   [proc_tid_children](https://man7.org/linux/man-pages/man5/proc_tid_children.5.html).
4. Zatrzymywać przez zweryfikowany pidfd i czekać na jego readiness, potem
   reapować własne dziecko. Śmierć PID-namespace init powoduje kernel SIGKILL
   pozostałych procesów tej przestrzeni. Timeout, błąd własności lub brak
   potwierdzenia nie są dowodem FullStop.
   [pidfd_send_signal](https://man7.org/linux/man-pages/man2/pidfd_send_signal.2.html),
   [PID namespaces](https://man7.org/linux/man-pages/man7/pid_namespaces.7.html).
5. Guardian musi przeżyć SIGKILL Main, aby sam zakończyć bootstrap. PDEATHSIG
   outer bwrap może wiązać go z guardianem, ale nie zastępuje niezależnego ownera.
   Oczekiwany parent musi być uchwycony **przed** fork, następnie sprawdzony po
   ustawieniu prctl; parametr dotyczy wątku rodzica i bywa zerowany przez zmiany
   credentials. [PR_SET_PDEATHSIG](https://man7.org/linux/man-pages/man2/PR_SET_PDEATHSIG.2const.html).

Pinned Tokio 1.54.1 sprawdzono źródłowo: `process/unix/orphan.rs:118` wywołuje
`try_wait` tylko dla `StdChild` umieszczonych w kolejce; `reap.rs:89` i
`pidfd_reaper.rs:108` również dotyczą konkretnego `inner`. Nie znaleziono
wait-any reapera adoptowanych, nieznanych initów w tym module. To wspiera
założenie, ale wymaga ponownej kontroli po zmianie zależności. Kod usługi nie może
dodawać innego reapera ani porzucać własności znanego helpera w czasie cleanup.

## Nadal nieudowodnione

- Integracja rzeczywistego Rust Main/guardian/Tokio, wszystkie okna startupu,
  lease/ACK i rzeczywisty transport worker v3.
- Jednoczesny SIGKILL Main i guardian: brak żywego ownera. To osobna bramka
  deployment cgroup/external owner; ten probe jej nie zamyka.
- Arbitrary PID reuse stress, migracja flag sygnałów, multi-execution concurrency
  oraz proces trwale utkwiony w kernel I/O. Bounded timeout pozostaje Unknown.

Nie traktować tego dowodu jako zgody na release płatnego programu przed pełnym
handoff + aktualnym ACK Main ani jako zamknięcia E4.4/E10.
