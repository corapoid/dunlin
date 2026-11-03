# Terminal i Run — implementacja i granice

Stan: **wdrożone lokalne PTY/VT, taby i scrollback oraz statyczne wykrywanie
komend z manifestów.** Ręczny odbiór GUI/IME i transport zdalny pozostają
oddzielne.

## Terminal

- `crates/terminal` uruchamia lokalny PTY przez `portable-pty` i utrzymuje
  `vt100::Parser` po stronie UI. Reader i waiter działają na osobnych wątkach;
  kanał wyjścia jest ograniczony, więc szybki proces nie może rozsadzić pamięci.
- Domyślny shell pochodzi z `$SHELL` (fallback `/bin/zsh`), startuje jako login
  shell w katalogu projektu. `TERM=xterm-256color`, `COLORTERM=truecolor`.
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
  `MasterPty::resize`), z minimalnymi wymiarami 10×40.
- `⌘F` albo **Find** otwiera wyszukiwanie widocznego buforu; pasujące fragmenty
  są podświetlane, a licznik pokazuje trafienia w widocznym viewportcie. Linki
  `http://`, `https://` i `mailto:` w tekście są klikalne (bez interpretera OSC 8).
- Obsługiwane wejście: Unicode z `key_char`, Enter/Tab/Backspace/Escape,
  strzałki/Home/End/Delete/Insert, Ctrl+C/D/Z/L/A/E/U/K/W, Cmd+C (kopiuj bufor),
  Cmd+V (bracketed paste, limit 1 MiB), Cmd+W (zamknij tab).
- Rendering obsługuje ANSI 16/256/RGB, bold, inverse i kursor blokowy. Nie ma
  jeszcze zaznaczania myszą, splitów, shell integration ani pełnego OSC 8;
  wyszukiwanie obejmuje widoczny viewport, nie cały scrollback naraz.

### Bezpieczeństwo i zdalność

Terminal jest lokalny i uruchamia kod projektu z uprawnieniami użytkownika.
Nie jest sandboxem. Projekt musi być zaufany. Dla projektu relay UI pokazuje
jawny komunikat i **nie** uruchamia lokalnego shella jako fallbacku; zdalny PTY
wymaga osobnego grantu relay/SSH, którego ta zmiana nie udaje.

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
- Historia jest lokalna, ograniczona do 50 rekordów, z jawnym statusem
  `running/succeeded/terminated/...`. Zamknięcie taba oznacza run jako przerwany
  przez użytkownika. Po restarcie historyczne `running` staje się
  `interrupted by app restart`; nic nie jest automatycznie wznawiane.
- Brak automatycznego retry, auto-run, wykonywania tekstu z logów lub wykrywania
  komend przez LLM.

## Walidacja

- Testy PTY spawnują prawdziwy `/bin/sh`, sprawdzają output i exit code.
- Testy VT sprawdzają ANSI, scrollback i bounded defaults.
- Testy Run pokrywają Node/Cargo/Make/Just/Python/scripts, wybór package
  managera, workspace Cargo, błędne manifesty i pomijanie `node_modules`.
- Pełny zestaw: **527 testów OK / 0 pominiętych**, `cargo fmt --check`, Clippy
  `-D warnings`, `git diff --check`.
- Ręczna QA GUI, IME/bidi, szybki output, SIGINT, zdalny PTY/SSH i rzeczywiste
  projekty pozostają do wykonania.
