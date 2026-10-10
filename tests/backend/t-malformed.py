import requests


def test_malformed_json_uses_the_same_error_envelope_as_every_other_failure():
    """A client should never have to parse prose to learn what went wrong."""
    r = requests.post(
        "VAR_{url}/api/solve",
        data="{not json",
        headers={"Content-Type": "application/json"},
        timeout=60,
    )
    assert r.status_code == 400, f"expected 400, got {r.status_code}: {r.text}"
    body = r.json()
    assert "error" in body, body
    assert body["error"]["code"], "errors carry a stable code"
    assert body["error"]["message"]
