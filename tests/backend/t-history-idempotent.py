import datetime
import requests

H = {"Content-Type": "application/json"}


def iso(minutes):
    return datetime.datetime.fromtimestamp(minutes * 60, datetime.timezone.utc).strftime(
        "%Y-%m-%dT%H:%M:00Z"
    )


def grid_for(tariff_id, now_minutes):
    h = requests.post(
        "VAR_{url}/api/horizon",
        json={"tariff_id": tariff_id, "slots": 96, "slot_minutes": 15,
              "now_minutes": now_minutes},
        headers=H, timeout=60,
    ).json()
    return h["start_epoch_minutes"]


def test_importing_the_same_export_twice_stores_it_once():
    """Re-importing a month must not duplicate a month.

    The unique key is the interval's start time and length, so a household that
    re-exports their utility data after a rate change does not end up billed
    twice for the same electrons.
    """
    start = grid_for("flat", 20736 * 1440 + 903)
    csv = "timestamp,kwh\n" + "\n".join(
        f"{iso(start + i * 15)},{0.5}" for i in range(8)
    )
    body = {
        "grid": {"start_epoch_minutes": start, "slot_minutes": 15, "slots": 96},
        "unit": "kwh",
        "interval_minutes": 15,
        "csv": csv,
    }

    first = requests.post("VAR_{url}/api/history/import", json=body, headers=H, timeout=120)
    assert first.status_code == 200, first.text
    a = first.json()
    assert a["imported"] == 8
    assert a["inserted"] == 8, f"the first import must insert all 8, inserted {a['inserted']}"
    assert a["intervals_already_stored"] is False
    assert a["summary"]["total_import_wh"] == 4000

    second = requests.post("VAR_{url}/api/history/import", json=body, headers=H, timeout=120)
    assert second.status_code == 200, second.text
    b = second.json()
    assert b["imported"] == 8
    assert b["inserted"] == 0, f"the second import must insert nothing, inserted {b['inserted']}"
    assert b["intervals_already_stored"] is True
    assert b["summary"]["total_import_wh"] == 4000, (
        "the stored total must not double on re-import"
    )
