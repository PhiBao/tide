import json
import requests


def test_bill_recomputes_exactly_for_a_flat_rate():
    """5 kWh at $0.25/kWh must bill exactly $1.25.

    The rate engine is exact integer arithmetic, so the total is reproducible
    by hand rather than approximately right. That is what makes the line-item
    audit trail real instead of decorative.
    """
    url = "VAR_{url}/api/bills"
    headers = {"Content-Type": "application/json"}
    payload = {
        "tariff_id": "flat",
        "grid": {
            "start_epoch_minutes": 20735 * 1440,
            "slot_minutes": 15,
            "slots": 96,
        },
        "import_wh": [5000] + [0] * 95,
        "export_wh": [0] * 96,
    }

    response = requests.post(url, json=payload, headers=headers, timeout=60)
    assert response.status_code == 200, f"expected 200, got {response.status_code}: {response.text}"

    data = response.json()
    # 5 kWh * $0.25/kWh = $1.25 = 1_250_000 micro-dollars.
    expected = 1_250_000
    assert data["total_micro_usd"] == expected, (
        f"expected exactly {expected} micro-USD for 5 kWh at $0.25/kWh, "
        f"got {data['total_micro_usd']}"
    )

    lines = data["lines"]
    assert len(lines) >= 1, "a bill must have at least one line"
    energy = sum(line["energy_wh"] for line in lines)
    assert energy == 5000, f"lines must account for all 5000 Wh, got {energy}"

    # The printed lines must add up to the printed total.
    line_cents = sum(round(line["amount_micro_usd"] / 10_000) for line in lines)
    total_cents = round(data["total_micro_usd"] / 10_000)
    assert line_cents == total_cents, (
        f"bill lines must sum to the total: {line_cents} vs {total_cents} cents"
    )
