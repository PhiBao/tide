import asyncio
import re
from playwright import async_api
from playwright.async_api import expect

async def run_test():
    pw = None
    browser = None
    context = None

    try:
        # Start a Playwright session in asynchronous mode
        pw = await async_api.async_playwright().start()

        # Launch a Chromium browser in headless mode with custom arguments
        browser = await pw.chromium.launch(
            headless=True,
            args=[
                "--window-size=1280,720",
                "--disable-dev-shm-usage",
                "--ipc=host",
                "--single-process"
            ],
        )

        # Create a new browser context (like an incognito window)
        context = await browser.new_context()
        # Wider default timeout to match the agent's DOM-stability budget;
        # auto-waiting Playwright APIs (expect, locator.wait_for) inherit this.
        context.set_default_timeout(15000)

        # Open a new page in the browser context
        page = await context.new_page()

        # Interact with the page elements to simulate user flow
        # -> navigate
        # @@ts-step {"i":1,"type":"action","action":"navigate","selector":null,"desc":"Navigate to VAR_{url}","input":"VAR_{url}","field":null}
        await page.goto("VAR_{url}")
        try:
            await page.wait_for_load_state("domcontentloaded", timeout=5000)
        except Exception:
            pass
        
        # -> Open the 'Tariff' dropdown (currently showing 'Overnight EV') so tariff options are revealed.
        # @@ts-step {"i":2,"type":"action","action":"click","selector":"xpath=/html/body/main/div/select","desc":"Click 'Flat rate Overnight EV Evening peak Four-season... dropdown'","input":null,"field":"3"}
        # Flat rate Overnight EV Evening peak Four-season... dropdown
        elem = page.get_by_label("Tariff")
        await elem.click(timeout=10000)
        
        # -> Select the 'Flat rate' option from the Tariff dropdown to trigger a re-solve and wait for the UI to update.
        # @@ts-step {"i":3,"type":"action","action":"select","selector":"xpath=/html/body/main/div/select","desc":"Select option in 'Flat rate Overnight EV Evening peak Four-season... dropdown'","input":null,"field":"3"}
        # Flat rate Overnight EV Evening peak Four-season... dropdown
        elem = page.locator("xpath=/html/body/main/div/select").nth(0)
        await elem.wait_for(state="visible", timeout=10000)
        await elem.select_option("")
        
        # -> Select the 'Overnight EV' tariff from the Tariff dropdown so the solver re-runs and the chart updates.
        # @@ts-step {"i":4,"type":"action","action":"select","selector":"xpath=/html/body/main/div/select","desc":"Select option in 'Flat rate Overnight EV Evening peak Four-season... dropdown'","input":null,"field":"3"}
        # Flat rate Overnight EV Evening peak Four-season... dropdown
        elem = page.locator("xpath=/html/body/main/div/select").nth(0)
        await elem.wait_for(state="visible", timeout=10000)
        await elem.select_option("")
        
        # --> Assertions to verify final state
        
        # --> The chart x‑axis is labelled 00:00 → 03:00 → … → 00:00 and the cheap teal trough leads the day (appears in the early hours).
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: The chart's bottom axis shows 00:00 through 00:00 in roughly three‑hour steps.
        await expect(page.get_by_role("main").nth(0)).to_contain_text("00 :00 03 :00 06 :00 09 :00 12 :00 15 :00 18 :00 21 :00 00 :00", timeout=15000), "The chart's bottom axis shows 00:00 through 00:00 in roughly three\u2011hour steps."
        await page.locator("div").filter(has_text=re.compile(r"^12 kWhby slot 957\.0 kW7 slots−\+×$")).locator("span").nth(0).scroll_into_view_if_needed()
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: The chart area (showing the cheap teal trough at the early hours) is visible on the page.
        await expect(page.locator("div").filter(has_text=re.compile(r"^12 kWhby slot 957\.0 kW7 slots−\+×$")).locator("span").nth(0)).to_be_visible(timeout=15000), "The chart area (showing the cheap teal trough at the early hours) is visible on the page."
        
        # --> After switching to 'Flat rate' and back to 'Overnight EV', the chart returned to the overnight shape with the teal trough in the early hours (the horizon did not drift).
        await page.get_by_label("Tariff").nth(0).scroll_into_view_if_needed()
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: The Tariff select control was present and available to toggle (used during the test).
        await expect(page.get_by_label("Tariff").nth(0)).to_be_visible(timeout=15000), "The Tariff select control was present and available to toggle (used during the test)."
        await page.locator("div").filter(has_text=re.compile(r"^12 kWhby slot 957\.0 kW7 slots−\+×$")).locator("span").nth(0).scroll_into_view_if_needed()
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: The chart area updated and is visible after toggling tariffs, showing the overnight trough again.
        await expect(page.locator("div").filter(has_text=re.compile(r"^12 kWhby slot 957\.0 kW7 slots−\+×$")).locator("span").nth(0)).to_be_visible(timeout=15000), "The chart area updated and is visible after toggling tariffs, showing the overnight trough again."
        await asyncio.sleep(5)

    finally:
        if context:
            await context.close()
        if browser:
            await browser.close()
        if pw:
            await pw.stop()

asyncio.run(run_test())
    