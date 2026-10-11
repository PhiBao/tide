# Tide

**A composer for time-varying cost.** Run your house at low tide.

Electricity costs a different amount every fifteen minutes. Tide draws that
curve, drops your appliances into its cheapest lawful windows, and **proves** the
answer is optimal — then shows you two bills, line by line, so you can check the
arithmetic yourself.

```
Live:  https://tide.kiter0211.workers.dev
Suite: TestSprite projects 949b0571-* (34 UI) and 235f314b-* (18 API), all passing
```

---

## The problem

Time-of-use pricing is being rolled out as the default tariff, and it is being
applied to households that were never given a way to see it. The marginal price
of a kilowatt-hour varies by 4–10× across a day, and the decision of *when* to
run a dishwasher is worth real money — but the document that determines it is a
40-page PDF rate schedule.

Every existing tool measures the past. Sense discontinued its energy monitor at
the end of 2025; SPAN's own documentation disclaims the accuracy of its cost
simulation; Emporia tells you circuit draw without identifying the devices. The
serious users have hand-rolled the missing feature in Home Assistant YAML and
PyScript. **Nobody has built the thing that actually decides.**

The insight is that a price you can *see* is a price you can plan around. Making
the cost curve touchable is the whole product.

## Who it is for

Homeowners and renters on time-of-use or dynamic pricing who have controllable
loads — an EV, a dishwasher, a water heater, a battery. The initial user is the
kind of person who has already worked out that charging overnight is cheaper,
and is irritated that nothing tells them *how much* or *what else* to move.

## What it does

1. Pick a tariff. A ribbon draws the marginal price for every 15-minute slot of
   the next 24 hours, anchored at midnight **in the tariff's own timezone**.
2. Add your loads: energy required, power limit, deadline. Each load carries the
   slot at which you would naturally start it.
3. Solve. Loads are placed into the cheapest windows that respect the site's
   power ceiling and every deadline.
4. Read the outcome: how much you save versus running things as you do now,
   whether the schedule is *proved* optimal, and two bills side by side.

The default scenario saves **$85.05/month** against $3.60 of unplanned spend for
the same 17 kWh — because the same energy, run at 18:00 instead of midnight,
costs 4.7× as much.

---

## Innovation

Three things are genuinely new here, and none of them is "it uses AI".

### 1. A scheduler that ships with its own proof

Dropping the site cap decouples the loads entirely: each independently runs in
its cheapest permitted slots, and the sum of those per-load optima is a **valid
lower bound** on the coupled problem. If the solver's answer equals that bound,
it is provably the global optimum — no exhaustive search required.

![The proof loop](docs/diagrams/proof-loop.png)

The certificate is what runs at full scale; the brute-force oracle is the
independent check that validates the solver on small instances. Both are real,
and the diagram shows which one carries the load.

The UI shows this as a certificate: `✓ proved optimal`, with the bound and the
actual cost printed side by side so you can see they are the same number. Every
energy tool asks you to trust its optimiser. This one argues its own case with
arithmetic you can check.

### 2. An independently verifiable rate engine

Price is resolved at **minute resolution**, and a slot's energy is attributed
across the rate periods it spans, weighted by minutes. That means:

- A bill's line items always sum exactly to its total (largest-remainder
  allocation), so "the lines don't add up" — the most common utility complaint —
  is structurally impossible.
- The optimiser's cost and a bill over that same schedule are **the same
  integer**, asserted in the test suite. Most energy tools compute them
  separately and quietly disagree.

### 3. A composer, not a monitor

Everyone else ships gauges and usage charts. Tide is the first surface in this
category where the interaction is *placement*: you watch blocks drop into the
troughs.

---

## Hackathon relevance

**Selected track:** Project Awards (TestSprite Season 4), with the CLI
Improvement Bonus as a parallel objective.

**Sponsor technology, meaningfully integrated:**

- **Cloudflare Workers** — the entire backend is one Rust Worker. Not a wrapper
  around a hosted service: the rate engine, the exact scheduler, and the
  brute-force oracle all run inside the Worker, on `wasm32-unknown-unknown`.
