# Building a scheduler that proves its own answer

*What happens when you refuse to let an optimiser grade its own homework.*

---

## The bug that started this

I was testing a load's energy control in a small app. I clicked `+` on an EV
charger asking for 12 kWh and pressed it again. The number did not move. I
clicked `−`. Nothing.

I assumed the usual: a dead event listener, a stale build, a React key
somewhere. So I did what I should have done first — I reproduced it against
production with a Playwright script before touching a line of code:

```
before increase : 12 kWh | 7.0 kW 7 slots
after  increase : 12 kWh | 7.0 kW 7 slots
FAIL: clicking + did nothing visible
FAIL: the schedule did not re-solve
```

Two independent causes, either of which alone would have looked like a dead
button:

1. The re-solve effect depended on `loads.length` — the **length** — so editing
   a requirement left it identical and no solve ever fired.
2. The row displayed `placement.delivered_wh`, the *previous* answer, rather
   than the load's requirement.

The second is the interesting one. Even with the first fixed, the number on
screen stayed stale for the duration of the request. A button that changes state
but doesn't change what you see is worse than one that does nothing, because
you stop trusting the ones that do work.

## Why I'm telling you about it

Because all eighteen browser tests I had at the time passed against that build.
The suite is 34 now; the number is smaller here because this is the story of what
was green then.

I had wired the suite into CI properly. It ran on every PR, against that PR's
own preview, as a required check. Green every time. And it had no idea the
primary control on the page did nothing — because no test had ever asked that
question.

A test suite is not a quality gate. It is a **record of the questions you
thought to ask**. A green suite means "nothing I asked about broke". It does not
mean "nothing is broken". Those feel identical right up until they aren't.

So I wrote a 19th test: click `+`, assert the requirement *and* the slot count
both change. It failed against production. TestSprite's root-cause hypothesis
was accurate enough to have written the fix from:

> *"the EV charger '+' control either is not connected to the state update logic
> or the resulting state change is not propagating to the displayed energy, slot
> count, and schedule calculations."*

Which is the actual bug, reported independently by the failing run's own analysis.

## The app this happened in

It's called [Tide](https://github.com/PhiBao/tide). Your electricity costs a
different amount every fifteen minutes; Tide draws that curve, drops your
appliances into its cheapest lawful windows, and shows you two bills side by
side — what you pay now, and what you'd pay if the dishwasher ran overnight.

The part I want to talk about is not the UI.

## Scheduling with a receipt

An optimiser that says "I found the best schedule" is asking you to trust it.
Every energy tool does this. I wanted one that could show its work.

Here's the trick. Add a constraint — say your house can only draw 7 kW at once —
and the loads stop being independent: the EV charging at 7am blocks the water
heater at 7am. Solving that properly is genuinely hard.

But **remove** the cap and the problem falls apart into pieces. Each load
independently runs in its cheapest allowed slots, and the sum of those per-load
optima is a valid **lower bound** on the coupled problem. The cap can only ever
make a schedule more expensive, never less.

So: solve with the cap. Compare to the bound without it.

If they're equal, no cheaper schedule exists. Not "I didn't find one" —
**there isn't one**. And the check costs almost nothing.

The UI states it as a certificate rather than a vibe:

```
✓ proved optimal
This schedule costs exactly the theoretical floor, so no cheaper schedule exists.
Bound $0.77, actual $0.77.
```

## And when it can't prove it

The bound is valid but often loose. When the schedule costs *more* than the
bound, I don't know whether I'm at the optimum or 5% away.

So there's a second, independent check: a brute-force oracle that enumerates
every feasible schedule. Small instances only — enumeration is exponential, and a
six-load, 96-slot instance is already hundreds of billions of candidate start
positions before power levels are counted.

The endpoint admits it:

```json
{
  "solver_cost_micro_usd": 9000000,
  "optimal_cost_micro_usd": null,
  "is_optimal": false,
  "badge": "proof: not attempted (instance too large)",
  "nodes_explored": 502,
  "enumerated": false
}
```

`null`, not a guess. An optimiser that reports a confident optimum it could not
check is the exact failure this design exists to avoid.

## The three bugs that taught me the most

