import requests

H = {"Content-Type": "application/json"}


def test_every_load_is_served_exactly_with_no_over_delivery():
    """Asking for 1 kWh must charge for 1 kWh, not 1.5."""
    scenario = {
        "id": "s", "name": "s", "tariff_id": "flat",
        "grid_start_epoch_minutes": 20735 * 1440, "slot_minutes": 15, "slots": 96,
        "site_cap_w": 0,
        "loads": [
            {"id": f"l{i}", "label": f"l{i}", "energy_wh": 1000 + 250 * i, "max_power_w": 2000,
             "deadline_slot": 95, "earliest_slot": 0, "prefer_contiguous": False, "natural_start_slot": 72}
            for i in range(5)
        ],
    }
    r = requests.post("VAR_{url}/api/solve", json={"tariff_id": "flat", "scenario": scenario},
                      headers=H, timeout=60)
    assert r.status_code == 200, r.text
    d = r.json()

    for placement, load in zip(d["schedule"]["placements"], scenario["loads"]):
        assert placement["delivered_wh"] == load["energy_wh"], (
            f"load {load['id']} asked for {load['energy_wh']} Wh and got "
            f"{placement['delivered_wh']} Wh"
        )
        assert placement["unmet"] is False
        assert len(placement["slots"]) == len(placement["watts"])
        for w in placement["watts"]:
            assert 0 < w <= 2000, f"unexpected draw {w}"
