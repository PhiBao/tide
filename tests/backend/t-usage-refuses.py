import datetime
import requests

H = {"Content-Type": "application/json"}


def grid_for(tariff_id="flat"):
    """A grid the server chose, so the fixtures always align with the horizon.

    Hard-coding a date fails the next morning: the horizon rolls forward and the
    parser (correctly) refuses a series beginning on a different day.
    """
    now = requests.post(
        "VAR_{url}/api/horizon",
        json={"tariff_id": tariff_id, "slots": 96, "slot_minutes": 15,
              "now_minutes": 20736 * 1440 + 903},
        headers=H, timeout=60,
    ).json()
    return now["start_epoch_minutes"]


def iso(minutes):
    return datetime.datetime.fromtimestamp(minutes * 60, datetime.timezone.utc).strftime(
        "%Y-%m-%dT%H:%M:00Z"
    )


def post(csv, unit="kwh", interval=15, start=None):
    if start is None:
        start = grid_for()
    return requests.post(
        "VAR_{url}/api/usage",
        json={"tariff_id": "flat",
              "grid": {"start_epoch_minutes": start, "slot_minutes": 15, "slots": 96},
              "unit": unit, "interval_minutes": interval, "csv": csv},
        headers=H, timeout=60,
    )


def test_a_series_on_the_wrong_day_is_refused_rather_than_shifted():
    """Guessing at slot alignment is the most common import failure."""
    start = grid_for()
    # A row one day before the horizon's own start.
    r = post(f"timestamp,kwh\n{iso(start - 1440)},1.0")
    assert r.status_code == 422, r.text
    body = r.json()
    assert body["error"]["code"] == "invalid_input"
    assert "align" in body["error"]["message"], body["error"]["message"]


def test_a_fractional_kwh_is_not_rounded_to_a_whole_kwh():
    """0.5 kWh must bill as half a kilowatt-hour, not one.

    An earlier parser used f64 rounding and turned 0.5 into 1, silently doubling
    every fractional reading.
    """
    start = grid_for()
    r = post(f"timestamp,kwh\n{iso(start)},0.5")
    assert r.status_code == 200, r.text
    d = r.json()
    # 0.5 kWh at $0.25/kWh = $0.125 = 125_000 micro-USD.
    assert d["import"]["total_import_wh"] == 500, d["import"]
    assert d["bill"]["total_micro_usd"] == 125_000, d["bill"]


def test_negative_energy_is_rejected_not_clamped():
    r = post(f"timestamp,kwh\n{iso(grid_for())},-1.0")
    assert r.status_code == 422, r.text
    assert "not a non-negative number" in r.json()["error"]["message"]


def test_an_unreadable_timestamp_names_its_line():
    r = post("timestamp,kwh\nnot-a-date,1.0")
    assert r.status_code == 422, r.text
    assert "line 1" in r.json()["error"]["message"]


def test_an_interval_that_does_not_divide_an_hour_is_rejected():
    r = post("kwh\n1.0", interval=7)
    assert r.status_code == 422, r.text
    assert "does not divide an hour" in r.json()["error"]["message"]


def test_rows_beyond_the_horizon_are_counted_not_silently_dropped():
    start = grid_for()
    csv = "timestamp,kwh\n" + "\n".join(f"{iso(start + h * 60)},1.0" for h in range(6))
    small = {"start_epoch_minutes": start, "slot_minutes": 15, "slots": 4}
    r = requests.post(
        "VAR_{url}/api/usage",
        json={"tariff_id": "flat", "grid": small, "unit": "kwh",
              "interval_minutes": 60, "csv": csv},
        headers=H, timeout=60,
    )
    assert r.status_code == 200, r.text
    d = r.json()
    assert d["import"]["intervals_read"] == 6
    assert d["import"]["intervals_outside_horizon"] == 5, (
        "five rows fall outside a one-hour grid; each must be reported, not dropped quietly"
    )
    assert d["import"]["total_import_wh"] == 1_000, "only the in-horizon row is billed"