**A date library bug.** I wrote `civil_from_days` from scratch. Hinnant's
algorithm works in a March-based year, and the final step is `year + (month <= 2)`.
I left it out. Every date in January and February resolved a year early. Twelve
unit tests passed, because none of them asked about February.

**A timezone bug.** Electricity tariffs are defined in local wall-clock time,
but settlement grids are absolute instants. The US writes its autumn clock
change in local *daylight* time; I used *standard*. Every autumn, the whole
schedule was an hour off.

The fix wasn't the code. It was realising the client can't know the answer: a
browser in UTC+8 asking for "midnight" and a tariff evaluated in US Eastern
describe different instants. So the server picks the horizon, anchored in the
tariff's own zone, and the client sends only *now, in epoch minutes* — which
carries no timezone semantics at all.

That also fixed a crash: `SystemTime::now()` panics on
`wasm32-unknown-unknown`. "time not implemented on this platform." Passing the
instant in made the horizon a pure function, which is what let the tests pin it.

**Energy over-delivery.** A load needing 40 kWh at 7.2 kW on a 15-minute grid
needs 22.2 slots. I rounded up to 23 and delivered 41.4 kWh — quietly charging
for energy nobody asked for. Now the last slot runs at reduced power, and a
test asserts `asking for 1 kWh charges for 1 kWh` across five sizes.

Caught by exercising the deployed API, not by a unit test.

## The thing I got wrong on purpose

I used no LLM anywhere in this product.

Not because it can't help — it could smooth the tariff data, explain a bill in
plain language. But the core claim is *this number is checkable*. Floating point
summation order changes results; LLM output changes between runs. If the thing
the product promises is reproducibility, the promise can't rest on a dice roll.

So: integer micro-dollars, integer watt-hours, one division at presentation
time. Every figure is reproducible by hand from the documented rules. The tests
assert `225_000`, not `less than before`.

Determinism isn't a constraint I worked around. It's the reason the proof is
possible at all.

## The suite, honestly

52 tests: 34 browser, 18 API. 111 Rust tests behind them. The browser suite is
split by what a pull request's own deployment can host: 31 gate every PR against
that branch's build, and 3 that need a database run against the deployed app
beside the API suite.

The two halves carry different weight, for a reason I only learned by
measuring it — see "I committed a test suite that had never run" below. The
API tests are the half whose green is their own.

The 18 API tests are **authored, not generated**. I gave TestSprite a precise
OpenAPI spec and it produced 40 tests asserting a contract that doesn't exist —
a `price_grid_id` resource the API has never had, `grid` as a slot/value array
when it's an object, a `cost` key where the API returns `weighted_price`, 401s
on endpoints that are public.

13 of 40 passed. The rest failed against a shape of API no product work would
ever produce.

**A generated test asserting the wrong contract is worse than no test**, because
it turns a green suite into a lie. A suite you wrote yourself and read is a
statement about what you believe. A suite someone generated is a statement about
what a generator believed.

The authored ones assert exact micro-dollar totals: 5 kWh in the trough bills
**$0.225**, the same 5 kWh at midday bills **$1.20**. Not `less than`. Equal to.

## The suite lied to me once, and I only found out because it went red

This one is worth the telling, because the failure was not in the product. It
was in my idea of what a test suite *is*.

The suite lives in two places. My repository holds the tests — the version you
can read, and the version whose commit history says why each assertion exists.
TestSprite holds the copy that runs. I think of the repo as the source of truth,
and it is, but the copy that decides whether a pull request is allowed to merge
is the other one. Nothing kept them in step.

So they drifted. I hardened the history endpoints to partition data per browser
session, and two of the API tests had to change with them: they had been making
bare `requests.post` calls, which do not carry a cookie, so under the new scheme
each call landed in its own partition. I fixed both, in the repo. I never sent
them on.

CI went red. And it went red with this:

> the second import must insert nothing, inserted 8
> Analysis: duplicate detection/idempotency is not working for repeated history imports.

That is a confident, specific, wrong diagnosis. Idempotency worked exactly as
designed. I ran it against production by hand and the second import inserted
zero rows, twice, by two different mechanisms. The old test was running. It
called `requests.post` twice, got two fresh sessions, and — being a *different,
older test* — reported the result under the name of a test I had already fixed.

