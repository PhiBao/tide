# The backend suite

These are **authored**, not generated. That distinction matters, and it is the
reason this directory exists.

## Why they are authored

TestSprite's plan generator, given a precise OpenAPI spec, produced 40 tests
that asserted a contract which does not exist:

- a `price_grid_id` resource the API has never had
- `grid` as an array of `{slot, value}` pairs when it is an object
- a `cost` key on each slot when the API returns `weighted_price`
- `401 Unauthorized` for endpoints that are public by design

13 of 40 passed; the other 27 were `blocked` or failing against a shape of API
that no amount of product work would have produced. **A generated test that
asserts the wrong contract is worse than no test**, because it turns a green
suite into a lie.

## What they assert instead

Each test carries an exact expected number wherever the arithmetic allows one,
because the app is exact integer arithmetic with no floating point anywhere. A
test that asserts `cost < baseline` would have passed against a buggy
optimiser; a test that asserts `cost == 375_000` cannot.

| Test | Asserts |
|---|---|
| `t-bill-exact.py` | 5 kWh at a flat $0.25/kWh bills exactly $1.25, and the lines sum to the total |
| `t-invariant.py` | The optimiser's cost equals a bill over that same schedule — one integer |
| `t-certificate.py` | `Proved` means cost == lower bound, to the micro-dollar |
| `t-verify-ok.py` | A brute-force oracle independently reaches the same optimum |
| `t-verify-refuse.py` | Too large → `enumerated: false`, no invented optimum |
| `t-horizon.py` | The grid starts at midnight in the *tariff's* zone, so the trough leads the day |
| `t-exact-delivery.py` | Every load is served exactly; asking for 1 kWh charges for 1 kWh |
| `t-site-cap.py` | The power ceiling is never breached, and no slot is drawn twice |
| `t-tou-exact.py` | The same 5 kWh bills $0.225 in the trough and $1.20 at midday, to the micro-dollar |
| `t-impossible.py` | An infeasible scenario is a 422 naming the offending load |
| `t-unknown-tariff.py` | An unknown tariff is a 404 with a stable code |
| `t-bad-slot-length.py` | A slot length that does not divide an hour is rejected |
| `t-tariff-source.py` | Every bundled tariff records where its numbers came from |
| `t-malformed.py` | Malformed JSON shares one error envelope |

## Running them

```bash
export TESTSPRITE_API_KEY=sk-...
export TESTSPRITE_PROJECT_ID=...   # the backend project

for f in tests/backend/t-*.py; do
  testsprite test create --project "$TESTSPRITE_PROJECT_ID" --type backend \
    --name "$(basename "$f" .py)" --code-file "$f"
done
testsprite test run --all --project "$TESTSPRITE_PROJECT_ID" --wait
```

`VAR_{url}` is substituted by the runner with the target base URL.
