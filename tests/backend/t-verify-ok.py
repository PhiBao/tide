import requests

H = {"Content-Type": "application/json"}
SCENARIO = {
    "id": "s", "name": "s", "tariff_id": "flat",
    "grid_start_epoch_minutes": 20735 * 1440, "slot_minutes": 15, "slots": 96,
    "site_cap_w": 0,
    "loads": [
        {"id": "a", "label": "a", "energy_wh": 1000, "max_power_w": 2000,
         "deadline_slot": 11, "earliest_slot": 0, "prefer_contiguous": False, "natural_start_slot": 72},
        {"id": "b", "label": "b", "energy_wh": 500, "max_power_w": 2000,
         "deadline_slot": 11, "earliest_slot": 0, "prefer_contiguous": False, "natural_start_slot": 72},
    ],
}


def test_the_oracle_confirms_the_solver_on_a_small_instance():
    """An independent brute force, not the optimiser grading its own homework.

    The window is deliberately narrow: exhaustive enumeration is exponential, so
    the oracle is only exact on instances small enough to enumerate. Narrowing
    the window is what makes this a genuine proof rather than a claim that the
    budget was never hit.
    """
    r = requests.post(
        "VAR_{url}/api/solve/verify",
        json={"tariff_id": "flat", "scenario": SCENARIO, "oracle_node_budget": 50_000},
        headers=H, timeout=120,
    )
    assert r.status_code == 200, r.text
    d = r.json()

    assert d["enumerated"] is True, "the oracle should have enumerated this instance"
    assert d["is_optimal"] is True, f"solver {d['solver_cost_micro_usd']} vs optimum {d['optimal_cost_micro_usd']}"
    assert d["optimal_cost_micro_usd"] == d["solver_cost_micro_usd"]
    assert d["nodes_explored"] > 0
    assert "exact optimum" in d["badge"], d["badge"]
