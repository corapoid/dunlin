# Tailscale MCP i dedykowana sieć — ustalenia

Sprawdzone 2026-11-10 w oficjalnej dokumentacji Tailscale. Nie wykonywano żadnych
operacji na realnym tailnecie ani nie używano poświadczeń.

## Oficjalne MCP: tak, w Aperture

Aperture udostępnia agregowany endpoint `/v1/mcp` oraz dwa wbudowane konektory
(dokumentacja oznacza je jako **alpha**):

- `Tailnet`: narzędzie `provision_node`, domyślnie widoczne jako
  `Tailnet_provision_node`. Po każdorazowej zgodzie człowieka pozwala dołączyć
  jeden nowy węzeł do istniejącego tailnetu. Wynik zawiera jednorazowy URL odbioru
  klucza, nie sam klucz. Wymaga konfiguracji OAuth i wąskich grantów.
- `TailnetSSH`: wykrywanie maszyn SSH i wykonanie komendy. Nie jest wymagane do
  Dunlin i **nie dodajemy go jako transportu aplikacji ani tylnej furtki**.

Nie ma podstaw do traktowania `provision_node` jako pełnego API tworzenia
tailnetów, edycji ACL/DNS czy zarządzania całym kontem. Nie podmieniamy go
nieoficjalnym npm MCP bez osobnej oceny i decyzji.

## Osobne sieci

Tailscale ma **Tailnets API** do tworzenia sieci API-only (alpha). Takie sieci
mają tylko tagged devices, nie zawierają użytkowników ludzkich i nie są widoczne
w zwykłym admin console. To inna funkcja niż MCP onboardingu urządzenia; nie jest
dobrą domyślną siecią dla zespołu korzystającego bezpośrednio z browsera.

## Decyzja Dunlin

Domyślnie istniejący ludzki tailnet, wydzielone tagi dla controller/worker i wąskie
grants na wymagane porty. Przykładowa przestrzeń nazw tagów (nie gotowa polityka):
`tag:dunlin-controller`, `tag:dunlin-worker`. Rejestracja urządzenia w tailnecie
nie daje automatycznie dostępu do projektów aplikacji.

Aperture MCP jest opcjonalnym sposobem uzyskania zatwierdzonego provisioningu;
nie obowiązkowym LLM gateway ani warunkiem uruchomienia Dunlin. Dostępność,
licencja/plan i tożsamość backendowego klienta Aperture wymagają weryfikacji
przed integracją. Key i collection URL nie mogą trafiać do logów, transkryptów,
snapshotów live ani przeglądarkowego storage. Każda zmiana polityki wymaga preview,
testów i zatwierdzenia administratora. Brak automatycznego tworzenia sieci teraz.

Prywatny `.ts.net` plus poprawny certyfikat nie dowodzi braku Funnel. Test odbioru
musi sprawdzić faktyczną konfigurację Serve/Funnel i tailnet policy. Instalator
nie nadpisuje cudzych tras. Nie włączamy publicznych webhook endpoints; integracje
Linear/GitHub w prywatnym wdrożeniu mają synchronizację przez outbound HTTPS.

## Źródła

- https://tailscale.com/docs/aperture/connectors/built-in-connectors
- https://tailscale.com/docs/aperture/mcp-server
- https://tailscale.com/docs/aperture/how-to/grant-mcp-tool-access
- https://tailscale.com/docs/features/tailnets-api
- https://tailscale.com/docs/features/multiple-tailnets

Nazwy narzędzi mogą zależeć od connector ID; implementacja ma korzystać z discovery,
nie zgadywać parametrów. Ponownie sprawdzić kontrakty przed E4.6.
