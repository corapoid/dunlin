# Produkt i scenariusze odbioru

## Nawigacja

Workspace → Projekty / Sesje / Workflow / Agenci / Do akceptacji / Linear /
Hosty / Ustawienia. Projekt jest podstawowym kontekstem, chat jednym z widoków.

| Widok | Układ | Dane i stany |
| --- | --- | --- |
| Projekty | Mesh-inspired cards/table/kanban | granty, hosty, aktywność, puste, filtry |
| Sesja | Echo-inspired lista/chat/inspector | streaming, zgody, context, diffs, offline |
| Agent | Flowly-inspired builder + test | harness/model, instrukcje, MCP, limits, capabilities |
| Workflow | paleta/graf/inspector/logi | draft/published/run, schema errors, approvals |
| Hosty | katalog + szczegóły | Tailscale identity, version, capabilities, health, drain |
| Linear | inbox + powiązania | issue/status/comments/run/PR, sync freshness/errors |
| Akceptacje | inbox + evidence | wersja artefaktu/SHA, decyzja, uprawnienia, expiry |

Wszystkie widoki wymagają loading/empty/error/forbidden/offline/stale, keyboard,
light/dark i reduced-motion. Brak demo danych udających dane serwera. Brak pełnego
Miro, współedycji grafu i premium ReUI w pierwszym wydaniu.

## Technologia

React/TypeScript/Vite, Tailwind v4, darmowe ReUI/shadcn w jednym wariancie Base UI.
React Flow OSS jako mechanika grafu. API queries nie mogą uruchamiać pollingu UI;
push przez WebSocket/AHP. Wyjścia i porty workflow mają JSON Schema.
ReUI MCP/skill to narzędzia development, nie runtime zależny od konta ReUI.

## Linear Publisher

Issue/brief → Writer → wersjonowany markdown → Human Review → [comments → bounded
Repeat Writer | accept → Developer/Publisher → repo/checks/approved merge → deployment
confirmation → article URL → Linear Done]. Narzędzia autora wybierane przez
capabilities MCP; istnienie konkretnych serwerów Yoast/FB/Excalidraw nie jest założeniem.

Zmiana tekstu unieważnia akceptację. Sama odpowiedź agenta lub merge nie dowodzi
wdrożenia. Publikacja wymaga skonfigurowanego kontraktu i sprawdzenia URL.

## Linear Product

Issue → host/project binding → isolated workspace → Developer → checks/commit →
push/PR → Human Review → [comments → bounded Repeat | approved SHA + checks → merge
→ Linear Done]. GitHub branch protection pozostaje nadrzędne. Brak unattended merge
bez jawnej polityki i zgody. Bot comments nie wyzwalają nieskończonej pętli.

## Końcowy odbiór

Dwie osoby i dwa hosty; brak cudzych prywatnych sesji; jednorazowa zgoda;
odłączenie browsera; restart controller/worker; offline; duplicate issue/webhook;
cross-host duplicate dispatch; odwołanie członka/hosta; zmiana SHA; niejednoznaczny
push/publish; MCP 2025/2026 i A2A; odtworzenie backup; Safari/Chrome/Firefox;
prywatny Tailscale bez Funnel. Fixtures nie zastępują prawdziwego keyringu/OAuth.
