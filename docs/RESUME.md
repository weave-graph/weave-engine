# Resume point: roadmap paused on 2026-10-21

The autonomous Phase A → B → C run ([ROADMAP.md](ROADMAP.md)) was paused by
the owner on 2026-10-21 to save token quota. This page records the exact state
and the steps to resume. Design decisions are in [DECISIONS.md](../DECISIONS.md).
Delete this page, or mark it "Superseded", once the run resumes.

## Merged

| Phase | Epics | Evidence |
|---|---|---|
| A (released as **v1.0.0**, tag on `ebfcd46`) | A1 #2, A2 #3, A3 #4, A4 #13, A5 #36, A7 #37, A6 #16; language LA0 #1, LA1 #3 | [CHANGELOG.md](../CHANGELOG.md), release `v1.0.0` |
| B, closed | B1 #38 (PRs #68, #69), B2 #5 (#71, #75, #77), B3 #39 (#70, #72, #73) | per-PR checklists |
| B, partial | B4 #40: part 1 #74 (MCP agent interface on `weave-service`) and part 2 #76 (quotas, minted IDs for everyone) | see "#40" below |

Versions on `main` (`a367f5a`): protocol **1.2.0**, store **32**.

In weave-language, the language is at Phase A parity, and these LB1 parts are
merged: #22, #23 (`weave sdk` stdin mode), #24 (vendor 1.2.0 and
`widen_readers`) and #25 (minted IDs in `check_agent_service.py`). The latest
language `main` is `76d4661`.

## In progress (not merged)

| Work | Branch | PR | Head | State |
|---|---|---|---|---|
| B4 part 3: sandboxed `compile` MCP tool and Python `AgentClient` | `b4-part3-sdk-compile` | #79 | `5ecec27` | `compiler-runtime` red. Still pins weave-language `73def87`; must move to `76d4661`, which has the minted-ID fix |
| B5 part 1: engine review modes, proposals and votes on registry authority, `FactProvenance.review` (contract **1.3.0**, store **33**) | `b5-review-part1` | #78 (draft) | `fee1f51` | Conflicts with `main`. 18 compatibility migration jobs fail, cause not yet diagnosed |

Neither PR contains `Closes #12` or `Closes #41`. #79 references #40, which is
already closed (see below).

## Open blockers and known issues

1. **#40 was closed too early.** #74 (part 1) put the phrase `"Closes #40"`
   inside its "not in this PR" note. GitHub treats that as a closing keyword,
   so merging #74 closed #40 on 2026-10-20 while parts 2 and 3 were still open.
   The orchestrator missed it in review.
   - **On resume:** reopen #40 (`gh issue reopen 40 -R weave-graph/weave-engine`)
     with a comment pointing here.
   - **Going forward:** PR bodies must never contain `close[sd]|fix(es|ed)|resolve[sd] #N`
     except in the final PR that delivers the epic. Write "part 3 will close
     the issue" instead.
2. **#78 compatibility failures.** All `storeNN-migration` jobs and
   "Accepted history across populated store20 upgrade" fail. The cause is
   unconfirmed. The likely one is the store bump 32→33 with workflow or
   script expectations still at 32: B1 hit the same class of failure when
   `--new-marker 31` was not updated. Diagnose from the logs before changing
   anything.
3. **#79 needs to absorb #78's Python changes.** #78 moves Python version
   constants to 1.3.0, so whichever of the two PRs merges second needs a small
   rebase.
4. **LB1 (weave-language#5) stays open** until #79's `compile` tool merges.
   That is the last LB1 item.
5. **Engine CI language pin.** `main`'s `integration.yml` still pins
   weave-language `5d2b9d2`. #79 bumps it; use `76d4661`.

## Next steps to resume, in order

1. Reopen #40, as in blocker 1.
2. **B4 part 3 (#79):**
   - set both weave-language refs in `integration.yml` to
     `76d4661fc5cfbc3f740a169bb29f2c1be284e17f`;
   - get CI green;
   - check every #40 criterion in the PR checklist;
   - merge. #79 must say `Closes #40`.

   After it merges, LB1 can close (weave-language#5), after the PR body's
   check against LB1's criteria.
3. **B5 part 1 (#78):**
   - rebase onto `main` after #79, keeping both sides of DECISIONS, STATUS,
     ROADMAP and CHANGELOG;
   - diagnose and fix the compatibility failures;
   - mark the PR ready and merge it as "Part of #12".
   - Then vendor contract 1.3.0 into weave-language. The B5 agent listed the
     files: changed `Cargo.toml`, `src/{lib,provenance,versions}.rs`,
     `tests/core_v1_fixtures.rs`, `fixtures/core-v1/manifest.json` and
     `negative/unknown-version.json`; added `positive/fact-provenance-review.json`,
     `negative/fact-provenance-self-review.json` and
     `negative/claimed-provenance-forged-review.json`.
4. **B5 part 2:**
   - add service and MCP tools to propose, list pending, accept and reject on
     `weave-service` and `weave-mcp`, extending the B4 tool registry;
   - add paired hidden-input tests;
   - the PR says `Closes #12`.
5. **B6 (#41), Phase B acceptance:** the integrated exit scenario from
   ROADMAP.md.
   - Two teams, each with a human and two agents, on one service.
   - Agents assert from maintenance logs and query each other's published
     knowledge.
   - A customer-restricted graph never influences the other team's results,
     counts, rankings, explanations, embeddings or kNN candidates.
   - The noninterference harness runs at least 10,000 cases per operator; a
     nightly job already does this.
   - Agent revocation applies on the next request.
   - The audit log shows who read and wrote each fact.
   - Follow the A6 pattern: one CI job, one pass/fail, and a release PR (likely
     protocol and minor tag `v1.x`), tagged by the orchestrator.
6. Then Phase C, in the order in ROADMAP.md: {C1, C2, C5} → {C3, C4} → C6.
   Inputs for C4 are recorded as comments on #15.

## Process notes for whoever resumes

- **Merging:** squash-merge only after all checks are green. Delete a branch
  only after confirming its PR state is `MERGED`; deleting after a failed
  merge closed #56. Never delete a branch that a stacked PR targets.
- **Agent setup:** launch engine sub-agents from the weave-engine directory,
  because worktree isolation follows the current directory. Give every agent a
  private scratch subdirectory.
- **12-hour review:** the review cron is
  `~/.local/share/weave-review/review.sh`, and it writes
  `~/work/weave/reviews/latest.md`. It keeps running while paused. Remove it
  with `crontab -e` if no reports are wanted.
