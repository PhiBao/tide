import datetime
import requests

H = {"Content-Type": "application/json"}

PROFILE = [
    (0, 0, 0.42), (0, 15, 0.40), (0, 30, 0.38), (0, 45, 0.41),
    (1, 0, 0.55), (1, 15, 0.60), (1, 30, 0.58), (1, 45, 0.62),
    (2, 0, 0.71), (2, 15, 0.75), (2, 30, 0.74), (2, 45, 0.78),
    (3, 0, 0.90), (3, 15, 0.94), (3, 30, 0.92), (3, 45, 0.96),
    (4, 0, 1.05), (4, 15, 1.10), (4, 30, 1.08), (4, 45, 1.12),
]
TOTAL_WH = 15010


def iso(minutes):
    """Epoch minutes to an ISO-8601 UTC timestamp."""
    return datetime.datetime.fromtimestamp(minutes * 60, datetime.timezone.utc).strftime(
        "%Y-%m-%dT%H:%M:00Z"
    )


def sample_csv(start_epoch_minutes):
    """Build the sample from the horizon the server chose.

    Hard-coding a date makes this test fail the next morning: the horizon rolls
    forward and the parser (correctly) refuses to align a series beginning on a
    different day.
    """
    rows = ["timestamp,kwh"]
    for i, (_, _, value) in enumerate(PROFILE):
        rows.append(f"{iso(start_epoch_minutes + i * 15)},{value}")
    return "\n".join(rows)


def test_a_pasted_usage_series_is_billed_exactly():
    """The parser is exact, so the bill is reproducible by hand."""
    now = 20736 * 1440 + 15 * 60 + 3
    horizon = requests.post(
        "VAR_{url}/api/horizon",
        json={"tariff_id": "overnight-ev", "slots": 96, "slot_minutes": 15, "now_minutes": now},
        headers=H, timeout=60,
    ).json()

    start = horizon["start_epoch_minutes"]
    r = requests.post(
        "VAR_{url}/api/usage",
        json={
            "tariff_id": "overnight-ev",
            "grid": {"start_epoch_minutes": start, "slot_minutes": 15, "slots": 96},
            "interval_minutes": 15,
            "unit": "kwh",
            "csv": sample_csv(start),
        },
        headers=H, timeout=120,
    )
    assert r.status_code == 200, r.text
    d = r.json()

    assert d["import"]["intervals_read"] == 20
    assert d["import"]["intervals_outside_horizon"] == 0
    assert d["import"]["total_import_wh"] == TOTAL_WH, (
        f"expected {TOTAL_WH} Wh, got {d['import']['total_import_wh']}"
    )

    # The lines must account for every watt-hour read, and add up to the total.
    energy = sum(line["energy_wh"] for line in d["bill"]["lines"])
    assert energy == TOTAL_WH, f"lines cover {energy} of {TOTAL_WH} Wh"
    cents = sum(round(line["amount_micro_usd"] / 10_000) for line in d["bill"]["lines"])
    assert cents == round(d["bill"]["total_micro_usd"] / 10_000)

    # Every row landed in the overnight window, so the whole bill is at $0.045.
    assert len(d["bill"]["lines"]) == 1, d["bill"]["lines"]
    assert d["bill"]["total_micro_usd"] == 225_150 * (TOTAL_WH // 5000), (
        f"unexpected total {d['bill']['total_micro_usd']}"
    )
