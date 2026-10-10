/**
 * The API client.
 *
 * Every call goes to the same origin, because the Rust Worker serves both this
 * static build and the API. One origin means one URL for TestSprite's UI tests
 * to hit, and no cross-origin path to generate auth or CORS false negatives.
 */

export const API_VERSION = 1;

/**
 * Format epoch minutes as an ISO-8601 timestamp in UTC.
 *
 * The API speaks epoch minutes everywhere; this is the one place the client
 * turns a number back into a timestamp, and it exists solely so the sample
 * series can be anchored to the horizon the server chose.
 */
export function epochMinutesToIso(minutes: number): string {
  const d = new Date(minutes * 60_000);
  return d.toISOString().replace(/\:\d\d\.\d\d\dZ$/, ":00Z");
}

/**
 * Sample intervals, anchored to the horizon the server chose.
 *
 * Hard-coding a date broke the moment the horizon rolled past it: the parser
 * (correctly) refused to align a series beginning on a different day, so the
 * demo failed with an alignment error the next morning. Anchoring the sample to
 * `horizon.start` means it always matches, whatever day it is read.
 */
export function sampleUsageCsv(startEpochMinutes: number, intervals = 20): string {
  const rows: string[] = ["timestamp,kwh"];
  // A plausible domestic profile: quiet overnight, a morning rise, and a
  // midday ramp. Values are illustrative and labelled as such in the UI.
  const profile = [
    0.42, 0.4, 0.38, 0.41, 0.55, 0.6, 0.58, 0.62, 0.71, 0.75,
    0.74, 0.78, 0.9, 0.94, 0.92, 0.96, 1.05, 1.1, 1.08, 1.12,
  ];
  for (let i = 0; i < intervals; i++) {
    const at = startEpochMinutes + i * 15;
    const value = profile[i % profile.length];
    rows.push(`${epochMinutesToIso(at)},${value.toFixed(2)}`);
  }
  return rows.join("\n");
}

export interface Grid {
  start_epoch_minutes: number;
  slot_minutes: number;
  slots: number;
}

export interface LoadRequest {
  id: string;
  label?: string | null;
  energy_wh: number;
  max_power_w: number;
  deadline_slot: number;
  earliest_slot?: number | null;
  prefer_contiguous?: boolean | null;
  /**
   * Where the household would naturally start this load without planning. The
   * baseline schedule runs it here, so this is what makes the saving figure
   * honest rather than a comparison against midnight.
   */
  natural_start_slot?: number | null;
}

export interface Scenario {
  id: string;
  name?: string | null;
  tariff_id: string;
  grid_start_epoch_minutes: number;
  slot_minutes: number;
  slots: number;
  site_cap_w: number;
  loads: LoadRequest[];
}

export interface Placement {
  load: string;
  slots: number[];
  watts: number[];
  delivered_wh: number;
  unmet: boolean;
  contiguous: boolean;
}

export interface SolveResponse {
  schedule: {
    placements: Placement[];
    site_draw_w: number[];
    cost_micro_usd: number;
  };
  optimality: "Proved" | "Heuristic";
  lower_bound_micro_usd: number;
  gap_percent_x1000: number;
  baseline_cost_micro_usd: number;
  /**
   * Where the household would run each load without planning. Shipped by the
   * server so both bills come from the same engine — a client-side
   * re-derivation would use different assumptions and could show a saving the
   * engine disagrees with.
   */
  baseline_placements: Array<{ load: string; slots: number[]; watts: number[] }>;
  diagnostics: string[];
}

export interface BillLine {
  label: string;
  energy_wh: number;
  rate_micro_usd_per_kwh: number | null;
  amount_micro_usd: number;
  detail: string;
}

export interface BillResponse {
  total_micro_usd: number;
  lines: BillLine[];
}

/** What a set of stored readings covers. */
export interface PeriodSummary {
  from_epoch_minutes: number | null;
  to_epoch_minutes: number | null;
  intervals: number;
  total_import_wh: number;
  total_export_wh: number;
  days_covered: number;
  mean_import_wh: number;
}

/** One tariff's cost over a stored period, ranked against the others. */
export interface RankedTariff {
  tariff_id: string;
  tariff_name: string;
  total_import_wh: number;
  total_micro_usd: number;
  delta_vs_cheapest_micro_usd: number;
  delta_vs_current_micro_usd: number | null;
  lines: Array<{
    label: string;
    energy_wh: number;
    rate_micro_usd_per_kwh: number | null;
    amount_micro_usd: number;
    detail: string;
  }>;
}

export interface TariffSummary {
  id: string;
  name: string;
  zone: string;
  periods: number;
  source_url: string | null;
  source_retrieved: string | null;
}

/** A thrown error carrying the API's stable machine-readable code. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

/**
 * API origin. Empty by default, meaning "same origin" — which is what the
 * Worker serves. Overridable so a local dev server can talk to a deployed API
 * without a proxy.
 */
const BASE = process.env.NEXT_PUBLIC_API_BASE ?? "";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(BASE + path, {
    ...init,
    headers: {
      "content-type": "application/json",
      "x-tide-api-version": String(API_VERSION),
      ...(init?.headers ?? {}),
    },
  });

  const text = await response.text();
  const body = text ? JSON.parse(text) : {};

  if (!response.ok) {
    const error = body?.error ?? {};
    throw new ApiError(
      response.status,
      error.code ?? "unknown",
      error.message ?? `request to ${path} failed with ${response.status}`,
    );
  }
  return body as T;
}

