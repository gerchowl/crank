# Architecture Decision Records

Every non-trivial design decision in crank is recorded here before it is implemented. Implementation
PRs cite the ADR (and section) they build. A decision changes only through a new ADR that supersedes
the old one, or an amendment to a Proposed ADR backed by evidence (for example a hardware-in-the-loop
result).

| ADR | Title | Status | Issue |
|---|---|---|---|
| [0001](0001-architecture.md) | Architecture, crate layout, Rust↔UI contract, persistence, testing | Proposed | #26 |
| [0002](0002-ble-trainer.md) | Bluetooth stack and Trainer abstraction (FE-C first) | Proposed, Amended (#54) | #27, #54 |

## Process

1. Open an issue labelled `adr` listing the decision points.
2. Copy the template below to `NNNN-short-title.md` (next free number, never reused).
3. PR into `dev`. Every open question is filed as a GitHub issue and linked from the ADR before merge.
4. Status moves `Proposed` → `Accepted` on merge after review (for ADR-0001/0002: a codex astra
   pre-mortem). Later: `Superseded by NNNN` or `Deprecated`, never deleted.

## Template

```markdown
# ADR-NNNN: <title>

- **Status:** Proposed | Accepted | Superseded by ADR-XXXX | Deprecated
- **Date:** YYYY-MM-DD
- **Issue:** #N
- **Related:** <ADRs, issues, spec files>

## 1. Context
What forces the decision: requirements, constraints, facts (cited), what went wrong before.

## 2. Decision
What we do, concretely: crate/module names, type and trait sketches, rates, table names, numbers.
One subsection per decision point of the issue.

## 3. Alternatives considered
Each alternative with the specific reason it was rejected.

## 4. Consequences
What becomes easier, harder, or impossible; follow-up work.

## 5. Open questions
| Question | Issue | Milestone |
|---|---|---|

## 6. References
External sources with URLs (and the date versions were checked); in-repo files.
```
