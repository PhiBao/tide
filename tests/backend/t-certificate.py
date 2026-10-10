import requests

H = {"Content-Type": "application/json"}
SCENARIO = {
    "id": "s", "name": "s", "tariff_id": "flat",
    "grid_start_epoch_minutes": 20735 * 1440, "slot_minutes": 15, "slots": 96,
    "site_cap_w": 4000,
    "loads": [
        {"id": "a", "label": "a", "energy_wh": 1000, "max_power_w": 2000,
         "deadline_slot": 11, "earliest_slot": 0, "prefer_contiguous": False, "natural_start_slot": 72},
        {"id": "b", "label": "b", "energy_wh": 500, "max_power_w": 2000,
         "deadline_slot": 11, "earliest_slot": 0, "prefer_contiguous": False, "natural_start_slot": 72},
    ],
}


def test_a_proved_schedule_sits_exactly_on_its_own_bound():
    """Proved optimal means cost == lower bound, not 'the optimiser finished'."""
    r = requests.post("VAR_{url}/api/solve", json={"tariff_id": "flat", "scenario": SCENARIO},
                      headers=H, timeout=60)
    assert r.status_code == 200, r.text
    d = r.json()

    assert d["optimality"] == "Proved", f"expected a proved result, got {d['optimality']}"
    assert d["gap_percent_x1000"] == 0, "a proved result must have zero gap"
    assert d["schedule"]["cost_micro_usd"] == d["lower_bound_micro_usd"], (
        f"cost {d['schedule']['cost_micro_usd']} must equal the bound {d['lower_bound_micro_usd']}"
    )
    # Exact figure, reproducible by hand: 1500 Wh at $0.25/kWh = $0.375.
    assert d["schedule"]["cost_micro_usd"] == 375_000
    assert d["diagnostics"] == [], "a proved result needs no caveat"
