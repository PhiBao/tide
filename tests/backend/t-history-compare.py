import datetime
import requests

H = {"Content-Type": "application/json"}


def iso(minutes):
    return datetime.datetime.fromtimestamp(minutes * 60, datetime.timezone.utc).strftime(
        "%Y-%m-%dT%H:%M:00Z"
    )


def test_every_tariff_is_ranked_and_the_cheapest_is_named_first():
    """The question this exists to answer: which tariff should I be on?

    The ranking is an exact integer comparison over the same stored readings, so
    the order is reproducible and the deltas must agree with the totals.
    """
    now = 20736 * 1440 + 903
    horizon = requests.post(
        "VAR_{url}/api/horizon",
        json={"tariff_id": "flat", "slots": 96, "slot_minutes": 15, "now_minutes": now},
        headers=H, timeout=60,
    ).json()
    start = horizon["start_epoch_minutes"]

    # A day with a quiet night and a heavy evening, so the tariffs separate.
    profile = []
    for i in range(96):
        hour = i / 4.0
        if hour < 6:
            profile.append(0.35)
        elif hour < 17:
            profile.append(0.55)
        else:
            profile.append(1.30)

    csv = "timestamp,kwh\n" + "\n".join(
        f"{iso(start + i * 15)},{v}" for i, v in enumerate(profile)
    )
    imported = requests.post(
        "VAR_{url}/api/history/import",
        json={"grid": {"start_epoch_minutes": start, "slot_minutes": 15, "slots": 96},
              "unit": "kwh", "interval_minutes": 15, "csv": csv},
        headers=H, timeout=180,
    )
    assert imported.status_code == 200, imported.text

    r = requests.post(
        "VAR_{url}/api/history/compare",
        json={"from_epoch_minutes": start, "to_epoch_minutes": start + 1440,
              "current_tariff_id": "flat"},
        headers=H, timeout=180,
    )
    assert r.status_code == 200, r.text
    d = r.json()

    ranked = d["ranked"]
    assert len(ranked) >= 4, f"expected every bundled tariff ranked, got {len(ranked)}"

    # Cheapest first, and each delta must be the difference from the head.
    cheapest = ranked[0]["total_micro_usd"]
    for i, entry in enumerate(ranked):
        assert entry["total_micro_usd"] >= cheapest, (
            f"entry {i} ({entry['tariff_id']}) is cheaper than the head, so the sort is wrong"
        )
        assert entry["delta_vs_cheapest_micro_usd"] == entry["total_micro_usd"] - cheapest
    assert ranked[0]["delta_vs_cheapest_micro_usd"] == 0

    # The current tariff's delta is measured against itself, so it is zero.
    current = next(e for e in ranked if e["tariff_id"] == "flat")
    assert current["delta_vs_current_micro_usd"] == 0, (
        "the current tariff's delta against itself must be zero"
    )
    # And every other tariff's delta against current equals total - flat's total.
    for entry in ranked:
        assert entry["delta_vs_current_micro_usd"] == entry["total_micro_usd"] - current["total_micro_usd"]

    # A uniform evening peak must make the flat tariff dearer than one with a
    # cheap overnight window, or the tariffs are not differentiating anything.
    overnight = next((e for e in ranked if e["tariff_id"] == "overnight-ev"), None)
    if overnight is not None:
        assert overnight["total_micro_usd"] < current["total_micro_usd"], (
            "a time-of-use tariff must beat flat rate on a load with a quiet night"
        )
