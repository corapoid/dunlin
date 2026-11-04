# Terminal i Run — implementacja i granice

Stan: **lokalny i relayowy PTY, SSH, taby i splity, zaznaczanie myszą,
scrollback, shell integration (OSC 133/7), OSC 8 oraz Run z manifestów.**

Ręczny odbiór GUI/IME/bidi, rzeczywisty host relay/SSH i nietypowe prompty
pozostają osobną macierzą QA.

## Terminal

- `crates/terminal` uruchamia lokalny PTY przez `portable-pty` i utrzymuje
  `vt100::Parser` po stronie UI. Reader i waiter działają na osobnych wątkach;
  kanał wyjścia jest ograniczony, więc szybki proces nie może rozsadzić pamięci.
- Domyślny shell pochodzi z `$SHELL`; gdy jest niedostępny, aplikacja próbuje
  `/bin/zsh`, `/bin/bash`, a potem `/bin/sh`. Startuje jako login shell w
  katalogu projektu. `TERM=xterm-256color`, `COLORTERM=truecolor`.
- Aplikacja usuwa z procesu terminala wewnętrzne zmienne `DUNLIN_*` i bearer
  token relayu. Pozostałe środowisko użytkownika jest dziedziczone; projektowe
  env jest przekazywane wyłącznie do jawnie uruchomionego Run.
- Terminal i Run są dodatkowymi panelami w jednym docku, a nie widokami
  głównymi. `⌘J` pokazuje/ukrywa Terminal, `⌘⇧J` Run; panel zamyka się też
  przez `×`, a stan jest zapamiętywany. Dock można przypiąć do dołu, lewej lub
  prawej krawędzi przyciskami w nagłówku albo przeciągnąć uchwyt `⠿` w stronę
  wybranej krawędzi. Wewnętrzna krawędź panelu jest uchwytem resize:
  przeciąganie zmienia wysokość (dół) lub szerokość (lewo/prawo), z limitami
  względem okna. Uchwyt ma 9 px szerokości i jest osobnym pasem layoutu, więc
  nie konkuruje ze scrollem treści; podwójny klik przywraca domyślny rozmiar.
  Terminal i Run przełączają się w tym samym docku bez zabijania sesji.
- Taby są niezależnymi sesjami. Zamknięcie taba wysyła kill do dziecka i usuwa
  PTY; zamknięcie aplikacji robi to samo przez `Drop`. Brak automatycznego
  wznawiania/replayu po restarcie.
- Scrollback jest ograniczony do 5 000 linii domyślnie (twardy sufit 50 000).
  Kółko myszy oraz PageUp/PageDown przewijają widok; wpisanie znaku wraca do
  live viewportu.
- Rozmiar PTY jest dopasowywany do okna przy renderze (SIGWINCH przez
  `MasterPty::resize`), z minimalnymi wymiarami 3×20.
- `⌘F` albo **Find** otwiera wyszukiwanie widocznego buforu; pasujące fragmenty
  są podświetlane, a licznik pokazuje trafienia w widocznym viewportcie.
- Obsługiwane wejście: Unicode z `key_char`, Enter/Tab/Backspace/Escape,
  strzałki/Home/End/Delete/Insert, Ctrl+C/D/Z/L/A/E/U/K/W, Cmd+A (zaznacz cały
  zachowany scrollback), Cmd+C (kopiuj zaznaczenie lub bufor), Cmd+V (bracketed
  paste, limit 1 MiB), Cmd+W (zamknij aktywny tab), Cmd+D (split w dół),
  Cmd+Shift+D (split w prawo).
- Rendering obsługuje ANSI 16/256/RGB, bold, inverse i kursor blokowy.

### Zaznaczanie myszą i linki

- Przeciąganie lewym przyciskiem tworzy zaznaczenie w absolutnych liniach
  bufora, więc nie znika przy przewijaniu i zmianie rozmiaru okna. Dwuklik
  zaznacza słowo/ścieżkę, trójklik całą linię, Shift-klik rozszerza istniejące
  zaznaczenie. Przeciąganie poza górną/dolną krawędź przewija o jedną linię.
- Cmd+A zaznacza cały zachowany scrollback, Cmd+C i **Copy** kopiują
  zaznaczenie; bez zaznaczenia kopiują cały widoczny bufor. Puste komórki są
  zachowywane jako spacje, końcowe spacje linii są obcinane, a limit schowka to
  1 MiB z zachowaniem granicy UTF-8.
- Zaznaczenie jest lokalne: aplikacje terminalowe nie dostają zdarzeń myszy
  (brak mouse reporting), więc nie ma konfliktu z zaznaczaniem tekstu.
