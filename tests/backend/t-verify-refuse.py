import requests

H = {"Content-Type": "application/json"}


def test_the_oracle_refuses_rather_than_inventing_an_optimum():
    """Six 20 kWh loads on a 96-slot grid is beyond exhaustive enumeration.

    The API must say so. An optimiser that reports a confident optimum it could
    not check is the failure this product exists to avoid.
    """
    loads = [
        {"id": f"l{i}", "label": "l", "energy_wh": 20000, "max_power_w": 7000,
         "deadline_slot": 95, "earliest_slot": 0, "prefer_contiguous": False, "natural_start_slot": 72}
        for i in range(6)
    ]
    scenario = {
        "id": "big", "name": "big", "tariff_id": "overnight-ev",
        "grid_start_epoch_minutes": 20735 * 1440, "slot_minutes": 15, "slots": 96,
        "site_cap_w": 0, "loads": loads,
    }
    r = requests.post(
        "VAR_{url}/api/solve/verify",
        json={"tariff_id": "overnight-ev", "scenario": scenario, "oracle_node_budget": 200},
        headers=H, timeout=120,
    )
    assert r.status_code == 200, r.text
    d = r.json()

    assert d["enumerated"] is False, "must not claim to have enumerated this instance"
    assert d["optimal_cost_micro_usd"] is None, "must not report an optimum it did not compute"
    assert d["is_optimal"] is False
    assert "too large" in d["badge"], d["badge"]