- **Workers Static Assets** — the same Worker serves the Next.js static export,
  giving the app a single origin. One URL for TestSprite's UI tests to hit, and
  no cross-origin path to generate auth or CORS false negatives.
- **`workers-rs` with the `http` feature** — an Axum router compiles directly to
  wasm, so `AGENTS.md`'s mandated Rust/Axum stack runs on the free tier without
  compromise.
- **GitHub Actions + Cloudflare previews** — every PR is deployed to its own
  preview and tested against *that* URL by TestSprite, with the result
  blocking the merge.

![The PR gate](docs/diagrams/pr-gate.png)

The target URL is minted by the workflow rather than guessed. TestSprite's
GitHub App route needs a URL *pattern* like `{branch}-worker.domain`, but
Cloudflare's branch→DNS-label transform is undocumented, so `feature/login` may
or may not become `feature-login-tide`. Naming the preview `pr-<number>` is
DNS-safe by construction and removes the guesswork.

**Judging criteria, addressed directly:**

| Criterion | How Tide answers it |
|---|---|
| 40% Project Quality | A polished, deterministic app with deliberate loading, empty and error states; 111 Rust tests behind it (93 in the pure core, 18 over HTTP). Exact integer money throughout — no floats anywhere in the domain — so every figure on screen is reproducible by hand. |
| 40% Tests That Run Themselves | **52 tests, all green.** 31 browser tests gate every PR against that PR's own deployment, 3 that need a database run against the deployed app beside the 18 authored API tests, which assert exact micro-dollar totals. The browser half is flaky at about 5% of results — measured, and stated in the limitations rather than hidden. Coverage spans all four tariffs, load inventory manipulation, schedule recomputation, the cost comparison, the site cap, exact delivery, DST anchoring, the empty state, mobile row layout, and every failure mode. The split is deliberate: **breadth from the browser suite, precision from the API suite.** The API tests are authored Python, never re-authored, and CI runs them strict (`--no-auto-heal`) so their green is their own. The browser suite is plan-driven — a judgement about how to drive the page rather than an assertion about it — and I measure that limit instead of hiding it: replayed verbatim, 22 of the 34 pass and 12 need the agent. Both suites are committed and bound to the ids that run them (`tests/suite.json`); CI proves the two copies agree *before* the suite runs. The gate has caught a real defect: [PR #2](https://github.com/PhiBao/tide/pull/2) went red, was diagnosed, fixed, and went 19/19 green. |
| 20% Innovation | The proof-backed optimiser is the whole product: a relaxation bound certifies optimality on any instance, and a brute-force oracle independently checks small ones — returning `null` rather than guessing when it cannot. No model, no heuristics, no confidence scores. Determinism is what makes the certificate possible, so the design refuses anything that would make it impossible. |
| ∞ Engagement | The long-form write-up above, published during the window; the suite committed in the repo and bound to the ids that execute it, so its claims are checkable rather than asserted; and the bugs told properly rather than summarised — including the one this project got wrong about itself. |

---

## Architecture

![Tide architecture](docs/diagrams/architecture.png)

<sub>Source: [`docs/diagrams/architecture.html`](docs/diagrams/architecture.html) ·
[the proof loop](docs/diagrams/proof-loop.png) ·
[the PR gate](docs/diagrams/pr-gate.png)</sub>

### Why `tide-core` is pure

`tide-core` has no I/O, no async runtime, and no floating point. That single
constraint buys four things:

1. **It compiles to both targets.** The same crate builds for `x86_64`, where
   the property-test corpus runs millions of instances at full speed, and for
   `wasm32-unknown-unknown`, where the identical arithmetic ships to production.
   Production and test cannot drift apart.
2. **Every result is reproducible by hand.** No `f64` summation order, no
   platform-dependent rounding. Money is integer micro-dollars, energy is
   integer watt-hours, and there is exactly one division, at presentation time.
3. **It is fast enough for the free tier.** The Workers free plan allows 10 ms
   of CPU per request. A 96-slot, three-load scenario solves in **61 µs**
   (best of 2000 runs, asserted in the suite) — roughly 160× inside the budget.
   The test guards a generous 2 ms so a loaded CI runner cannot make it flake.
