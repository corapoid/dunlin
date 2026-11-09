# 1. Git v2 — odbiór ręczny i rzeczywisty relay

## Cel i stan obecny

Zamknąć odbiór istniejącego Git v2 bez rozszerzania funkcjonalności.
Przygotowanie i potwierdzanie mutacji, blokady, ochrona buforów, graf, inline
blame i transport relay mają implementacje oraz testy. Nadal brakuje ręcznego
odbioru natywnych dialogów, hoverów i rzeczywistego wdrożenia serwera.

## Zakres i granice

- Odbiór istniejących stash/worktree/merge/rebase/cherry-pick i ich zabezpieczeń.
- Odbiór grafu, szczegółów, menu, tooltipów i inline blame.
- Naprawy błędów odtworzonych podczas tego odbioru.
- Bez kolejnych mutacji, rozszerzeń blame, PR/Actions i automatycznego rollbacku.
- Bez operacji na roboczym repozytorium użytkownika lub jego produkcyjnym relay.

## Miejsca w kodzie i dokumentacji

- `apps/desktop/src/git_history/{mutations.rs,graph.rs,details.rs,smoke.rs}`.
- `apps/desktop/src/inline_blame.rs`.
- `crates/workspace/src/git/{operations.rs,blame.rs}`.
- `crates/relay/src/api/operations.rs` i desktopowy transport `remote/operations`.
- `docs/git-operations-and-blame.md`, `docs/git-history.md`.
- `scripts/git-operations-smoke.py`.

## Etapy

1. [ ] Przygotować izolowane repozytoria i osobny katalog danych aplikacji.
   Zapisać wersję aplikacji, Git i systemu; odseparować konfigurację Git.
   Fixture musi zawierać merge, rename, binaria, Unicode, częściowy staging,
   untracked/ignored oraz konfliktowe historie.
2. [ ] Uruchomić właściwe testy workspace/desktop/relay i istniejący Git smoke
   w dark/wide oraz light/narrow. Zapisane stare wyniki nie zamykają tego kroku.
3. [ ] Ręcznie odebrać dialogi: preview, właściwy target/OID, Cancel/Enter/Esc,
   Tab/Shift-Tab, przywrócenie fokusu, dostępność i ostrzeżenia o hookach.
   Zmiana formularza nie może zmieniać zatwierdzanego planu.
4. [ ] Odebrać operacje i konflikty: stash push/apply/pop/drop, worktree
   add/remove/prune, merge/rebase/cherry-pick, continue/abort. Sprawdzić brudny
   index, kolizje ignored, brak mainline, blokady i nieaktualny plan.
5. [ ] Odebrać zachowanie buforów: niezapisany tekst, otwarte aliasy plików,
   watcher, zamknięcie widoku podczas operacji, nieudany Reload oraz jawny
   Overwrite. Żadnego automatycznego save/discard i utraty Undo edytora.
6. [ ] Odebrać graf i blame: hover ścieżki, merge i rodzice, stabilny cel menu
   po refreshu, zawartość tooltipu, link do commitu, długie linie, wąski panel,
   zmienione linie i Undo, zmiana HEAD oraz Reduce Motion.
7. [ ] Po osobnym uzgodnieniu wdrożyć aktualny relay na izolowanym hoście.
   Powtórzyć istotne przypadki przez prawdziwy transport; porównać obserwowany
   stan repozytorium na hoście z odpowiedziami aplikacji.
8. [ ] Sprawdzić cofnięcie grantu/device, zmianę scope, stary relay, offline,
   zerwanie połączenia po starcie mutacji i spóźnione odpowiedzi. Nie ponawiać
   automatycznie mutacji o nieznanym wyniku; wymagać inspekcji stanu.
9. [ ] Każdy wykryty błąd najpierw odtworzyć, następnie naprawić i dodać
   adekwatny test regresji. Powtórzyć dotkniętą część odbioru.

## Kryteria odbioru

- [ ] Wszystkie wykonane przypadki mają status PASS/FAIL/BLOCKED i opis środowiska.
- [ ] Anulowanie nie mutuje Git; przeterminowany plan nie wykonuje operacji.
- [ ] Zmiana widoku/projektu nie zwalnia trwającej blokady i nie miesza wyników.
- [ ] Bufory, index i Undo nie są nadpisywane bez jawnej operacji użytkownika.
- [ ] Relay nie interpretuje ścieżek lokalnie, nie obchodzi grantów i nie ujawnia tokenów.
- [ ] Wynik loopback, ręczny GUI i rzeczywisty host są opisane oddzielnie.

## Rezultat

Uzupełnić istniejące dokumenty odbioru o aktualne dowody i pozostałe blokery.
Usunąć z roadmapy wyłącznie rzeczy faktycznie odebrane; brak dostępnego hosta
oznacza BLOCKED dla remote, nie zaliczenie całego Git v2.