export const api = {
  health: () => request<{ status: string; api_version: number }>("/api/health"),

  tariffs: () => request<{ tariffs: TariffSummary[] }>("/api/tariffs"),

  /**
   * Ask the server for a horizon anchored in the tariff's own timezone, along
   * with its price series. The client must never do timezone arithmetic: a
   * browser's local midnight is meaningless against a tariff evaluated in a
   * different zone, and guessing would label the chart with a clock that
   * disagrees with the prices it shows.
   */
  horizon: (tariffId: string, slots: number, slotMinutes: number) =>
    request<{
      slot_minutes: number;
      start_epoch_minutes: number;
      slots: Array<{
        slot: number;
        start_epoch_minutes: number;
        mean_micro_usd_per_kwh: number;
        weighted_price: number;
      }>;
    }>("/api/horizon", {
      method: "POST",
      // Only "now, in epoch minutes" comes from the client. Every timezone rule
      // — including what "midnight" means for this tariff — stays on the server.
      body: JSON.stringify({
        tariff_id: tariffId,
        slots,
        slot_minutes: slotMinutes,
        now_minutes: Math.floor(Date.now() / 60000),
      }),
    }),

  solve: (tariffId: string, scenario: Scenario) =>
    request<SolveResponse>("/api/solve", {
      method: "POST",
      body: JSON.stringify({ tariff_id: tariffId, scenario }),
    }),

  bills: (tariffId: string, grid: Grid, importWh: number[], exportWh: number[]) =>
    request<BillResponse>("/api/bills", {
      method: "POST",
      body: JSON.stringify({ tariff_id: tariffId, grid, import_wh: importWh, export_wh: exportWh }),
    }),

  /** Read a CSV usage series and bill it, exactly, with the chosen tariff. */
  usage: (
    tariffId: string,
    grid: Grid,
    csv: string,
    unit: "wh" | "kwh",
    intervalMinutes: number,
  ) =>
    request<{
      import: {
        import_wh: number[];
        export_wh: number[];
        intervals_read: number;
        intervals_outside_horizon: number;
        total_import_wh: number;
      };
      bill: BillResponse;
    }>("/api/usage", {
      method: "POST",
      body: JSON.stringify({
        tariff_id: tariffId,
        grid,
        csv,
        unit,
        interval_minutes: intervalMinutes,
      }),
    }),

  /**
   * Store a usage export durably, so a whole period can be compared across
   * tariffs later. Idempotent: re-importing the same export stores nothing
   * twice, because the unique key is the interval's start time and length.
   */
  historyImport: (csv: string, unit: "wh" | "kwh", intervalMinutes: number, startEpochMinutes: number) =>
    request<{
      imported: number;
      inserted: number;
      intervals_already_stored: boolean;
      summary: PeriodSummary;
    }>("/api/history/import", {
      method: "POST",
      body: JSON.stringify({
        grid: { start_epoch_minutes: startEpochMinutes, slot_minutes: 15, slots: 96 },
        csv,
        unit,
        interval_minutes: intervalMinutes,
      }),
    }),

  /** Rank every bundled tariff over what is stored, cheapest first. */
  historyCompare: (currentTariffId?: string) =>
    request<{ summary: PeriodSummary; ranked: RankedTariff[] }>("/api/history/compare", {
      method: "POST",
      body: JSON.stringify({
        from_epoch_minutes: 0,
        to_epoch_minutes: 9_999_999_999,
        current_tariff_id: currentTariffId ?? null,
      }),
    }),

  /** Remove everything stored for this browser session. */
  historyClear: () =>
    request<{ removed: number }>("/api/history/session", { method: "DELETE" }),

  verify: (tariffId: string, scenario: Scenario, nodeBudget?: number) =>
    request<{
      solver_cost_micro_usd: number;
      optimal_cost_micro_usd: number | null;
      is_optimal: boolean;
      badge: string;
      enumerated: boolean;
      nodes_explored: number;
    }>("/api/solve/verify", {
      method: "POST",
      body: JSON.stringify({
        tariff_id: tariffId,
        scenario,
        ...(nodeBudget ? { oracle_node_budget: nodeBudget } : {}),
      }),
    }),
};

/** Format micro-dollars as a human dollar figure. */
export function usd(microUsd: number, decimals = 2): string {
  const cents = Math.round(microUsd / 10_000);
  const sign = cents < 0 ? "-" : "";
  const abs = Math.abs(cents);
  const dollars = Math.floor(abs / 100);
  const remainder = String(abs % 100).padStart(decimals === 2 ? 2 : 0, "0");
  return `${sign}$${dollars.toLocaleString("en-US")}.${remainder}`;
}

/** Format micro-dollars per kWh as a readable rate. */
export function rate(microUsdPerKwh: number): string {
  const dollars = microUsdPerKwh / 1_000_000;
  return `$${dollars.toFixed(3)}`;
}

/** Watt-hours as a readable energy figure. */
export function kwh(wh: number): string {
  const k = wh / 1000;
  return `${k % 1 === 0 ? k.toFixed(0) : k.toFixed(1)} kWh`;
}
