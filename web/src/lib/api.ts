/**
 * The API client.
 *
 * Every call goes to the same origin, because the Rust Worker serves both this
 * static build and the API. One origin means one URL for TestSprite's UI tests
 * to hit, and no cross-origin path to generate auth or CORS false negatives.
 */

export const API_VERSION = 1;

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
