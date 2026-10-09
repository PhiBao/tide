"use client";

/**
 * The ribbon.
 *
 * This is the product; everything else on the page explains it, so it gets the
 * care. Two ideas govern the layout:
 *
 * 1. **The load lane sits above the curve, not on it.** A load block pressed
 *    into the middle of the price area is ambiguous — you cannot tell whether it
 *    is sitting in a trough or covering a peak. Giving loads their own lane
 *    makes "where in the tide did my appliances land?" a one-glance question.
 *
 * 2. **Colour maps to price and only to price.** The curve reads as a
 *    temperature gauge without a legend, and load blocks are distinguished by
 *    lane position and the manifest swatch rather than by hue, so the palette
 *    never has to carry two meanings at once.
 */

import { useMemo } from "react";

export interface PriceSlot {
  mean_micro_usd_per_kwh: number;
  weighted_price: number;
}

export interface RibbonLoad {
  id: string;
  label: string;
  slots: number[];
  watts: number[];
  color: string;
}

interface Props {
  prices: PriceSlot[];
  loads: RibbonLoad[];
  slotMinutes: number;
  slots: number;
}

/** Six stops, trough to peak. Mirrors --price-0..--price-5 in globals.css. */
const SCALE = ["#0d6e6e", "#2a8f8a", "#6b9e4a", "#b8873a", "#c0532f", "#a8253c"];

/** Map a price to a colour by ranking it against the horizon's own range. */
function priceColor(price: number, min: number, max: number): string {
  if (max <= min) return SCALE[2];
  const t = (price - min) / (max - min);
  // Slight easing keeps the mid-tones legible instead of collapsing onto amber.
  const eased = Math.pow(t, 0.82);
  return SCALE[Math.min(SCALE.length - 1, Math.floor(eased * SCALE.length))];
}

const LOAD_LANE_HEIGHT = 44;
const BLOCK_H = 9;
const ROW_H = 12;

export default function Ribbon({ prices, loads, slotMinutes, slots }: Props) {
  const width = Math.max(slots * 11, 720);
  const height = 200;
  const pad = { top: 0, right: 0, bottom: 26, left: 0 };
  const plotH = height - pad.top - pad.bottom;
  const plotW = width - pad.left - pad.right;
  const laneY = 3;
  const baselineY = laneY + LOAD_LANE_HEIGHT;

  // Every hook runs before any early return, so the hook order is identical on
  // every render. React error #310 is what happens otherwise, and it only
  // appears once a render takes the early-exit path.
  const axisLabels = useMemo(() => {
    const stepHours = Math.max(1, Math.round((slots * slotMinutes) / 60 / 8));
    const out: number[] = [];
    for (let h = 0; h <= (slots * slotMinutes) / 60; h += stepHours) out.push(h);
    return out;
  }, [slots, slotMinutes]);

  const geometry = useMemo(() => {
    if (prices.length === 0) return null;
    const values = prices.map((p) => p.mean_micro_usd_per_kwh);
    const min = Math.min(...values);
    const max = Math.max(...values);
    // Headroom so the peak never touches the top of the plot.
    const ceiling = max * 1.02;
    const slotW = plotW / slots;

    const bands = values.map((v, i) => {
      const x = pad.left + i * slotW;
      const y = baselineY + plotH - (v / ceiling) * plotH;
      return { x, y, w: slotW, h: baselineY + plotH - y, color: priceColor(v, min, max) };
    });
    return { min, max, bands, slotW };
  }, [prices, slots, plotW, plotH]);

  // One row per load in the lane, so overlapping windows stay legible.
  const laneRows = useMemo(() => {
    return loads.map((load, i) => {
      const bar = load.slots
        .map((slot, k) => ({ slot, watts: load.watts[k] ?? 0 }))
        .filter((d) => d.slot < slots);
      if (bar.length === 0) return null;
      return {
        load,
        bar,
        y: laneY + 3 + (i % 3) * ROW_H,
      };
    })
    .filter((entry): entry is NonNullable<typeof entry> => entry !== null);
  }, [loads, slots]);

  if (!geometry || prices.length === 0) {
    return (
      <figure className="ribbon">
        <div className="state">Pick a tariff to draw its price curve.</div>
      </figure>
    );
  }

  return (
    <figure className="ribbon">
      <svg
        className="ribbon-svg"
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="none"
        role="img"
        aria-label={`Price curve across ${slots} slots of ${slotMinutes} minutes, with ${loads.length} loads placed in its cheapest windows`}
      >
        {/* Hour hairlines, so the curve reads against a clock. */}
        {axisLabels.map((h) => {
          const x = pad.left + ((h * 60) / slotMinutes) * geometry.slotW;
          return (
            <line
              key={`grid-${h}`}
              x1={x}
              x2={x}
              y1={baselineY}
              y2={baselineY + plotH}
              stroke="var(--hair)"
              strokeWidth={1}
            />
          );
        })}

        {/* The price area: one band per settlement slot. */}
        {geometry.bands.map((band, i) => (
          <rect
            key={`band-${i}`}
            x={band.x}
            y={band.y}
            width={Math.max(band.w - 0.5, 0.8)}
            height={band.h}
            fill={band.color}
          />
        ))}

        {/* The baseline the curve sits on, doubling as the load lane's floor. */}
        <line
          x1={pad.left}
          x2={pad.left + plotW}
          y1={baselineY}
          y2={baselineY}
          stroke="var(--ink)"
          strokeWidth={1.5}
        />

        {/* The load lane: a recessed track, clearly separate from the curve, so
            a block reads as an object placed on a shelf rather than a mark
            drawn over data. */}
        <rect
          x={pad.left}
          y={laneY}
          width={plotW}
          height={LOAD_LANE_HEIGHT}
          fill="var(--paper-2)"
        />
        <line
          x1={pad.left}
          x2={pad.left + plotW}
          y1={laneY}
          y2={laneY}
          stroke="var(--ink)"
          strokeWidth={1}
        />

        {laneRows.map(({ load, bar, y }) =>
          bar.map((draw) => (
            <rect
              key={`${load.id}-${draw.slot}`}
              className="load-block"
              x={pad.left + draw.slot * geometry.slotW + 0.8}
              y={y}
              width={Math.max(geometry.slotW - 1.4, 4)}
              height={BLOCK_H}
              rx={0}
              fill={load.color}
            />
          )),
        )}
      </svg>

      <figcaption className="ribbon-axis">
        {axisLabels.map((h) => (
          <span key={`axis-${h}`}>{String(h % 24).padStart(2, "0")}:00</span>
        ))}
      </figcaption>
    </figure>
  );
}