- Linki `http://`, `https://` i `mailto:` w tekście są klikalne. OSC 8 jest
  parsowany przyrostowo także wtedy, gdy sekwencja jest podzielona między
  chunki; link jest przypisany do komórek i renderowany na niebiesko. Do
  otwarcia kwalifikują się wyłącznie jawnie dozwolone schematy; `javascript:`,
  `file:` i inne URI są ignorowane.

### Splity

- Przyciski `◫`/`⬓` albo `⌘⇧D`/`⌘D` dzielą aktywny pane w prawo/w dół i
  uruchamiają w nowym pane niezależny shell (lokalny, relay albo SSH — zgodnie
  z projektem). Split nie duplikuje sesji: stary tab zostaje w lewym/górnym
  pane, nowy w prawym/dolnym.
- Podział jest drzewem o proporcji 0.5, ograniczonej do 0.15–0.85. Uchwyt
  podziału ma 6 px i można go przeciągać; rozmiar każdego PTY jest liczony
  osobno i wysyłany jako SIGWINCH/`resize`.
- Aktywny pane ma mocniejszą ramkę. Kliknięcie pane ustawia aktywny tab, a
  pasek tabów pozwala przełączać sesje; tab już widoczny jest tylko
  aktywowany, ukryty zastępuje aktywny pane. Zamknięcie taba zwija dokładnie
  jeden liść split tree. Zamknięcie ostatniego taba usuwa drzewo.
- Wysokość wiersza 15 px, szerokość komórki 7.1 px i padding 8 px są wspólne
  dla renderu i hit-testingu. Canvas rejestruje bounds pane, dzięki czemu
  zaznaczanie działa także po zmianie splitu i docku.

### Shell integration

- Włączona domyślnie (Settings → Editor → Terminal → **Shell integration**).
  Zmiana dotyczy nowych tabów; istniejące sesje zachowują swoje hooki.
  Aplikacja tworzy prywatny katalog `0700` w katalogu tymczasowym i uruchamia
  zsh przez `ZDOTDIR` albo bash przez `--rcfile`; nie modyfikuje dotfiles
  użytkownika. Wrapper najpierw source'uje oryginalne `.zshenv`/`.zprofile`/
  `.zshrc` (zsh) lub profile/`.bashrc` (bash), a potem dodaje hooki.
- Obsługiwane markery: OSC 133 `A` (prompt), `B` (początek linii komendy),
  `C` (start wykonania), `D;<exit>` (koniec), OSC 7 `file://host/path` oraz
  OSC 1337 `CurrentDir=`/`RemoteHost=`. Akceptowany jest też wariant OSC 633.
  Percent-encoding ścieżki jest dekodowany.
- UI pokazuje bieżący katalog z OSC 7, zielony znacznik `OSC 133`, tint linii
  promptu oraz akcje **Cmd** (kopiuj ostatnią komendę) i **Out** (kopiuj output
  ostatniego zakończonego polecenia). Nic nie jest automatycznie uruchamiane
  ani wysyłane do shella; akcje tylko kopiują tekst.
- Bloki są ograniczone do 512 pozycji; output jest wycinany z bieżącego
  bufora/scrollbacku, nie jest przechowywany jako ukryta kopia. Nieznany shell
  (np. fish) działa bez integracji i bez błędu; użytkownik może ją wyłączyć,
  jeśli własny prompt używa `ZDOTDIR`/`PROMPT_COMMAND` w nietypowy sposób.
- Integracja nie jest instalowana w zdalnym shellu SSH; jeśli host sam emituje
  OSC 133/7, UI je pokaże.

### Transport relay

- Projekt relay używa `GET /v1/terminal?project=...&rows=...&cols=...` z
  bearer tokenem. Relay sprawdza urządzenie i grant projektu, po czym otwiera
  PTY w katalogu projektu na hoście relay. Klient nie uruchamia lokalnego
  shella jako fallbacku.
- Protokół WebSocket: binarne frame'y to surowe bajty PTY w obu kierunkach;
  tekstowe frame'y to JSON `ready`, `exit`, `error`, `resize`, `close`, `ping`.
  Klient wysyła `resize` po zmianie pane i `close` przy zamykaniu taba. Relay
  wysyła `exit` z kodem/sygnałem i zamyka socket. Heartbeat ma 30 s.
- Limity: 8 równoczesnych sesji terminalowych na relay, wejściowy frame do
  2 MiB, `rows` 4–400, `cols` 20–500, kanały wyjścia/wejścia są ograniczone.
  Połączenie wymaga HTTPS/WSS poza loopbackiem; token nie jest przekazywany
  do powłoki.
