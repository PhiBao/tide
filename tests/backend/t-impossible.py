import requests

H = {"Content-Type": "application/json"}


def test_an_impossible_scenario_is_a_422_that_names_the_load():
    """A bad request must say which load and why, not silently do its best."""
    scenario = {
        "id": "s", "name": "s", "tariff_id": "flat",
        "grid_start_epoch_minutes": 20735 * 1440, "slot_minutes": 15, "slots": 96,
        "site_cap_w": 0,
        "loads": [
            {"id": "ev", "label": "EV", "energy_wh": 40000, "max_power_w": 7000,
             "deadline_slot": 2, "earliest_slot": 0,
             "prefer_contiguous": False, "natural_start_slot": 0},
        ],
    }
    r = requests.post("VAR_{url}/api/solve", json={"tariff_id": "flat", "scenario": scenario},
                      headers=H, timeout=60)
    assert r.status_code == 422, f"expected 422, got {r.status_code}: {r.text}"
    body = r.json()
    assert body["error"]["code"] == "invalid_input"
    assert "ev" in body["error"]["message"], body["error"]["message"]
