import requests

H = {"Content-Type": "application/json"}


def test_the_site_cap_is_never_breached_in_any_slot():
    CAP = 3000
    loads = [
        {"id": f"l{i}", "label": "l", "energy_wh": 3000, "max_power_w": 2000 if i % 2 else 1500,
         "deadline_slot": 95, "earliest_slot": 0, "prefer_contiguous": False, "natural_start_slot": 72}
        for i in range(4)
    ]
    scenario = {
        "id": "s", "name": "s", "tariff_id": "flat",
        "grid_start_epoch_minutes": 20735 * 1440, "slot_minutes": 15, "slots": 96,
        "site_cap_w": CAP, "loads": loads,
    }
    r = requests.post("VAR_{url}/api/solve", json={"tariff_id": "flat", "scenario": scenario},
                      headers=H, timeout=60)
    assert r.status_code == 200, r.text
    d = r.json()

    for slot, draw in enumerate(d["schedule"]["site_draw_w"]):
        assert draw <= CAP, f"slot {slot} drew {draw} W over the {CAP} W cap"
    # and no load draws in the same slot twice
    for placement in d["schedule"]["placements"]:
        assert len(placement["slots"]) == len(set(placement["slots"])), (
            f"load {placement['load']} appears twice in one slot"
        )
