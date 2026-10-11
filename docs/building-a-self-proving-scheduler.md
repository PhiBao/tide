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

Which is the actual bug, found independently, from a failure bundle.

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
every feasible schedule. Small instances only — enumeration is exponential, and
a 96-slot, six-load problem has 10¹⁴ combinations.

The endpoint admits it:

```json
{
  "enumerated": false,
  "optimal_cost_micro_usd": null,
  "badge": "proof: not attempted (instance too large)"
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
assert `375_000`, not `less than before`.

Determinism isn't a constraint I worked around. It's the reason the proof is
possible at all.

## The suite, honestly

52 tests: 34 browser, 18 API. 111 Rust tests behind them.

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

The check cost about eighty lines and fires on all four layers — API code, UI
plans, UI code, and the structure of the binding itself. It found a second
problem the moment I wrote it: one test existed only on the portal, so the
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
that have never existed here. They looked for a textbox called `Load 4 label`. The app's loads
are called `EV charger`, `Water heater` and `Dishwasher`, and there is no fourth.
Several called `select_option("")` — selecting the option whose value is the
empty string, which is to say, nothing.)

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

That leaves the honest position, which is in the README's limitations rather
than buried here: **breadth comes from the browser suite, precision from the
API suite.** A browser test that is re-authored at run time can pass over a
regression, because the agent is repairing the test and a green verdict cannot
tell you which one it repaired. The API tests cannot do that: eighteen of them,
exact micro-dollar totals, running strict. That is the half whose green I will
defend.

The lesson is not "don't trust your tests". It is narrower and more useful:
**a suite has a grammar, and you have to know which clause you are being told.**
"Passes" can mean "the product is correct", or "the test can be made to pass".
Both are printed in green. Only one of them is about your product.

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

## What's missing

Still missing, stated plainly: tiered volumetric rates, battery arbitration, and
accounts. You can store your own smart-meter export and see every tariff priced
against it, but the store is partitioned by a cookie rather than a login — a
partition key is not a credential, and I say so in the README rather than let you
infer otherwise. Scenarios live in the URL, so a scenario is a link you keep, not
an object you name.

And the suite still has a hole. A user found the `+` button. There may be more.

---

*Tide is Rust + Cloudflare Workers + Next.js, MIT OR Apache-2.0. The repo
carries the failure bundle from the bug that started this, and the PR that
closed it.*
