import requests


def test_every_bundled_tariff_records_where_its_numbers_came_from():
    """A fabricated tariff would make every downstream figure unverifiable."""
    r = requests.get("VAR_{url}/api/tariffs", timeout=60)
    assert r.status_code == 200, r.text
    tariffs = r.json()["tariffs"]
    assert len(tariffs) >= 4, f"expected the bundled set, got {len(tariffs)}"
    for t in tariffs:
        assert t["source_url"], f"{t['id']} has no source_url"
        assert t["source_retrieved"], f"{t['id']} has no retrieval date"
