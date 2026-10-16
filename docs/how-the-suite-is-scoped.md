# How the suite is scoped

52 tests, three TestSprite suites, one binding file. This is the runbook: what
runs where, what a green means, and which parts are weak.

| Suite | Tests | TestSprite project | Runs against |
|---|---|---|---|
| `ui` | 31 | Tide (`949b0571-…`) | the pull request's own deployment |
| `storage` | 3 | Tide web - storage (`a54cd337-…`) | production |
| `api` | 18 | Tide API (`235f314b-…`) | production |

Behind them: 111 Rust tests, which need no account and no network.

## Why the split

The 31 in `ui` are the per-pull-request gate. Every PR gets its own deployment,
and those tests run against `pr-N-tide.kiter0211.workers.dev` — the build from
that PR's own commit — before the branch can merge. That is the loop the
hackathon is about, and it is the reason `--target-url` exists.

The other three need a store. A Cloudflare *Preview* deployment does not carry
the D1 binding from `wrangler.toml`, so `/api/history/*` answers
`persistence is not configured for this deployment` on a preview. The
`Preview` job therefore deploys an ordinary PR-scoped Worker instead, which does
carry bindings — but the three tests still run against production, beside the
API suite, because that is where a real store is guaranteed. It keeps the gate
fast and it exercises storage against a real deployment. The cost is honest: a
store regression is caught after merge, not before it.

The 18 `api` tests are scoped to production for a documented reason:
`--target-url` is silently ignored for backend projects — verified by pointing
it at a URL that does not serve this API and watching all 18 pass anyway.

## What a green means

Not the same thing in both halves.

The 18 API tests are **authored Python**, never re-authored, and CI passes
`--no-auto-heal` so the guarantee is the repository's rather than inherited from
a platform default. They assert exact micro-dollar totals: 5 kWh in the trough
bills `225_000`, the same 5 kWh at midday bills `1_200_000`. Not `less than`.

The browser tests are executed by an agent working from a plan, and the agent
re-authors generated selectors as it goes. Replayed verbatim
(`--no-auto-heal`), **22 of 34 pass** and the rest name elements this
application has never had. So a green browser run means "an agent could make a
passing test out of this", which is not the same claim as "the app is correct".

**Breadth comes from the browser suite. Precision comes from the API suite.**

## The flake rate, measured

Three consecutive runs of the 31-test gate failed one or two tests each, and
**never the same ones**:

| Run | Result | What failed |
|---|---|---|
| 1 | 30/31 | one test |
| 2 | 29/31 | `Re-solve the schedule after changing loads`, `Increasing a load's energy re-solves the schedule` |
| 3 | 29/31 | `A rejected usage import explains itself`, `Expanding a bill line reveals the rule that produced it` |

Five failures out of ninety-three results — about 5%, across four different
tests. Both tests from run 2 pass in isolation against the very same preview.
Lowering concurrency from 4 to 2 did not change it.

Two plausible causes, one of them by design: the agent re-authors the tests as it
runs them, so it is not the same program twice; and the app recalculates a
certified optimiser on every interaction, so "the answer has changed" is a claim
with a latency attached.

A red that means *try again* is worse than no gate, because it teaches everyone
to re-run until green. This one is measured and written down instead of tuned
until it looked green.

## Keeping the two copies in step

The tests live in this repository and, once bound, on TestSprite. Only the second
copy decides whether a pull request may merge. They drifted once — a fix landed
here while the portal kept running the previous version, and the symptom was a
red run whose reported cause (*idempotency is not working*) was not the real one.

`tests/suite.json` binds every test to the id that executes it, and
`scripts/suite-sync.py` checks it in both directions for all three suites:

```
$ python3 scripts/suite-sync.py
in sync: 18 api, 31 ui, 3 storage tests, all bound, no drift
```

CI runs that **before** any suite runs, so a pull request that edits a test
without sending the edit on fails at the check rather than reporting a green run
about code that never executed. `--push` sends this repository's version to the
portal; `--refresh-plans` re-exports a plan the portal regenerated.

## Why there is no model in the core

Determinism is what makes the certificate of optimality possible: a relaxation
lower bound proves the schedule optimal on any instance, and a brute-force
oracle independently checks small ones, returning `null` rather than guessing
when it cannot. Floating-point summation order and model output both vary
between runs, so neither is allowed anywhere near a number the product claims
is reproducible. The one allowance is the browser suite, above, where the cost
is stated.
