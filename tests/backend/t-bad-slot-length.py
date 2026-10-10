import requests

H = {"Content-Type": "application/json"}


def test_a_slot_length_that_does_not_divide_an_hour_is_rejected():
    """A 7-minute grid would truncate energy per slot, so it must not be allowed."""
    r = requests.post(
        "VAR_{url}/api/prices",
        json={"tariff_id": "flat",
              "grid": {"start_epoch_minutes": 0, "slot_minutes": 7, "slots": 10}},
        headers=H, timeout=60,
    )
    assert r.status_code == 422, f"expected 422, got {r.status_code}: {r.text}"
    body = r.json()
    assert body["error"]["code"] == "invalid_input"
    assert "slot length" in body["error"]["message"]
