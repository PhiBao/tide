# The UI suite

34 tests that drive the deployed page in a real browser. They are grouped by
what they are responsible for, not by screen:

| Group | What it holds the build to |
|---|---|
| **The proof** | The certificate is shown, the bound equals the schedule's cost, and the wording says *proved* only when a bound exists. |
| **The arithmetic** | Every total on screen equals the total the tariff implies, recomputed from the same inputs. |
| **The schedule** | Each load is served exactly once, no slot exceeds the site cap, and a load's window and duration survive a round trip. |
| **The ranking** | Storing an export prices every tariff against it, the cheapest is called out, and storing twice does not double. |
| **The edges** | A long label does not break the row, a zero-length load is refused, and the page survives a reload. |

Thirty-one of these tests are the per-pull-request gate. Every pull request gets
its own deployment, and those 31 run against it before the branch can merge — so
a regression is caught on the branch that caused it, not after it reaches
production. The other three need a database, so they live in their own TestSprite
project and run against the deployed app beside the API suite.

**The gate is also flaky, at about 5% of results.** Three consecutive runs of the
31 failed one or two tests each and never the same ones, and the two failures
from one run both passed in isolation against the very same deployment. Lowering
concurrency from 4 to 2 did not change it. Two plausible causes: the agent
re-authors these tests as it runs them, so it is not the same program twice, and
the app recalculates a certified optimiser on every interaction, so "the answer
has changed" is a claim with a latency. Treat this half as a smoke alarm rather
than a proof; the API suite is the half whose green means something.

## What is committed here, and what deliberately is not

`plans/` holds one file per test: its steps in plain language, which is the
scenario design and the thing a reader can check without reading code. That is
the contract this suite is accountable to.

**The generated code is not committed, and that is a finding rather than an
omission.** Frontend tests on TestSprite are executed by an agent working from
the plan, and when the agent finds the generated selectors stale it re-authors
them. The stored code is therefore an output of a run, not an input to one.

There is a measured reason to treat it that way. Replaying all 34 tests
verbatim — `testsprite test run --all --no-auto-heal` — passes **22 of 34**.
The other 12 fail against code that names elements the application does not
have, or calls `select_option("")` with no option. Those 12 pass in CI because
the agent repairs them on the way past.

I committed that code once, exported from the portal, and described it as *the
artefact that executes*. That was wrong, and the way it was wrong is worth
stating: it was a description of a suite that had never run in that form. A
strict replay is what exposed it — a check that asked whether the committed
suite passes **on its own** answered 17/34 for that generation.

That generation is gone. The 34 tests were deleted and regenerated from the
plans in this directory, against the application as it stands, and the
regenerated suite measures 22 of 34. So a third of it still depends on the
agent. Regeneration improved the figure; it did not remove the dependency,
which is why the limit is stated here rather than celebrated.

## The consequence, stated plainly

A frontend test that is re-authored at run time can pass over a regression: the
agent is repairing the test, not the application, and a green verdict cannot
distinguish the two. That is a real limit of this half of the suite.

It is bounded by the other half. The 18 API tests are authored Python, never
re-authored — *"code you wrote yourself is never re-authored"* — and CI passes
`--no-auto-heal` on that job so the guarantee is explicit rather than inherited.
They assert exact micro-dollar totals. Breadth comes from these 34; precision
comes from those 18.

## Checking that the committed suite is the suite that runs

`tests/suite.json` binds every test here to the id that executes it, and
`scripts/suite-sync.py` verifies it in both directions for both suites: every
committed artefact must match what the portal holds, and every test on the
portal must be committed here. CI runs that check *before* the suite does.

```bash
python3 scripts/suite-sync.py                  # check both suites
python3 scripts/suite-sync.py --push           # send this repo's version, plans and code
python3 scripts/suite-sync.py --refresh-plans  # re-export plans the portal regenerated
```

The check exists because the two copies drifted once, silently. A fix landed in
the repository while the portal kept running the previous version, and the only
symptom was a red CI run whose reported cause — *idempotency is not working* —
was not the real one. Nothing was wrong with idempotency. The old test was
running.