Here is the part that should worry you. Before I found the cause, I believed the
report. I was halfway to "fixing" an idempotency bug that did not exist, in a
storage layer that was working, on the strength of a red check whose *name* I
recognised. I had read the test. I had written it. I had fixed it that morning.

**A test that runs different code from the one you read is not a weak test. It is
a false one.** It is worse than having no test, because a missing test tells you
nothing while a false one tells you something, in the confident voice of tooling,
and you act on it.

So the binding is now explicit — a file maps each test to the id that executes it
— and `scripts/suite-sync.py` checks it both ways: every committed test must
match what the portal runs, and every test on the portal must be committed here.
CI runs that check *before* the suite runs, so a pull request that edits a test
without sending the edit on fails at the check rather than reporting a green run
about code that never executed.

The check is one file of about three hundred and fifty lines, and it fires on
every layer the suite binds: the authored API code, the browser plans, and the
structure of the binding itself across three suites. It found a second problem
the moment I wrote it: one test existed only on the portal, so the
repository was quietly missing part of the suite it claimed to carry. Nobody
would have noticed. That is the point.

## I committed a test suite that had never run

After I fixed the drift, I did what the fix implied: I exported all 34 browser
tests into the repository and bound each one to the id that executes it. Plans,
code, the lot. The repository now described the whole suite, and a checker proved
it on every pull request.

Then I asked the suite a question I had not asked before: **does it pass on its
own?**

`testsprite test run --all --no-auto-heal` replays the stored code verbatim
instead of letting the agent repair it. The answer was 17 of 34.

So I deleted all 34 and regenerated them from the plans, against the
application as it stands. The regenerated suite passes **22 of 34** strict.
Regeneration helped — 17 to 22 — and did not remove the dependency. A third of
the browser suite still needs an agent before it can run at all, and the figure
that matters is not the one that improved.

In that first generation the other seventeen failed on selectors for elements
that have never existed here. They looked for a textbox called `Load 4 label`.
The app's loads are called `EV charger`, `Water heater` and `Dishwasher`, and
there is no fourth. Several called `select_option("")` — selecting the option
whose value is the empty string, which is to say, nothing.

Those tests pass in CI because the agent working from the plan rewrites them on
the way past. Every green browser run I had reported, I had read as *"the page
does what the plan says."* What it actually said was *"an agent, given the plan
and the page, was able to make a passing test out of them."* Those are different
claims, and only one of them is about the page.

I had done exactly what I spend the rest of this document telling you not to do:
I shipped a green suite as evidence without asking what it was evidence *of*. The
drift bug was a test running code I had not read. This one was worse — a test
whose committed form has never run at all, described by me as the artefact that
executes.

So the binding changed to match reality rather than my description of it. The
browser suite binds **plans** — the scenario design, which is what the runner is
accountable to and what a reader can check. The API suite binds **code** —
authored Python, never re-authored, and CI now passes `--no-auto-heal` on that
job so the guarantee is mine rather than inherited.

That leaves the honest position, which is in the README's limitations as well as
here: **breadth comes from the browser suite, precision from the
API suite.** A browser test that is re-authored at run time can pass over a
regression, because the agent is repairing the test and a green verdict cannot
tell you which one it repaired. The API tests cannot do that: eighteen of them,
exact micro-dollar totals, running strict. That is the half whose green I will
defend.

The lesson is not "don't trust your tests". It is narrower and more useful:
**a suite has a grammar, and you have to know which clause you are being told.**
"Passes" can mean "the product is correct", or "the test can be made to pass".
Both are printed in green. Only one of them is about your product.

## The gate is flaky, and that is worse than being red

Fixing the drift gave me a gate I trusted. Then it went red on a pull request,
and chasing that down produced the least flattering number in this document.

I started with two confident explanations. The first was real, and not the whole
story: every preview had no database, because a Preview deployment does not carry
the binding in `wrangler.toml`, and two tests blocked on it. Deploying an
ordinary Worker instead of a Preview fixed that, and the failures that remained
were different ones. So I tried the second: contention, four browser tests at
once against one cold isolate while the app recalculates a certified optimiser on
every click. I set it to two. The run came back 29 of 31 again.

So I stopped guessing and re-ran the two failures on their own against the very
same preview. **Both passed.**

