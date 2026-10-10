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

Because all 18 of my tests passed against that build.

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

33 tests: 19 UI, 14 API. 88 Rust tests behind them.

The 14 API tests are **authored, not generated**. I gave TestSprite a precise
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

## What's missing

Still missing, stated plainly: no persistence, no usage import, tiered
volumetric rates, battery arbitration. You cannot enter your own 12 months of
smart-meter data yet — which is the step that turns a demonstration into a
personal answer.

And the suite still has a hole. A user found the `+` button. There may be more.

---

*Tide is Rust + Cloudflare Workers + Next.js, MIT OR Apache-2.0. The repo
carries the failure bundle from the bug that started this, and the PR that
closed it.*
