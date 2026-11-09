# Dunlin documentation

Native, local-first agent workspace for **macOS 15+ on Apple Silicon**, with an
optional self-hosted macOS/Linux relay. Start with the path that matches your task.

| I want to… | Start here |
| --- | --- |
| Install Dunlin and send my first prompt | [Installation](getting-started/installation.md) → [First session](getting-started/first-session.md) |
| Learn the application | [User guide](user-guide/README.md) |
| Set up a remote machine | [Relay administration](administration/README.md) → [SSH](administration/ssh.md) |
| Find a setting, command or data location | [Reference](reference/README.md) |
| Understand or change the code | [Architecture](architecture/README.md) → [Development](development/README.md) |
| Run tests or prepare a release | [Quality](quality/README.md) |
| See limitations and remaining work | [Product status](project/status.md) → [Roadmap](project/roadmap.md) |
| Investigate an older decision or test run | [Archive](archive/README.md) |

## Reading the documentation

- Active guides describe the **current source tree**, not necessarily the last
  published release. Versioned changes are in the [changelog](../CHANGELOG.md).
- **Implemented** does not mean **manually accepted**. Automated fixtures, live
  provider checks, physical input/TCC and deployed-host acceptance are separate.
  See the [manual acceptance matrix](quality/manual-acceptance.md).
- Local Mac, relay host and SSH target are different scopes. Credentials, paths
  and settings must not be assumed to synchronize between them.
- Commands run from the repository root unless stated otherwise. Screenshots are
  illustrations, not evidence that a feature passed acceptance on this checkout.

## Documentation layout

| Directory | Purpose |
| --- | --- |
| [getting-started/](getting-started/README.md) | Installation and a minimal first-session walkthrough. |
| [user-guide/](user-guide/README.md) | Task-oriented application instructions and feature boundaries. |
| [administration/](administration/README.md) | Relay/SSH setup, backups, diagnostics and recovery. |
| [reference/](reference/README.md) | Configuration, command interfaces, formats and exact contracts. |
| [architecture/](architecture/README.md) | Repository map, data flow, persistence and trust boundaries. |
| [development/](development/README.md) | Contribution setup, CI/CD, shared UI and extension development. |
| [quality/](quality/README.md) | Repeatable checks, coverage, performance and manual acceptance. |
| [project/](project/README.md) | Current product status and the single remaining-work backlog. |
| [archive/](archive/README.md) | Explicitly historical plans, investigations and release evidence. |
| `screenshots/` | Assets used by the [gallery](user-guide/screenshots.md) and README. |

## Repository policies

[Contributing](../CONTRIBUTING.md) · [Security reporting](../SECURITY.md) ·
[License](../LICENSE) · [Third-party notices](../THIRD_PARTY_NOTICES.md) ·
[License texts](../licenses/)

For documentation changes, follow the [maintenance rules](development/documentation.md).
