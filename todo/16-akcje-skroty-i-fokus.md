# 16. Wspólny rejestr akcji, skróty i fokus

## Cel i stan obecny

Każda akcja ma jedną definicję, jednakowe warunki dostępności i przewidywalną
obsługę klawiaturą. Inspiracje: VS Code keybindings i JetBrains Find Action.
Dunlin ma natywne menu, paletę i hinty, ale definicje oraz routing są rozłożone
między `native_menu.rs`, `app.rs` i powierzchnie. Shift+Tab w composerze przełącza
tryb, kolidując ze zwykłym cofaniem fokusu; najpierw potwierdzić aktualny kod.

## Zakres v1 i miejsca integracji

- Rejestr najczęstszych akcji, dostępność, etykiety i kontekstowy routing skrótów.
- Fokus i pełna nawigacja bez myszy; później edytor keymap i diagnostyka konfliktów.
- `apps/desktop/src/{native_menu.rs,app.rs,views/palette.rs,workspace_panes.rs}`.
- `apps/desktop/src/{views/chat.rs,terminal/mod.rs,app/overlays.rs}` i settings.
- `crates/ui/src/{controls.rs,settings.rs}` oraz native UI smoke/AX.

## Etap A — minimalny rejestr, bez wielkiego przepisywania aplikacji

1. [ ] Zinwentaryzować menu, Command, key handlers, toolbar i context menus.
   Spisać istniejące skróty i wyjątki; nie usuwać funkcji jako skutku refaktoru.
2. [ ] Zdefiniować ActionId, label/aliases, target, effects, availability(reason),
   default bindings i scope. Oddzielić opis akcji od serwisu, który ją wykonuje.
3. [ ] Migrować pionowo: nawigacja/paleta, composer, review/checks. Menu,
   przycisk i skrót korzystają z tej samej definicji i tego samego executor guard.
4. [ ] Kontekst zawiera focused pane/control, session/project/host i revision
   celu. Nie opierać działania na globalnym „ostatnio aktywnym” projekcie.
5. [ ] Dostępność wyjaśnia brak trust, adaptera, celu lub busy operation.
   Serwis ponawia kontrolę tuż przed skutkiem; ActionId nie jest capability grant.

## Etap B — kontrakt skrótów i dostępność

| Skrót | Docelowy kontrakt po audycie konfliktów |
| --- | --- |
| `⌘P` | Quick Open plików i istniejących kart z planu 8 |
| `⌘⇧P`, `⌘K` | Zachować obecne wejścia do palety/launchera |
| `⌘L` | Fokus composera istniejącej rozmowy, bez czyszczenia draftu |
| `⌥Enter` | Akcje dla bieżącego zaznaczenia/błędu/diffu |
| `⌃Tab` | MRU kart, z jawnym wyborem i anulowaniem |
| `⌘.` | Stop wskazanej aktywnej operacji, bez rollbacku |
| `Tab`, `Shift+Tab` | Nawigacja fokusem; nie zmiana trybu agenta |

1. [ ] Zachować obecne `⌘1/2/3` (Chat/Graph/Trace), `⌘J`, `⌘⇧J`, `⌘⌥P`
   i skróty edytora. Zmiany mapy uzasadnić, pokazać w migracji i dać reset.
2. [ ] Tryb agenta dostałby osobny konfigurowalny skrót po sprawdzeniu konfliktów.
   Zmiana trybu wymaga acknowledged adapter state, nie tylko zmiany etykiety.
3. [ ] Rozwiązać priorytet modal → focused control → pane → app. Terminal,
   IME i pola tekstowe zachowują własne kombinacje, copy/paste i native editing.
4. [ ] Stop rozróżnia agent/check/run. Jeśli cel nie jest jednoznaczny, wyświetlić
   wybór zamiast anulować wszystkie procesy. Escape najpierw zamyka overlay.
5. [ ] Każdy dialog ma początkowy fokus, trap, logiczne Tab/Shift-Tab i restore
   do istniejącego elementu. Enter/Space aktywuje focused button, nie dwa handlery.

## Etap C — edytor keymap i odkrywalność

1. [ ] Dodać listę akcji także bez skrótu: wyszukiwanie nazwą i naciśniętą
   kombinacją, przypisanie, usunięcie i reset. Zapis ustawień bez restartu.
2. [ ] Pokazywać konflikty wraz z warunkami scope; ten sam skrót w rozłącznych
   kontekstach jest dozwolony. Nie nadpisywać kolizji bez informacji dla użytkownika.
3. [ ] Dodać „Przypisz skrót” z palety/menu. Tooltip, menu i lista skrótów pokazują
   tę samą rzeczywistą mapę, nie hard-coded hinty sprzed zmiany ustawień.
4. [ ] Obsłużyć układy klawiatury, Unicode, dead keys i oznaczenia macOS.
   Ustalić rozróżnienie logical key vs physical key; nie przejmować systemowych skrótów.
5. [ ] Diagnostyka pokazuje wykryty skrót, scope i wybraną akcję, bez logowania
   wpisywanego tekstu, promptów lub sekretów. Brak globalnych hotkeys w v1.

## Testy i kryteria odbioru

- [ ] Ten sam target i guard dla menu/przycisku/palety/skrótu, również po zmianie
  panelu lub otwarciu drugiej sesji. Brak podwójnego execution przez propagację.
- [ ] Shift+Tab cofa fokus; nie podnosi uprawnień ani nie zmienia trybu agenta.
- [ ] Keyboard-only flow: otwórz plik → kontekst → prompt → decyzja → review.
- [ ] Dwa aktywne procesy nie prowadzą do przypadkowego „stop all”.
- [ ] Native tests: terminal, modal, editor, composer, IME, menu i aktywny VoiceOver.
- [ ] Keymap migration, reset, konflikt, uszkodzone ustawienia i zmiana layoutu.
- [ ] Dark/light, Compact/Comfortable, minimalne okno, zoom i Reduce Motion.
- [ ] Wczesny odbiór rejestru/fokusu oddzielny od późniejszego edytora skrótów.

## Zależności i granice

Fundament planów 8–19; nie czekać z małymi poprawkami planu 4 na cały refaktor.
Bez pełnego keymap emulation JetBrains/Vim i bez zmiany modelu uprawnień.

Źródło: [VS Code keybindings](https://code.visualstudio.com/docs/configure/keybindings).
