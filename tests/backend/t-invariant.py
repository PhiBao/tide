import requests

H = {"Content-Type": "application/json"}
GRID = {"start_epoch_minutes": 20735 * 1440, "slot_minutes": 15, "slots": 96}
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


def test_the_optimisers_cost_equals_a_bill_over_its_own_schedule():
    """The product's central claim, asserted over HTTP.

    Most energy tools compute a schedule's cost and a bill for the same usage
    separately, then quietly disagree. Here they are one integer, and this test
    is the reason that is more than a sentence in the README.
    """
    base = "VAR_{url}"

    solved = requests.post(f"{base}/api/solve", json={"tariff_id": "flat", "scenario": SCENARIO},
                           headers=H, timeout=60)
    assert solved.status_code == 200, solved.text
    solution = solved.json()

    # Derive the usage from the schedule's own placements: watts * slot_minutes / 60.
    usage = [0] * 96
    for placement in solution["schedule"]["placements"]:
        for slot, watts in zip(placement["slots"], placement["watts"]):
            usage[slot] += watts * 15 // 60

    billed = requests.post(
        f"{base}/api/bills",
        json={"tariff_id": "flat", "grid": GRID, "import_wh": usage, "export_wh": [0] * 96},
        headers=H, timeout=60,
    )
    assert billed.status_code == 200, billed.text
    bill = billed.json()

    assert bill["total_micro_usd"] == solution["schedule"]["cost_micro_usd"], (
        f"the optimiser reported {solution['schedule']['cost_micro_usd']} but a bill over "
        f"its own schedule totals {bill['total_micro_usd']}"
    )
    assert sum(usage) == 1500, f"expected 1500 Wh scheduled, got {sum(usage)}"
