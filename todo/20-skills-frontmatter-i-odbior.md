# 20. Skills — kompatybilne frontmatter i odbiór UI

## Cel i stan obecny

Audyt working tree: 2026-11-09. Importować istniejące `SKILL.md` bez uszkadzania
opisu ani cichego usuwania metadanych przy instalacji. To rozszerzenie istniejącej
biblioteki, nie nowy manager skills. Priorytet: P1 poprawność importu i odbiór,
P2 szersza zgodność metadanych. Status funkcji i QA prowadzić w roadmapzie #53/#10.

Już istnieją: biblioteka i edycja, discovery, import folderu/GitHub, wieloplikowe
bundles UTF-8, instalacje globalne/projektowe, eksport do innego folderu oraz
aktualizacja/usuwanie z kontrolą hashy i ochroną cudzych plików/symlinków.
Nie implementować tego drugi raz. Ręczny odbiór roadmapy #10 nie jest wykonany
przez samo istnienie testów jednostkowych lub smoke.

`frontmatter_value` w `apps/desktop/src/skills.rs` czyta jedną linię i rozumie
JSON-owe podwójne cudzysłowy, nie pełne YAML. Dla `description: >` zwraca marker
zamiast opisu. `Skill` przechowuje tylko wybrane pola, a `rendered()` generuje
nowy nagłówek, więc dodatkowe metadane oryginału giną. Parser Markdown w vendorze
nie jest parserem importu skills i jego rozszerzenie nie zamyka tego planu.

## Zakres v1 i miejsca integracji

- `apps/desktop/src/skills.rs`: import, model biblioteki, serializacja, hashe,
  rendering i kontrola instalacji; UI Skills w `views/settings.rs`.
- Obsługa YAML frontmatter, nie nowe formaty plików JSON/TOML ani nowy katalog online.
- Zachować obecne limity plików/bundle, ograniczenia ścieżek i brak uruchamiania
  skryptów, instalacji harnessów, promptów lub automatycznych aktualizacji.
- Starsze zapisane biblioteki i rekordy instalacji muszą nadal się otwierać.

## Etap A — parser i walidacja

1. [ ] Wybrać utrzymywany parser YAML po przeglądzie zależności/licencji; nie
   rozbudowywać parsera liniowego do kolejnego niejawnego podzbioru YAML.
2. [ ] Obsłużyć scalars plain/single-quoted/double-quoted, escaping, komentarze,
   `|`, `|-`, `|+`, `>`, `>-`, `>+`, CRLF i opcjonalny UTF-8 BOM. Zachować treść
   body oraz poprawnie oddzielić początkowe frontmatter od separatorów Markdown.
3. [ ] Wymagać pojedynczego mappingu i niepustych stringów `name`/`description`;
   odrzucać duplikaty kluczy, niezamknięty nagłówek, zły typ i przekroczenie limitów.
   Nie konwertować `null`, listy ani liczby na opis przez `to_string()`.
4. [ ] Ograniczyć głębokość/liczbę węzłów i ekspansję; odrzucać nieobsługiwane
   tagi/aliasy zamiast wykonywać konstrukcje lub ładować zasoby zewnętrzne.
   Błędy mają nazwę pliku/pola, nie pełną treść importowanego dokumentu.

## Etap B — metadane i bezpieczny round-trip

1. [ ] Dodać wersjonowany model metadanych z domyślnymi wartościami dla starej
   biblioteki. Rozdzielić wewnętrzne ID Dunlina, oryginalne `name` i tytuł UI;
   ustalić migrację bez zmieniania ścieżek istniejących instalacji.
2. [ ] Zachowywać dozwolone nieznane pola jako dane YAML; minimum obejmuje
   `license`, `compatibility`, `metadata` oraz warianty pól harness-specific.
   Nie obiecywać zachowania komentarzy/formatowania, jeśli parser daje tylko
   semantyczny round-trip. Nie kopiować całego starego nagłówka obok nowych pól.
3. [ ] Ustalić jawne reguły eksportu dla OpenCode/Claude/Codex/Other. Pola wpływające
   na narzędzia, model i wywoływanie (np. `allowed-tools`, `model`,
   `disable-model-invocation`) nie mogą być cicho usunięte ani rozszerzać grantów
   Dunlina. Nieobsługiwany eksport blokować albo wymagać potwierdzonej konwersji
   z podglądem utraconych pól. Reguła bundled commit skill pozostaje restrykcyjna.
4. [ ] Wliczać metadane do hasha pełnego bundle. Aktualizacja biblioteki nie
   nadpisuje zainstalowanych kopii; preview pokazuje również zmiany metadanych.
   Zachować kontrolę własności i odmowę nadpisania plików zmienionych zewnętrznie.

## Etap C — testy i odbiór

- [ ] Fixtures dla każdego wariantu scalarów, Unicode, apostrofów, URL z `#`,
  dwukropków, pustych linii i separatorów `---` wewnątrz body.
- [ ] Negatywne fixtures: duplikaty, zły typ, pusty opis, malformed YAML,
  wielodokumentowe wejście, alias bomb, zbyt głębokie/duże metadane.
- [ ] Import → edycja opisu → eksport → ponowny import zachowuje semantykę
  metadanych i body; migracja starej biblioteki nie zmienia jej instalacji.
- [ ] Instalacja/update/uninstall nadal chronią pliki użytkownika i symlinki;
  odrzucony import nie zostawia częściowego wpisu ani plików.
- [ ] Natywny smoke: wybór/import folderu, import linku na kontrolowanym fixture,
  preview błędu, Save/Cancel i potwierdzenie porzucenia edycji; brak promptu/agenta.
- [ ] Fizyczny odbiór #10: oba motywy, minimalne okno, Tab/Shift-Tab/Escape,
  powrót fokusu, VoiceOver, global/project/Other i zewnętrzna zmiana pliku.
  Real-host security audit/Codex CLI odbierać osobno, tylko za zgodą na host/koszt.

## Kolejność i granice

Najpierw kontrakt metadanych, potem parser i eksport, na końcu integracja UI.
Nie wydawać samego poprawnego odczytu YAML, jeśli kolejna instalacja nadal
bez ostrzeżenia usuwa znaczące pola. Obsługa frontmatter knowledge/artifacts
pozostaje poza zakresem. Dokumentację biblioteki uzupełnić w
[skills](../docs/user-guide/skills.md); backlog: [roadmapa](../docs/project/roadmap.md).