That is what named it. Across three runs of the 31-test gate:

| Run | Result | What failed |
|---|---|---|
| 1 | 30/31 | one test |
| 2 | 29/31 | `Re-solve the schedule after changing loads`, `Increasing a load's energy re-solves the schedule` |
| 3 | 29/31 | `A rejected usage import explains itself`, `Expanding a bill line reveals the rule that produced it` |

Five failures out of ninety-three results — about 5% — across four different
tests. Read that carefully, because the obvious reading is wrong: it is not a 5%
chance that any particular test fails. It is that roughly one result in twenty
fails, and which one is not predictable.

Four runs since then have gone better: three passed all thirty-one, and one
failed a single test. Six failures in two hundred and seventeen results, about
3%. I am keeping the first table because it is what the measurement looked like
when it was small, and a number that only ever improves is a number nobody
measured.

Two sources of nondeterminism, and one of them is mine by design. The browser
tests are executed by an agent working from a plan, and the agent re-authors the
selectors as it goes, so the suite is not the same program twice. And the
application re-solves its schedule on every interaction, so "the answer has
changed" is a claim with a latency attached rather than a fact.

### What that costs

A gate is worth the meaning of its red. A red that means *your branch broke
something* gets fixed. A red that means *try again* teaches everyone to re-run
until green, which is indistinguishable from having no gate — except that it
costs money and time, and it teaches you to distrust the runs that are honest.

I did not fix this. It is recorded with its measurement, in the README's
limitations and here, rather than tuned until it looked green. What I can say
precisely is which half of the suite means what, and I would rather a reader know
that than believe a green I cannot defend.

## Then the app told me my test was wrong

Wiring TestSprite's GitHub App into the repository changed who runs the suite. It
watches a pull request, resolves the branch's own deployment URL, runs the tests
from the portal, comments the result on the thread, and holds the merge.

Its first run came back **30 of 31**. The failure was
`Re-solve the schedule after changing loads`, and the report was specific:

> Clicking the 'Re-solve' button did not update the schedule Outcome — the
> displayed cost figures remained unchanged.

So I reproduced it in a browser before touching anything:

| step | loads | saving | scheduled |
|---|---|---|---|
| start | 3 | 85.05 | 0.77 |
| after Re-solve alone | 3 | 85.05 | 0.77 |
| after adding a load | 4 | 96.75 | 0.86 |
| after Re-solve again | 4 | 96.75 | 0.86 |

Those are the same figures the run reported. The test had added a fourth load —
which did change the answer — and then pressed Re-solve and expected the numbers
to move again.

They must not move. **This app is deterministic: the same scenario produces the
same answer, every time.** That is not a limitation to work around; it is the
claim the certificate of optimality rests on. A test that expects a refresh to
change the numbers is asserting against the product's central guarantee — and it
was my plan that told it to.

