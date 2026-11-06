# Eksport i import ustawień oraz struktury projektów

**Settings → General → Export / import settings and project structure**.

1. Wybierz **Settings**, **Project structure** albo oba zakresy.
2. Eksport do JSON pokazuje podgląd przed wyborem pliku docelowego.
3. Eksport do repozytorium wymaga jawnego URL GitHub.com/GitLab.com. Po podglądzie
   i potwierdzeniu tworzy/aktualizuje `dunlin-backup.json` na domyślnej gałęzi
   istniejącego, zainicjalizowanego repozytorium. Zapis tworzy commit; nie wykonuje
   lokalnego `git push`. Aktualizacja używa SHA pliku / `last_commit_id`, bez
   automatycznych ponowień przy konflikcie lub niepewnym wyniku.
4. Import JSON albo import z repozytorium czyta ten sam format i pokazuje podgląd.
   Można wybrać zakres importu i wpisać katalog pobierania lub użyć **Browse…**.
5. **Import selected data** dopisuje brakujące grupy i projekty. Nie klonuje kodu.
   Nowe repozytoria mają ikonę pobierania i status **Not downloaded** na liście.
6. Kliknięcie ikony pobiera pojedyncze repo. Ikona przy grupie pobiera wszystkie
   niepobrane repozytoria tej grupy i podgrup **kolejno**, nie równolegle.
   Każde repo wymaga potem osobnego nadania zaufania przed uruchomieniem agenta.

## Zakres i bezpieczeństwo

- Struktura obejmuje nazwy, grupy i podgrupy, kolory, przypięcia oraz kanoniczne
  adresy HTTPS repozytoriów z GitHub.com/GitLab.com (także podgrupy GitLab).
- Repozytoria lokalne muszą mieć obsługiwany, jednoznaczny Git remote. `origin`
  ma pierwszeństwo. Repo już zaimportowane, lecz niepobrane, również można eksportować.
- **Projekty na zdalnych hostach/relay nie są eksportowane**, nawet jeżeli ich kod
  pochodzi z GitHub/GitLab. Podgląd eksportu zawiera wyraźną listę wykluczonych
  projektów z powodem. Pomijane są też repo bez obsługiwanego remote i kolejne
  wpisy z tym samym URL. Same nazwy wykluczonych projektów nie trafiają do pliku.
- Nie eksportujemy lokalnych ścieżek ani identyfikatorów zawierających ścieżki.
  Import wybiera nowy katalog; checkouty trafiają do
  `<wybrany katalog>/<github.com lub gitlab.com>/<namespace>/<repo>`.
- Ustawienia obejmują przenośne preferencje wyglądu, edytora, czatu, startu,
  powiadomień, paneli i budżetu. Nie obejmują historii, kont relay, SSH, uprawnień,
  zaufania, poleceń uruchomieniowych, sekretów, zmiennych środowiskowych,
  konfiguracji rozszerzeń, skills, nadpisań runtime projektów/grup ani własnych ikon.
- Import nie usuwa istniejących projektów ani sesji. Repozytoria dopasowuje po
  kanonicznym URL bez rozróżniania wielkości liter; istniejący wpis pozostaje
  niezmieniony. Grupy dopasowuje po nazwie i pełnej hierarchii. Ponowny import
  nie tworzy tych samych grup/repozytoriów drugi raz.
- Import katalogu i wybranych preferencji zapisuje się w jednej transakcji SQLite.
  Błędny format, wersja, URL, cykl grup albo błąd zapisu nie zostawia częściowego
  importu. Nieobsługiwane pola maszynowe są odrzucane, nie ignorowane.
- Klonowanie nie nadpisuje ani nie usuwa istniejących katalogów; odrzuca symlinki
  w tworzonych podkatalogach hosta/namespace. Częściowe pliki po błędzie zostają
  na dysku do ręcznego sprawdzenia. Niepobrane/nieudane wpisy zachowują ikonę.
- Autoryzacja pozostaje w `gh`/`glab`. Zaloguj się poza Dunlin przez
  `gh auth login --hostname github.com` / `glab auth login --hostname gitlab.com`.
  Zapis wymaga dostępu do zawartości repo i zgody reguł ochrony gałęzi.
