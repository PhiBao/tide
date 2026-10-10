import requests

H = {"Content-Type": "application/json"}


def test_the_horizon_anchors_at_midnight_in_the_tariffs_own_zone():
    """A viewer in any timezone must see a chart whose clock matches its prices."""
    # 2026-10-09T15:03Z. US Eastern is on daylight time (-4), so this is 11:03
    # local and the next local midnight is 2026-10-10T04:00Z.
    now = 20735 * 1440 + 15 * 60 + 3
    r = requests.post(
        "VAR_{url}/api/horizon",
        json={"tariff_id": "overnight-ev", "slots": 96, "slot_minutes": 15, "now_minutes": now},
        headers=H, timeout=60,
    )
    assert r.status_code == 200, r.text
    d = r.json()

    assert d["start_epoch_minutes"] == 20736 * 1440 + 240, (
        f"expected the next Eastern midnight at 04:00Z, got {d['start_epoch_minutes']}"
    )
    assert len(d["slots"]) == 96

    # The trough must LEAD the day, which is the entire point of anchoring in the
    # tariff's zone rather than the viewer's.
    first = d["slots"][0]["mean_micro_usd_per_kwh"]
    later = d["slots"][40]["mean_micro_usd_per_kwh"]
    assert first == 45_000, f"expected the overnight trough to lead, got {first}"
    assert later == 240_000, f"expected midday to be dear, got {later}"
    assert first < later