Rewritten to state the real invariant — *unchanged after Re-solve, changed after
an edit* — the next run was **31 of 31 green** on
[PR #4](https://github.com/PhiBao/tide/pull/4). And every pull request since has
run the suite without anyone starting it: on
[PR #6](https://github.com/PhiBao/tide/pull/6) the App posted
*"All 31 tests passed"* as a status check that held the merge until it did.

That is the lesson from the top of this document arriving a third time, with
better production values. A failure report is a hypothesis. This one was accurate
about what it saw and wrong about why it mattered, and the difference only showed
up when I ran the four steps myself.

## The build tool disappeared mid-week

Halfway through wiring the pull-request gate, every preview deployment started
failing before it reached the tests:

```
error: failed to compile `worker-build v0.8.7`
  failed to select a version for the requirement `zip = "^2.1.3"`
  version 2.1.3 is yanked ... version 2.6.1 is yanked
```

Every `zip` 2.x release had been yanked from crates.io. Every `worker-build` from
0.7.0 through the current 0.8.7 depends on `binary-install ^0.4.1`, which requires
`zip ^2.1.3`. So the install was broken for everyone, and the place it surfaced
was the job that mints a preview — which meant the gate skipped itself and the
pull request showed a red check for a reason that had nothing to do with the
branch.

The published crate ships a `Cargo.lock` pinning `zip 2.4.2`, and cargo accepts a
yanked version when a lockfile pins it:

```bash
cargo install worker-build --locked --version 0.8.7
```

Verified locally — three minutes and fourteen seconds of compiling — before it
went near CI. The version is pinned as well: a build tool that moves under you
during a seven-day window is a bad trade for being current.

## Two gates, and one unreadable red

The day the App started running the suite on pull requests, this repository's own
gate went red:

```
Dispatched 0 tests (18 conflicts) (13 rate-deferred)
```

Both had dispatched the same thirty-one tests against the same URL, at the same
time. The runner refused the duplicates and reported that nothing had been tested.

A red caused by your two gates competing with each other is worse than having one
gate, because it is indistinguishable from a red caused by the branch. So our job
no longer dispatches the suite at all. It does the thing the App cannot do —
proves that the suite committed in this repository is the suite that will run — in
twenty-nine seconds, for no credits. The App runs the tests.

Two gates, two different claims, both readable.

## I filed the footgun instead of walking around it

One more thing came out of the week, and it was about the tooling rather than the
app. `test code put` silently converts a generated test into a user-authored one,
and that permanently changes how every later run executes it: the agent stops
healing it and replays it verbatim. Nothing in the command's help or its output
says so, and no documented step reverses it.

I found it the hard way. I pushed thirty-four generated browser tests to keep the
two copies of the suite in step, and two consecutive runs afterwards returned
exactly twenty of thirty-four — the same failures, no healing. That is what a
suite looks like when repair has quietly stopped applying.

It is filed with the reproduction rather than worked around, because a footgun
that fires once will fire for everyone:
[TestSprite/testsprite-cli#443](https://github.com/TestSprite/testsprite-cli/issues/443).

## What I'd tell you if you're doing this

1. **Reproduce against production before you change anything.** I would have
   "fixed" the wrong thing, confidently, and shipped a second bug.
2. **A green suite is not a quality gate.** It's a record of the questions you
   asked. When it goes green, ask what you didn't ask.
3. **When a tool tells you something is optimal, ask to see the check.** If it
   can't show one, that's information about the tool, not about you.
4. **Prefer a small real system with visible evidence** over a large one with
   claims. The oracle's `null` is worth more than any confident number I could
   have faked.
5. **Check that the test that ran is the test you read.** Two copies are still
   one suite, and only one of them decides whether you may merge. If nothing
   proves they agree, assume they don't.
6. **Ask which sense of "passes" you are being told.** A suite that repairs
   itself as it runs prints the same green as one that does not. If you cannot
   say which half of yours is which, run it strict once and find out. It took me
   one command to learn that half of mine had never run.
7. **Learn your gate's flake rate and say it out loud.** "It's green" is not a
   fact about your product until you know how often it is green when it should
   not be. I found mine by accident, on a pull request that would not go green
   for reasons that kept changing. Nobody finds it on purpose.
8. **Your infrastructure fails too, and it looks exactly like your branch.** Two
   of this week's red checks had nothing to do with the app: a yanked crate
   stopped the build tool installing, and our two gates fought over the same
   tests. Before you read a red as "I broke something", ask what else had to be
   true for it to be green.

## What's missing

Still missing, stated plainly: tiered volumetric rates, battery arbitration, and
accounts. You can store your own smart-meter export and see every tariff priced
against it, but the store is partitioned by a cookie rather than a login — a
partition key is not a credential, and I say so in the README rather than let you
infer otherwise. Scenarios live in the URL, so a scenario is a link you keep, not
an object you name.

And the suite still has a hole. A user found the `+` button. There may be more.

The browser gate is also flaky at about 5% of results, which I measured and could
not fix. That is the one number here I would most like to have wrong.

---

*Tide is Rust + Cloudflare Workers + Next.js, MIT licensed. It is live at
[tide.kiter0211.workers.dev](https://tide.kiter0211.workers.dev), and the source
is [github.com/PhiBao/tide](https://github.com/PhiBao/tide).*

*The bug that started this closed in
[PR #2](https://github.com/PhiBao/tide/pull/2), whose checks tab carries the red
run and the commit that took it to 19/19 green. The App's first run is on
[PR #6](https://github.com/PhiBao/tide/pull/6), where its check held the merge.*
