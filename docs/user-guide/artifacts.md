# Artifacts as evidence

Open **Knowledge → Artifacts**. Artifacts are content-addressed blobs with SQLite
metadata, not files silently added to the project repository.

## Capture and inspect

- Command palette → **Save Clipboard as Artifact**.
- Command palette → **Save Current File as Artifact**.
- Knowledge → Artifacts → **Save clipboard**.

Local blobs live beneath the application data directory's `artifacts/<project>/`.
The local UI previews bounded text/images and supports deletion/export. Review
clipboard/file content for secrets before capture or sharing. Capturing evidence
does not prove a claim is true; freshness and provenance are separate concerns.

Direct capture controls on individual chat/tool cards, diffs and check reports
are still remaining work. A complete claims/evidence cockpit must not be inferred
from an artifact link or a storage table; see the [roadmap](../project/roadmap.md).

## Remote sharing

Granted relay projects can upload, preview, share and revoke remote artifacts.
Share links are **public capability URLs** with expiry; the token grants access
to the content. They are not identity-based project invitations. Content is served
with restrictive headers; HTML is plain text. Local-only artifacts can export but
cannot use relay sharing without remote storage.

Revocation stops future access. It cannot recall already downloaded copies.
Keep tokens out of public logs/issues. [Backup](../administration/data-and-recovery.md)
must include blob files as well as metadata if you need the captured content.

Sources: [artifact UI](../../apps/desktop/src/knowledge.rs),
[artifact types/storage](../../crates/artifacts/src),
[relay artifact routes](../../crates/relay/src/api.rs).
