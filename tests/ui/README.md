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

These tests are the per-pull-request gate. Every pull request gets its own
preview deployment, and this suite runs against that preview before the branch
can merge — so a regression is caught on the branch that caused it, not after it
reaches production.

## What is in this directory, and why there are two layers

Each test has two artefacts, because a test on TestSprite is both a plan and a
program:

- `plans/<slug>.json` — the steps, in plain language: what the test *was asked
  to do*. This is the scenario design, and it is the part a reader can check
  without reading code.
- `code/<slug>.py` — the Playwright the runner actually executes: what the test
  *does*, including every assertion and its expected value. This is where
  assertion depth lives.

The plan is authored on TestSprite, so the copy here is an export and moves
portal → repo. The code is authored here, so it moves repo → portal.

## These files are the executed artefact, not documentation

Editing a file here does not change what runs. `scripts/suite-sync.py` proves
that every file here matches what the portal holds, in both directions, and CI
runs it before this suite runs — a pull request that edits a test without
sending the edit on fails at that step rather than reporting a green run about
code that never executed.

```bash
python3 scripts/suite-sync.py                # check both suites
python3 scripts/suite-sync.py --push         # send this repo's code to the portal
python3 scripts/suite-sync.py --refresh-plans  # re-export the portal's plans
```

That check exists because the two copies drifted once, silently. A fix landed
here while the portal kept running the previous version, and the only symptom
was a red CI run whose reported cause — "idempotency is not working" — was not
the real one. Nothing was wrong with idempotency. The old test was running.