4. **It is testable exhaustively.** The oracle can brute-force small instances,
   and the tests assert the solver matches the true optimum on every one.

### The modules that matter

| Module | Responsibility |
|---|---|
| `money.rs` | Exact fixed-point types and the rounding policy |
| `civil.rs` | Proleptic Gregorian date arithmetic (dependency-free) |
| `zone.rs` | Time zones, DST rules, UTC ↔ local wall clock |
| `timegrid.rs` | The absolute settlement grid and its validation |
| `rates.rs` | Tariffs, minute-resolution prices, auditable bills |
| `solver.rs` | The exact scheduler and its optimality certificate |
| `oracle.rs` | A brute-force reference optimum, for proving the solver |
| `verify.rs` | Running solver and oracle against each other |

### Frontend

Next.js with `output: 'export'`, served as static assets by the same Worker.
No UI library — the components are hand-built against a deliberately small
design system, because the ribbon *is* the interface and nothing may compete
with it.

---

## Running it

### Prerequisites

- Node.js ≥ 20.19, pnpm ≥ 9
- Rust ≥ 1.85 with the `wasm32-unknown-unknown` target
- A Cloudflare account with Workers access

### Install

```bash
git clone https://github.com/PhiBao/tide && cd tide

# Rust workspace
cargo test --workspace          # 85 tests

# Frontend
pnpm --dir web install
pnpm --dir web build            # -> web/out

# Worker
cargo install -q worker-build
(cd crates/worker && worker-build --release)
wrangler deploy                 # or: wrangler dev
```

### Environment variables

None required to run. To point a local dev server at a deployed API:

```bash
NEXT_PUBLIC_API_BASE=https://tide.kiter0211.workers.dev
```

### Commands

| Command | What it does |
|---|---|
| `cargo test --workspace` | The full suite: 111 tests |
| `cargo clippy --workspace --tests -- -D warnings` | Lint; CI treats any warning as a failure |
| `cargo fmt --all -- --check` | Format; verified from a clean clone, not just locally |
| `pnpm --dir web build` | Static frontend export |
| `wrangler deploy` | Deploy the Worker + assets |
| `wrangler preview --name pr-123` | Deploy a branch-scoped preview |

---

## Demo

