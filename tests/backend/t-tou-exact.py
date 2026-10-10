import requests

H = {"Content-Type": "application/json"}


def test_the_trough_and_the_peak_bill_exactly_as_the_tariff_says():
    """Same energy, different hour, materially different bill.

    This is the claim the whole product rests on, so it is asserted to the
    micro-dollar rather than compared with a greater-than.
    """
    now = 20735 * 1440 + 15 * 60 + 3
    horizon = requests.post(
        "VAR_{url}/api/horizon",
        json={"tariff_id": "overnight-ev", "slots": 96, "slot_minutes": 15, "now_minutes": now},
        headers=H, timeout=60,
    ).json()
    grid = {"start_epoch_minutes": horizon["start_epoch_minutes"], "slot_minutes": 15, "slots": 96}

    def bill_with_energy_in(slot):
        usage = [0] * 96
        usage[slot] = 5000  # 5 kWh
        return requests.post(
            "VAR_{url}/api/bills",
            json={"tariff_id": "overnight-ev", "grid": grid,
                  "import_wh": usage, "export_wh": [0] * 96},
            headers=H, timeout=60,
        ).json()

    cheap = bill_with_energy_in(0)     # overnight, $0.045/kWh
    dear = bill_with_energy_in(40)     # midday, $0.240/kWh

    # 5 kWh * $0.045 = $0.225 = 225_000 micro-USD.
    assert cheap["total_micro_usd"] == 225_000, cheap
    # 5 kWh * $0.240 = $1.20 = 1_200_000 micro-USD.
    assert dear["total_micro_usd"] == 1_200_000, dear
    assert cheap["total_micro_usd"] < dear["total_micro_usd"] / 4
