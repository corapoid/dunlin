# Quality and acceptance

| Question | Document |
| --- | --- |
| What does a green check prove? | [Test quality, measured coverage and assertion strength](test-quality.md) |
| How do independent GUI workflows run? | [Desktop GUI scenarios](gui-scenarios.md) |
| How do I manually test a deployed relay with anonymous OpenCode? | [Remote VM and real-host E2E](remote-e2e.md) |
| What still needs a human or real host? | [R11 manual acceptance](manual-acceptance.md) |
| What are Graph/Trace layout and accessibility limits? | [Graph/Trace regression scope](graph-trace-virtualization.md) |
| How were speed and memory measured? | [Performance](performance.md) |
| What must happen before a new release? | [Release process](release-process.md) |
| How are the required checks wired? | [CI/CD](../development/ci-cd.md) |

## Documentation checks

Run from the repository root:

```sh
python3 scripts/check_docs.py
python3 -m unittest discover -s scripts/tests -v
git diff --check
```

The documentation checker validates local Markdown/HTML links, heading anchors,
section indexes and reachability in maintained documentation. It does not make
network requests or establish the truth of feature claims. See the
[maintenance rules](../development/documentation.md).

## Evidence discipline

Automated, manual, real-provider and deployed-host results are separate. A command
listed in a guide is a reproduction recipe, not proof it ran on your checkout.
Test counts in dated ledgers belong only to those runs. Pending manual acceptance
must remain visible and must not be promoted to passed based on a mock fixture.

[Documentation home](../README.md)