- Zalecane jest **prywatne repozytorium**: same nazwy projektów i preferencje
  mogą być poufne, mimo pomijania sekretów. Dunlin nie tworzy repo ani nie zmienia
  jego widoczności. GitHub Enterprise / self-managed GitLab nie są obsługiwane.
- Format v1 ma limit 1 MiB, 1000 grup i 2000 repozytoriów. Limit pliku odpowiada
  obsłudze base64 w GitHub Contents API. To kopia konfiguracji, nie backup kodu.

## Kontrakt i dowody

Stan początkowy: `38c0853` + istniejące lokalne zmiany. Użyte istniejące kontrakty:

| Twierdzenie | Dowód i lokalizacja | Wersja | Pewność | Brakujące ogniwo | Następna weryfikacja |
|---|---|---|---|---|---|
| Zdalny projekt oznacza projekt relay, nie Git remote | `crates/domain/src/lib.rs`: `RemoteProject`, `Project::is_remote` | workspace 0.1.0-beta.8 | wysoka | brak | test wykluczeń eksportu |
| Obsługiwane tożsamości repo są URL github.com/gitlab.com | `crates/workspace/src/git/hosting.rs`: `RepositoryLink::parse`, `detect` | jak wyżej | wysoka | brak | test rzeczywistych lokalnych Git remote |
| Istnieją bezpieczne klony przez CLI | `github/imports.rs::clone_with`, `gitlab.rs::clone_repository` | jak wyżej | wysoka | brak | test odmowy nadpisania i symlinków |
| Preferencje należą do lokalnego store | `apps/desktop/src/app.rs::StoredSettings`, `stored_settings`; `crates/storage/src/lib.rs` | jak wyżej | wysoka | brak | test transakcji i ponownego otwarcia bazy |
| Repozytorium przechowuje plik base64 z kontrolą rewizji | [GitHub Contents API](https://docs.github.com/en/rest/repos/contents#create-or-update-file-contents), [GitLab Repository Files API](https://docs.gitlab.com/api/repository_files/), `catalog_backup.rs` | REST, format pliku v1 | wysoka | nie wykonano zapisu na prawdziwym koncie | test integracyjny na prywatnym repo testowym |
| CLI akceptuje jawny host, HTTP status oraz JSON przez stdin | lokalne `gh api --help`, [glab api](https://docs.gitlab.com/cli/api/) | zainstalowany gh; glab lokalnie niedostępny | wysoka dla kontraktu, testowy transport | brak testu prawdziwego glab | smoke z zainstalowanym glab |

Przepływ (wszystkie powiązania potwierdzone kodem/kontraktem):

```text
Project.remote --is_remote()--> wykluczenie projektu relay
lokalny Project.path --Git remote/detect--> RepositoryLink --allowlista--> eksport URL
ProjectGroup.id --mapa syntetycznych ID--> przenośna hierarchia
URL repo kopii --gh/glab API(default branch)--> dunlin-backup.json
JSON --walidacja/prepare--> nowe lokalne ID + PendingClone(destination)
potwierdzenie --Store.import_catalog(transaction)--> trwały katalog
ikona pobrania --kolejna operacja clone(url,destination)--> lokalny checkout bez zaufania
```

Odrzucone interpretacje: Git remote nie jest projektem relay; plik eksportu nie jest
zrzutem SQLite; klonowanie nie oznacza zaufania; nazwa repo nie zastępuje URL.
Brak nierozstrzygniętych relacji danych. Otwarta pozostaje weryfikacja na prawdziwych
kontach oraz ręczny smoke natywnego UI. Właściciel następnego kroku: maintainer/QA,
na prywatnym repo testowym, po instalacji i autoryzacji obu CLI.

Testy: `cargo test -p dunlin backup`, `cargo test -p dunlin-workspace catalog_backup`,
`cargo test -p dunlin-storage catalog_import`. Przed wydaniem: pełne testy i ręczny
przebieg export → import na inny katalog → pojedyncze/grupowe pobieranie → restart.

Weryfikacja w aktualnym drzewie: 393 testy aplikacji, 35 testów storage, 17 testów
domain i 3 testy nowej warstwy repozytorium przechodzą. Szeroki równoległy przebieg
workspace zgłosił błędy w innych testach Git/comparison/cleanup; pojedynczo ponowiony
`cleanup_revalidates_ignores_and_repository_identity_and_leaves_nonignored_folders`
przeszedł. Nie traktować całej regresji workspace jako zaliczonej.