**The proof is a pull request.** [PR #1](https://github.com/PhiBao/tide/pull/1) is
the submission artefact: its checks tab shows `Checks`, `Preview` and
`TestSprite` all green, with TestSprite dispatching the UI suite against
`https://pr-1-tide.kiter0211.workers.dev` — the preview built from that PR's own
commit — and reporting **18/18 passed**. The suite has grown since; the gate
now runs 34. The live URL below is deployed from `main` by the same pipeline.

**And here is the case worth reading.** [PR #2](https://github.com/PhiBao/tide/pull/2)
is a bug found by hand, turned into a test, and fixed *because* the test failed.
Its run dispatched 19 tests against `https://pr-2-tide.kiter0211.workers.dev`,
the first of them red against the deployed build, and the last
**19/19 green** against the commit that fixed it. That is the whole argument for
self-running tests, and it happened on a real defect rather than a rehearsal:
the `+` / `−` controls changed state and nothing downstream moved.

**Try this first:** open the live URL and watch the ribbon. The trough on the
left is the cheap overnight window; the crimson wall to its right is the
expensive day. Press **Re-solve** and watch the blocks settle into the trough.

**Then:** expand any line in either bill. Every line names its component, its
energy, and the rule that produced it.

**Then:** change a load's energy with the `+` / `−` buttons and watch the saving
figure move — the app re-solves on every change, and the certificate stays green
because the optimum is re-derived, not cached.

**If you want to check our work:** `POST /api/solve/verify` with a small
scenario runs the brute-force oracle and reports either an exact match or
admits the instance was too large to enumerate. It never invents an optimum.

---

## Verification

What is real, what is a fixture, and what is not:

| Claim | Status |
|---|---|
| The rate engine (TOU windows, sub-slot attribution, line-item bills) | **Real and complete.** Covered throughout the 93-test pure-core suite, with hand-worked billing fixtures: 5 kWh in the trough is $0.225 to the micro-dollar, the same energy at midday is $1.20. |
| The scheduler | **Real.** Greedy + local search, proven optimal against a lower bound. |
| The optimality certificate | **Real.** Valid lower bound, checked per solve. |
| The brute-force oracle | **Real.** Exhaustive enumeration with a documented node budget; refuses rather than guessing on large instances. |
| DST handling | **Real.** US and EU rules modelled explicitly, with the local-standard/local-daylight convention distinction the US requires. Covered by the zone tests. |
| Bundled tariffs | **Real published rate structures**, with source URLs and retrieval dates recorded in the API response. Not live feeds — a tariff is a rate *shape*, and Tide's job is the arithmetic over it. |
| Usage series | **Real, importable, and stored.** Paste a CSV export from your utility or smart meter: it is billed by the same engine, exactly, and the intervals are kept in D1 so a whole period can be compared across every tariff. Re-importing the same export stores it once. The bundled demonstration set is clearly labelled as a sample. |
| Usage history | **Real, visible, and per-visitor.** Store an export and every bundled tariff is priced against it, cheapest first, with deltas against both the cheapest and the tariff you are on. Each browser session has its own partition, so one visitor's readings are never mixed into another's, and "Forget my data" removes yours. |
| Hardware integration | **Not implemented, and not claimed.** Tide produces the *decision*; it does not actuate a plug. See Limitations. |

Every number in the UI is computed by the same integer arithmetic the tests
assert on. There is no mock path anywhere in the codebase.

---

## Security and reliability

- **No user data.** No accounts, no PII, nothing to leak. Scenarios are shareable
  by URL.
- **Input validation.** serde deserialisation with explicit types and bounded
  sizes; grids validate slot count and divisibility before any work is done.
- **Failures are typed.** Every error maps to a status code with a stable
  machine-readable `code`. A 422 always means "your input is unusable", never
  "something broke".
- **CPU-bounded.** All loops are bounded by construction; the oracle has a hard
  node budget. Nothing in the request path can spin past a Worker's CPU limit.
- **No panics on wasm.** A `SystemTime` read in the domain layer used to panic on
  `wasm32-unknown-unknown`; it is now an explicit parameter, which also makes the
  horizon a pure function the tests can pin.
- **Secrets.** The TestSprite API key lives as a GitHub Actions secret; nothing
  is committed. `.gitignore` covers `.env*`, `*.pem`, `.dev.vars*`.
- **Fork PRs.** The workflow skips them, so a forked PR cannot spend credits or
  see the key.
- **Dependency surface.** `tide-core` has two dependencies (`serde`,
  `serde_json`), both wasm-safe. The HTTP layer adds Axum and `tower-http`.

---

## Limitations and honest tradeoffs

- **Tiered volumetric rates are not modelled.** TOU windows, fixed charges,
  export credits and (planned) demand charges are; progressive tiers per billing
  period are not. This is the single most common gap in residential tariffs and
  is the first thing on the roadmap.
- **The oracle is small by design.** Exhaustive enumeration is exponential, so it
  runs on instances up to a documented budget and refuses beyond it. The
  *certificate* — the relaxation bound — is what carries at full scale, and it is
  valid on any instance.
- **Tariff data is bundled, not live.** Adding a real utility feed (Green Button)
  is a data-integration problem, not an engineering one, and is deliberately out
  of scope: Tide's value is the arithmetic and the interaction model.
- **History is per-session, not per-account.** A cookie partitions the store so
  one visitor's readings never mix with another's, and "Forget my data" removes
  them. But a partition key is not a credential: it keeps data apart, it does not
  protect it. There is no login. Treat this as a demo-grade boundary, not an
  account system.
- **Scenarios live in the URL, not in the store.** The loads and the horizon are
  encoded in the page rather than persisted, so saving a scenario means saving a
  link. Only imported *usage* is stored durably. Sharing a scenario that way is
  deliberate — it needs no account — but it means a scenario is not a durable
  object you can name and return to.
- **Panels are a long page, not a workspace.** The ribbon, the load manifest, the
  single-day bill, and the tariff ranking are sections of one scrolling page.
  That is the right shape for one decision and the wrong shape for managing a
  year of them.
- **Half the browser suite is re-authored as it runs.** TestSprite executes
  frontend tests with an agent working from the plan, and the agent rewrites the
  generated selectors when it finds them stale. Replayed verbatim, 22 of the 34
  pass; the other 12 need the agent. So a
  green browser run cannot distinguish "the app works" from "the test was
  repaired", and this is the one place where a pass is weaker evidence than it
  looks. That is why the exact assertions live in the API suite, which is
  authored code and runs strict.
- **The monthly figure extrapolates a single modelled day.** The Outcome panel
  multiplies one day's saving by thirty, and now says so rather than presenting
  the result as a bill you would receive. A real month has a different mix of
  days, and a different mix of weather.
- **The browser gate is flaky, and I measured how much.** Across six
  consecutive runs of the 31-test gate, each run failed one or two tests, and
  **never the same ones**: `Re-solve the schedule after changing loads` and
  `Increasing a load's energy re-solves the schedule` on one run, `A rejected
  usage import explains itself` and `Expanding a bill line reveals the rule that
  produced it` on the next, both of which pass in isolation against the very same
  preview. That is 5 failures out of 93 test results, about 5%, spread across four
  different tests -- so it is a per-result rate, not a 5% chance that any
  particular test will fail, and no single test is reliably failing. Lowering concurrency from 4 to 2 did not remove it. The honest
  reading is that this half of the suite is a smoke alarm rather than a
  proof: it catches large breakage and occasionally cries wolf. The 18 API tests
  and the 111 Rust tests are the half whose green means something.
- **The store is tested against production, not on a preview.** The three
  browser tests that need a database run in their own TestSprite project against
  the deployed app, beside the API suite, which is production-scoped for the same
  reason. The per-pull-request gate runs the 31 a preview can host. A preview
  *can* carry a store — it took deploying a real PR-scoped Worker rather than a
  Preview deployment, and that is what the `Preview` job now does — so this is a
  scoping choice rather than a limitation.
- **Aggregation is exact but unindexed.** `history_compare` reads every stored
  interval in the requested range and prices each tariff over all of them. That
  is fine for a year of one household and would want pre-aggregation before it
  met a thousand.
- **Savings figures are illustrative.** The default scenario demonstrates the
  mechanism on a real published rate shape; the per-household figure depends
  entirely on the usage you supply.

---

## Roadmap

1. **Tiered volumetric rates** — the biggest real-world gap.
2. **Battery arbitration** — charge when cheap, discharge when dear, with
   round-trip efficiency and cycle cost modelled. The engine already supports it;
   the UI does not yet.
3. **Durable shared scenarios** via D1, so a link is a real persisted object.
4. **Multi-period usage history** — accumulate 12 months of imported interval
   data and compare a year of every tariff against it, rather than a single
   day.
5. **Beyond electricity.** The engine is domain-agnostic: any horizon + cost
   curve + deadlines + capacity cap is the same problem. Obligation deadlines,
   cloud job scheduling, and toll or charging costs are the same shape.

---

## Repository layout

```
crates/core/     pure domain: money, time, rates, solver, oracle, verify
crates/api/      Axum router, shared by native tests and the Worker
crates/worker/   Cloudflare Worker entrypoint (wasm32)
web/             Next.js frontend, static export
tests/           both suites: suite.json binds each test to the id that runs it
                 backend/  18 authored API tests (the code that runs)
                 ui/       34 browser tests as plans (the code is a run artefact)
scripts/         suite-sync.py: proves the committed suite is the suite that runs
docs/diagrams/   architecture, proof loop, and PR-gate diagrams
.github/         CI: fmt, clippy, test, preview deploy, TestSprite gate
```

## Licence

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. The Rust workspace declares `MIT OR Apache-2.0`, which is the standard dual licence for Rust crates — it keeps the core reusable by other Crates.io packages without forcing a licence choice on them.

## Long-form write-up

[**Building a scheduler that proves its own answer**](docs/building-a-self-proving-scheduler.md)
— why all eighteen browser tests I had at the time passed against a build whose
primary control did nothing, what
that says about what a green suite means, and the three bugs (a date algorithm,
a timezone convention, and 1.4 kWh nobody asked for) that taught me the most.