- **Relayowy PTY nie jest sandboxem.** Użytkownik z grantem projektu otrzymuje
  pełny shell na hoście relay z uprawnieniami procesu relay. Grant projektu
  jest granicą capability; nie należy go przyznawać osobom, które nie mogą
  wykonywać kodu na tym hoście.

### Transport SSH v1

- Przycisk **SSH** w toolbarze terminala otwiera pasek `user@host` (alias
  z `~/.ssh/config`) i opcjonalną ścieżkę zdalną. `Connect` uruchamia lokalny
  `/usr/bin/ssh -tt` w PTY; nie ma własnego klienta kryptografii ani kopiowania
  kluczy.
- Używane są `ControlMaster=auto`, prywatny `ControlPath` w katalogu `0700`,
  `ControlPersist=60` i `ConnectTimeout=15`. Uwierzytelnianie, agent, klucze i
  `known_hosts` są obsługiwane przez OpenSSH; nowy host key wymaga jawnej
  akceptacji w terminalu. Target jest walidowany (brak opcji/spacji), a ścieżka
  zdalna jest pojedynczo cytowana przed `cd`.
- Do hosta nie są wysyłane projektowe env ani tokeny Dunlin. SSH nie
  bootstrapuje jeszcze relayu, nie zarządza Keychain i nie buduje tuneli —
  to pozostaje zakresem etapu SSH XL.

### Bezpieczeństwo i zdalność

Terminal lokalny uruchamia kod projektu z uprawnieniami użytkownika. Nie jest
sandboxem. Projekt musi być zaufany. Dla projektu relay UI używa wyłącznie
grantu relay; SSH wymaga jawnego targetu i decyzji użytkownika. Zamknięcie okna
lub taba kończy proces; nie ma ukrytego replayu.

## Run

- `dunlin_workspace::run::detect` czyta statycznie, bez ewaluacji manifestów:
  `package.json` (scripts + npm/pnpm/yarn/bun), `Cargo.toml` (run/test/check/
  build/clippy, `[[bin]]`, `[[example]]`, workspace), `Makefile`, `justfile`,
  `pyproject.toml` (pytest/ruff/mypy), `go.mod`, `deno.json`, `composer.json`,
  `Rakefile`, `mix.exs`, `gradlew`, `mvnw` oraz `scripts/` i `bin/` dla
  `.sh/.bash/.zsh/.py/.js/.mjs/.cjs/.rb/.pl`.
- Skan jest ograniczony do 40 katalogów (depth 2), 80 kandydatów, 1 MiB na
  manifest; pomija `.git`, `node_modules`, `target`, build/dist, venv i katalogi
  ukryte. Symlinki nie są śledzone.
- Kandydat ma stabilne ID, dokładny `argv`, program, cwd względny i źródło.
  Uruchomienie nie przechodzi przez shell string; projektowe env jest pokazane
  jako **nazwy**, bez wartości, i przekazane do PTY.
- Każde uruchomienie wymaga natywnego potwierdzenia z preview argv/cwd/env.
  Komenda startuje w nowym tabie terminala; exit code/sygnał trafia do historii.
- Run jest lokalny. Projekt relay może użyć terminala relay, ale detekcja i
  uruchamianie Run na hoście relay pozostają osobnym zakresem.
- Historia jest lokalna, ograniczona do 50 rekordów, z jawnym statusem
  `running/succeeded/terminated/...`. Zamknięcie taba oznacza run jako przerwany
  przez użytkownika. Po restarcie historyczne `running` staje się
  `interrupted by app restart`; nic nie jest automatycznie wznawiane.
- Brak automatycznego retry, auto-run, wykonywania tekstu z logów lub wykrywania
  komend przez LLM.

## Walidacja

- Testy PTY spawnują prawdziwy `/bin/sh`, sprawdzają output i exit code.
- Testy VT sprawdzają ANSI, scrollback, OSC 8/133/7, zaznaczanie, wycinanie
  tekstu, split ratio i bezpieczne linki.
- Testy shell integration uruchamiają prawdziwe `/bin/zsh` i `/bin/bash`,
  gdy są dostępne, i czekają na zakończony blok OSC 133 z komendą i exit code.
- Testy relay sprawdzają 401/403 na WebSocket, `ready` i strumień prawdziwego
  PTY zakończony `exit`.
- Zestaw terminala/relaya/desktopu: **380 testów OK / 0 pominiętych**
  (`cargo test -p dunlin-terminal -p dunlin-relay -p dunlin`). `cargo fmt
  --all --check`, Clippy `-D warnings` dla zmienionych pakietów i
  `git diff --check` przechodzą.
- Ręczna QA GUI, IME/bidi, mouse drag w splitach, szybki output, SIGINT,
  prawdziwy host relay/SSH i nietypowe prompty pozostają do wykonania.
