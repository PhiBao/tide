"use client";

/**
 * The instrument.
 *
 * The page is one continuous object rather than a dashboard: a masthead, the
 * ribbon, the load manifest, and the outcome. There is no sidebar, no nav, and
 * no card grid, because the product has exactly one screen's worth of things to
 * say and padding it out with chrome would dilute the one thing that matters.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import Ribbon, { type RibbonLoad } from "@/components/Ribbon";
import {
  ApiError,
  api,
  sampleUsageCsv,
  kwh,
  rate,
  usd,
  type BillResponse,
  type Grid,
  type Placement,
  type Scenario,
  type SolveResponse,
  type TariffSummary,
} from "@/lib/api";

/** Load colours. Assigned in manifest order and stable, so a load keeps its
 *  identity across re-solves and a screenshot matches the running app. */
const LOAD_COLORS = ["#14161c", "#0d6e6e", "#a8253c", "#b8873a", "#2a8f8a", "#c0532f"];

/** How much one press of +/- moves a load's requirement. */
const STEP_WH = 500;
/** Below this a load is not meaningfully schedulable, so the control disables. */
const MIN_ENERGY_WH = 500;

interface Load {
  id: string;
  label: string;
  energy_wh: number;
  max_power_w: number;
  deadline_slot: number;
  earliest_slot: number;
  /** When the household would naturally start it: the baseline runs it here. */
  natural_start_slot: number;
}

/**
 * The starting scenario: a realistic household. Natural starts are when these
 * things actually get switched on — the car on arrival in the evening, the
 * dishwasher after dinner, the water heater with the morning shower — because
 * comparing against midnight would show a saving of zero for a household that
 * overpays every single night.
 */
function initialLoads(slots: number): Load[] {
  return [
    { id: "ev", label: "EV charger", energy_wh: 12000, max_power_w: 7000, deadline_slot: slots - 1, earliest_slot: 0, natural_start_slot: 72 },
    { id: "dish", label: "Dishwasher", energy_wh: 2000, max_power_w: 2000, deadline_slot: Math.floor(slots * 0.6), earliest_slot: 0, natural_start_slot: 80 },
    { id: "heat", label: "Water heater", energy_wh: 3000, max_power_w: 3000, deadline_slot: Math.floor(slots * 0.5), earliest_slot: 0, natural_start_slot: 28 },
  ];
}

