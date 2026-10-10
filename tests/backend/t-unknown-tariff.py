import requests

H = {"Content-Type": "application/json"}


def test_an_unknown_tariff_is_a_404_with_a_stable_code():
    r = requests.get("VAR_{url}/api/tariffs/does-not-exist", headers=H, timeout=60)
    assert r.status_code == 404, r.text
    body = r.json()
    assert body["error"]["code"] == "not_found"
    assert "does-not-exist" in body["error"]["message"]