export default function Home() {
  const [tariffs, setTariffs] = useState<TariffSummary[]>([]);
  const [tariffId, setTariffId] = useState("");

  const [prices, setPrices] = useState<{ mean_micro_usd_per_kwh: number; weighted_price: number }[]>([]);
  const [loads, setLoads] = useState<Load[]>([]);
  const [solution, setSolution] = useState<SolveResponse | null>(null);
  const [bills, setBills] = useState<{ scheduled: BillResponse; baseline: BillResponse } | null>(null);
  const [expandedLine, setExpandedLine] = useState<string | null>(null);
  const [usageCsv, setUsageCsv] = useState("");
  const [usageUnit, setUsageUnit] = useState<"kwh" | "wh">("kwh");
  const [usageInterval, setUsageInterval] = useState(15);
  const [usageResult, setUsageResult] = useState<{
    import: { intervals_read: number; intervals_outside_horizon: number; total_import_wh: number };
    bill: BillResponse;
  } | null>(null);
  const [usageLoading, setUsageLoading] = useState(false);
  const [usageError, setUsageError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const SLOTS = 96;
  const SLOT_MINUTES = 15;

  // The horizon is chosen by the server, anchored in the tariff's own timezone.
  // The browser's local clock must never decide when "today" starts, because a
  // viewer in UTC+8 and a tariff evaluated in US Eastern are talking about
  // different instants, and the chart would be labelled with a clock that
  // disagrees with the prices it shows.
  const [horizon, setHorizon] = useState<{
    start: number;
    slotMinutes: number;
    slots: number;
  } | null>(null);

  useEffect(() => {
    api
      .tariffs()
      .then((r) => {
        setTariffs(r.tariffs);
        // Prefer the tariff that makes the product's value self-evident.
        const preferred =
          r.tariffs.find((t) => t.id === "overnight-ev") ?? r.tariffs[0];
        if (preferred) setTariffId(preferred.id);
      })
      .catch((e) => setError(e instanceof ApiError ? e.message : String(e)));
  }, []);

  useEffect(() => {
    if (loads.length > 0 || !tariffId) return;
    setLoads(initialLoads(SLOTS));
  }, [tariffId, loads.length]);

  // Ask the server for the horizon and its price series whenever the tariff
  // changes. The response carries the grid's start, so the client never has to
  // reason about a timezone.
  useEffect(() => {
    if (!tariffId) return;
    let cancelled = false;
    api
      .horizon(tariffId, SLOTS, SLOT_MINUTES)
      .then((d) => {
        if (cancelled) return;
        setPrices(d.slots);
        setHorizon({
          start: d.start_epoch_minutes,
          slotMinutes: d.slot_minutes,
          slots: d.slots.length,
        });
      })
      .catch((e: unknown) =>
        setError(e instanceof ApiError ? e.message : String(e)),
      );
    return () => {
      cancelled = true;
    };
  }, [tariffId]);

  const scenario: Scenario = useMemo(
    () => ({
      id: "demo",
      name: "Demo household",
      tariff_id: tariffId,
      grid_start_epoch_minutes: horizon?.start ?? 0,
      slot_minutes: SLOT_MINUTES,
      slots: SLOTS,
      site_cap_w: 7000,
      loads: loads.map((l) => ({ ...l, label: l.label })),
    }),
    [tariffId, horizon, loads],
  );

  const solve = useCallback(async () => {
    if (!tariffId || loads.length === 0) return;
    setLoading(true);
    setError(null);
    try {
      const result = await api.solve(tariffId, scenario);
      setSolution(result);

      // Two bills from the same engine, so the comparison is real: any
      // difference is caused by the schedule, not by the arithmetic.
      // Both usage series come from the server's placements, so the two bills
      // describe exactly the two schedules the engine costed.
      const importScheduled = new Array(SLOTS).fill(0);
      const importBaseline = new Array(SLOTS).fill(0);

      const fill = (target: number[], placements: Placement[]) => {
        for (const placement of placements) {
          for (const [k, slot] of placement.slots.entries()) {
            if (slot >= SLOTS) continue;
            const watts = placement.watts[k] ?? 0;
            target[slot] += (watts * SLOT_MINUTES) / 60;
          }
        }
      };
      fill(importScheduled, result.schedule.placements);
      fill(importBaseline, result.baseline_placements as unknown as Placement[]);

      const g: Grid = {
        start_epoch_minutes: scenario.grid_start_epoch_minutes,
        slot_minutes: SLOT_MINUTES,
        slots: SLOTS,
      };
      const noExport = new Array(SLOTS).fill(0);
      const [scheduled, baseline] = await Promise.all([
        api.bills(tariffId, g, importScheduled, noExport),
        api.bills(tariffId, g, importBaseline, noExport),
      ]);
      setBills({ scheduled, baseline });
    } catch (e) {
      setError(e instanceof ApiError ? `${e.code}: ${e.message}` : String(e));
      setSolution(null);
      setBills(null);
    } finally {
      setLoading(false);
    }
  }, [tariffId, scenario, loads]);

  // Solve whenever anything that changes the answer changes.
  //
  // The dependency used to be `loads.length`, which meant editing a load's
  // energy or deadline changed nothing: the length was identical, so no
  // re-solve fired and the buttons looked broken. This key covers every field
  // the optimiser reads.
  const loadsKey = useMemo(
    () =>
      loads
        .map(
          (l) =>
            `${l.id}:${l.energy_wh}:${l.max_power_w}:${l.deadline_slot}:${l.earliest_slot}:${l.natural_start_slot}`,
        )
        .join("|"),
    [loads],
  );

  useEffect(() => {
    if (!tariffId || loads.length === 0 || prices.length === 0) return;
    // Debounce so a burst of +/- clicks produces one solve rather than five.
    const timer = setTimeout(() => void solve(), 220);
    return () => clearTimeout(timer);
  }, [tariffId, loadsKey, loads.length, prices.length, solve]);

  const billMyUsage = useCallback(async () => {
    if (!tariffId || !horizon || usageCsv.trim().length === 0) return;
    setUsageLoading(true);
    setUsageError(null);
    setUsageResult(null);
    try {
      const result = await api.usage(
        tariffId,
        {
          start_epoch_minutes: horizon.start,
          slot_minutes: horizon.slotMinutes,
          slots: horizon.slots,
        },
        usageCsv,
        usageUnit,
        usageInterval,
      );
      setUsageResult(result);
    } catch (e) {
      // The parser's errors are written for a person: they say which line and
      // why, so they are surfaced verbatim rather than summarised.
      setUsageError(
        e instanceof ApiError ? `${e.code}: ${e.message}` : String(e),
      );
    } finally {
      setUsageLoading(false);
    }
  }, [tariffId, horizon, usageCsv, usageUnit, usageInterval]);

  const ribbonLoads: RibbonLoad[] = useMemo(
    () =>
      (solution?.schedule.placements ?? []).map((placement, i) => ({
        id: placement.load,
        label: placement.load,
        slots: placement.slots,
        watts: placement.watts,
        color: LOAD_COLORS[i % LOAD_COLORS.length],
      })),
    [solution],
  );

  const saving = bills ? bills.baseline.total_micro_usd - bills.scheduled.total_micro_usd : 0;
  const monthly = saving * 30;

  const updateLoad = (id: string, patch: Partial<Load>) =>
    setLoads((prev) => prev.map((l) => (l.id === id ? { ...l, ...patch } : l)));

  const removeLoad = (id: string) => setLoads((prev) => prev.filter((l) => l.id !== id));

  const addLoad = () => {
    const next = loads.length + 1;
    setLoads((prev) => [
      ...prev,
      {
        id: `load-${next}`,
        label: `Load ${next}`,
        energy_wh: 2000,
        max_power_w: 2000,
        deadline_slot: SLOTS - 1,
        earliest_slot: 0,
        natural_start_slot: 72,
      },
    ]);
  };

  return (
    <main className="shell">
      <header className="masthead">
        <div className="masthead-row">
          <div className="wordmark">
            Tide<small>a composer for time-varying cost</small>
          </div>
          <p className="tagline">Run your house at low tide.</p>
        </div>
      </header>

      {/* ---------------------------------------------------------------- */}
      <div className="controls">
        <label htmlFor="tariff" className="rule-label" style={{ margin: 0 }}>
          Tariff
        </label>
        <select
          id="tariff"
          value={tariffId}
          onChange={(e) => setTariffId(e.target.value)}
        >
          {tariffs.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>

        <span className="solving">
          {SLOTS * SLOT_MINUTES / 60}h · {SLOT_MINUTES}min · {(SLOTS * SLOT_MINUTES) / 60}h horizon
        </span>

        <button onClick={() => void solve()} disabled={loading || !tariffId}>
          {loading ? "Solving…" : "Re-solve"}
        </button>
        {loading && <span className="solving" role="status">re-deriving the optimum…</span>}
      </div>

      {/* The ribbon. */}
      <Ribbon prices={prices} loads={ribbonLoads} slotMinutes={SLOT_MINUTES} slots={SLOTS} />

      {error && <pre className="state error" style={{ marginTop: 18 }}>{error}</pre>}

      {/* ---------------------------------------------------------------- */}
      <h2 className="rule-label">Loads</h2>
      <div className="load-list">
        {loads.map((load, i) => {
          const placement = solution?.schedule.placements.find((p) => p.load === load.id);
          // Show the *requirement*, which is what the +/- buttons edit, so the
          // figure responds the instant it is clicked. The placement's delivered
          // energy is the same number once the re-solve lands, but in the gap
          // between edit and solve it is the previous answer — displaying it
          // made the buttons look dead.
          const requirement = load.energy_wh;
          return (
            <div className="load" key={load.id}>
              <div className="load-name">
                <span className="swatch" style={{ background: LOAD_COLORS[i % LOAD_COLORS.length] }} />
                <input
                  aria-label={`${load.label} label`}
                  value={load.label}
                  onChange={(e) => updateLoad(load.id, { label: e.target.value })}
                />
              </div>

              <div className="load-readout">
                <div>{kwh(requirement)}</div>
                <small>by slot {load.deadline_slot}</small>
              </div>

              <div className="load-readout power">
                <div>{(load.max_power_w / 1000).toFixed(1)} kW</div>
                <small>{placement ? `${placement.slots.length} slots` : "—"}</small>
              </div>

              <div className="load-actions">
                <button
                  type="button"
                  disabled={load.energy_wh <= MIN_ENERGY_WH}
                  data-testid={`decrease-${load.id}`}
                  onClick={() =>
                    updateLoad(load.id, {
                      energy_wh: Math.max(MIN_ENERGY_WH, load.energy_wh - STEP_WH),
                    })
                  }
                  aria-label={`decrease ${load.label}`}
                >
                  −
                </button>
                <button
                  type="button"
                  data-testid={`increase-${load.id}`}
                  onClick={() => updateLoad(load.id, { energy_wh: load.energy_wh + STEP_WH })}
                  aria-label={`increase ${load.label}`}
                >
                  +
                </button>
                <button
                  type="button"
                  data-testid={`remove-${load.id}`}
                  onClick={() => removeLoad(load.id)}
                  aria-label={`remove ${load.label}`}
                >
                  ×
                </button>
              </div>
            </div>
          );
        })}
      </div>

      <div style={{ marginTop: 12 }}>
        <button onClick={addLoad}>+ Add a load</button>
      </div>

      {/* ---------------------------------------------------------------- */}
      <h2 className="rule-label">Your own usage</h2>
      <p className="panel-note">
        Paste an interval export from your utility or smart meter. It is billed
        by the same engine as everything else, exactly — the parser refuses to
        guess rather than quietly truncating or rounding your data.
      </p>

      <div className="usage">
        <textarea
          className="usage-input"
          value={usageCsv}
          onChange={(e) => setUsageCsv(e.target.value)}
          spellCheck={false}
          rows={8}
          aria-label="Usage CSV"
          placeholder={"timestamp,kwh\n2026-10-10T00:00:00Z,0.42\n2026-10-10T00:15:00Z,0.40"}
        />

        <div className="usage-controls">
          <label>
            Unit
            <select
              value={usageUnit}
              onChange={(e) => setUsageUnit(e.target.value as "kwh" | "wh")}
            >
              <option value="kwh">kWh</option>
              <option value="wh">Wh</option>
            </select>
          </label>
          <label>
            Interval
            <select
              value={usageInterval}
              onChange={(e) => setUsageInterval(Number(e.target.value))}
            >
              {[5, 10, 15, 20, 30, 60].map((m) => (
                <option key={m} value={m}>
                  {m} min
                </option>
              ))}
            </select>
          </label>

          <div className="usage-buttons">
            <button
              type="button"
              className="primary"
              onClick={() => void billMyUsage()}
              disabled={usageLoading || usageCsv.trim().length === 0 || !horizon}
            >
              {usageLoading ? "Billing…" : "Bill this usage"}
            </button>
            <button
              type="button"
              onClick={() => {
                // Anchored to the horizon the server chose, so the sample always
                // aligns — a hard-coded date stops working the moment the
                // horizon rolls past it.
                setUsageCsv(sampleUsageCsv(horizon?.start ?? 0));
                setUsageUnit("kwh");
                setUsageInterval(15);
              }}
              disabled={!horizon}
            >
              Use a sample
            </button>
          </div>
        </div>
      </div>

      {usageError && (
        <pre className="state error" style={{ marginTop: 12 }}>
          {usageError}
        </pre>
      )}

      {usageResult && (
        <div className="bills" style={{ marginTop: 14 }}>
          <div className="bill">
            <div className="bill-head">
              <h3>Your usage, on this tariff</h3>
              <div className="bill-total">{usd(usageResult.bill.total_micro_usd)}</div>
            </div>
            <BillTable
              bill={usageResult.bill}
              expanded={expandedLine}
              setId={setExpandedLine}
              prefix="yours"
            />
          </div>
          <div className="bill">
            <div className="bill-head">
              <h3>What the parser read</h3>
              <div className="bill-total">{kwh(usageResult.import.total_import_wh)}</div>
            </div>
            <table>
              <tbody>
                <tr>
                  <td>Intervals read</td>
                  <td className="num">{usageResult.import.intervals_read}</td>
                </tr>
                <tr>
                  <td>Outside the horizon</td>
                  <td className="num">
                    {usageResult.import.intervals_outside_horizon}
                  </td>
                </tr>
                <tr>
                  <td>Energy metered</td>
                  <td className="num">{kwh(usageResult.import.total_import_wh)}</td>
                </tr>
              </tbody>
            </table>
            {usageResult.import.intervals_outside_horizon > 0 && (
              <p className="panel-note">
                Rows beyond the horizon are counted here and not billed, so an
                import can never quietly understate a bill.
              </p>
            )}
          </div>
        </div>
      )}

      {/* ---------------------------------------------------------------- */}
      {solution && bills && (
        <>
          <h2 className="rule-label">Outcome</h2>
          <div className="outcome">
            <div>
              <div className="saving-label">Saving versus running it as you do now</div>
              <div className="saving">{usd(monthly, 2)}</div>
              <div style={{ fontSize: 12, color: "var(--ink-3)", marginTop: 4 }}>
                per month · {usd(saving)} today
              </div>
            </div>

            <div>
              <span
                className={`certificate ${solution.optimality === "Proved" ? "proved" : "heuristic"}`}
              >
                {solution.optimality === "Proved" ? "✓ proved optimal" : "≈ bounded search"}
              </span>
              <div className="outcome-note">
                {solution.optimality === "Proved"
                  ? `This schedule costs exactly the theoretical floor, so no cheaper schedule exists. Bound ${usd(solution.lower_bound_micro_usd)}, actual ${usd(solution.schedule.cost_micro_usd)}.`
                  : `${(solution.gap_percent_x1000 / 1000).toFixed(3)}% above the theoretical floor. ${solution.diagnostics[0] ?? ""}`}
              </div>
            </div>
          </div>

          <div className="bills">
            <div className="bill">
              <div className="bill-head">
                <h3>Scheduled</h3>
                <div className="bill-total">{usd(bills.scheduled.total_micro_usd)}</div>
              </div>
              <BillTable bill={bills.scheduled} expanded={expandedLine} setId={setExpandedLine} prefix="sched" />
            </div>
            <div className="bill">
              <div className="bill-head">
                <h3>As you run it now</h3>
                <div className="bill-total">{usd(bills.baseline.total_micro_usd)}</div>
              </div>
              <BillTable bill={bills.baseline} expanded={expandedLine} setId={setExpandedLine} prefix="base" />
            </div>
          </div>
        </>
      )}

      {!solution && !error && (
        <div className="state">
          <span className="solving">Drawing the price curve…</span>
        </div>
      )}

      <footer style={{ marginTop: 56, paddingTop: 16, borderTop: "1px solid var(--hair)", fontSize: 12, color: "var(--ink-3)" }}>
        Tide · every figure is computed exactly and can be checked by hand · the optimiser
        carries its own proof
      </footer>
    </main>
  );
}

/** A hairline bill table. Rows expand to show the rule that produced them. */
function BillTable({
  bill,
  expanded,
  setId,
  prefix,
}: {
  bill: BillResponse;
  expanded: string | null;
  setId: (id: string | null) => void;
  prefix: string;
}) {
  if (bill.lines.length === 0) {
    return <div className="state">No lines to show.</div>;
  }
  return (
    <table>
      <thead>
        <tr>
          <th>Component</th>
          <th style={{ textAlign: "right" }}>Energy</th>
          <th style={{ textAlign: "right" }}>Amount</th>
        </tr>
      </thead>
      <tbody>
        {bill.lines.map((line, i) => {
          const id = `${prefix}-${i}`;
          const open = expanded === id;
          return (
            <>
              <tr className="row" key={id} onClick={() => setId(open ? null : id)}>
                <td>{line.label}</td>
                <td className="num">{kwh(line.energy_wh)}</td>
                <td className="num">{usd(line.amount_micro_usd)}</td>
              </tr>
              {open && (
                <tr className="detail" key={`${id}-detail`}>
                  <td colSpan={3}>
                    {line.detail}
                    {line.rate_micro_usd_per_kwh !== null && (
                      <> · {rate(line.rate_micro_usd_per_kwh)}/kWh</>
                    )}
                  </td>
                </tr>
              )}
            </>
          );
        })}
      </tbody>
    </table>
  );
}
