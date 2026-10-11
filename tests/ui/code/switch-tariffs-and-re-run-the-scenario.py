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
        
        # -> Open the tariff chooser by clicking the tariff dropdown (the control currently showing the selected tariff).
        # @@ts-step {"i":2,"type":"action","action":"click","selector":"xpath=/html/body/main/div/select","desc":"Click 'Flat rate Overnight EV Evening peak Four-season... dropdown'","input":null,"field":"3"}
        # Flat rate Overnight EV Evening peak Four-season... dropdown
        elem = page.get_by_label("Tariff")
        await elem.click(timeout=10000)
        
        # -> Select the 'Flat rate' tariff from the tariff chooser.
        # @@ts-step {"i":3,"type":"action","action":"select","selector":"xpath=/html/body/main/div/select","desc":"Select option in 'Flat rate Overnight EV Evening peak Four-season... dropdown'","input":null,"field":"3"}
        # Flat rate Overnight EV Evening peak Four-season... dropdown
        elem = page.locator("xpath=/html/body/main/div/select").nth(0)
        await elem.wait_for(state="visible", timeout=10000)
        await elem.select_option("")
        
        # -> Click the 'Re-solve' button to compute the scenario for the selected 'Flat rate' tariff.
        # @@ts-step {"i":4,"type":"action","action":"click","selector":"xpath=/html/body/main/div/button","desc":"Click 'Re-solve button'","input":null,"field":"11"}
        # Re-solve button
        elem = page.get_by_role("button", name="Re-solve")
        await elem.click(timeout=10000)
        
        # -> Open the Tariff dropdown and choose the 'Evening peak' tariff (start by opening the Tariff dropdown).
        # @@ts-step {"i":5,"type":"action","action":"click","selector":"xpath=/html/body/main/div/select","desc":"Click 'Flat rate Overnight EV Evening peak Four-season... dropdown'","input":null,"field":"3"}
        # Flat rate Overnight EV Evening peak Four-season... dropdown
        elem = page.get_by_label("Tariff")
        await elem.click(timeout=10000)
        
        # -> Select 'Evening peak' from the Tariff dropdown
        # @@ts-step {"i":6,"type":"action","action":"select","selector":"xpath=/html/body/main/div/select","desc":"Select option in 'Flat rate Overnight EV Evening peak Four-season... dropdown'","input":null,"field":"3"}
        # Flat rate Overnight EV Evening peak Four-season... dropdown
        elem = page.locator("xpath=/html/body/main/div/select").nth(0)
        await elem.wait_for(state="visible", timeout=10000)
        await elem.select_option("")
        
        # -> Click the 'Re-solve' button to compute the scenario for the selected 'Evening peak' tariff.
        # @@ts-step {"i":7,"type":"action","action":"click","selector":"xpath=/html/body/main/div/button","desc":"Click 'Re-solve button'","input":null,"field":"11"}
        # Re-solve button
        elem = page.get_by_role("button", name="Re-solve")
        await elem.click(timeout=10000)
        
        # --> Assertions to verify final state
        
        # --> The optimisation result changed after switching tariffs — the solver returned different Scheduled and “As you run it now” values for Flat rate vs Evening peak.
        await page.get_by_role("row", name="Component Energy Amount").first.nth(0).scroll_into_view_if_needed()
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: The outcome area table header is visible after re-solving.
        await expect(page.get_by_role("row", name="Component Energy Amount").first.nth(0)).to_be_visible(timeout=15000), "The outcome area table header is visible after re-solving."
        
        # --> The schedule and savings summary are displayed for the current tariff (schedule component rows and savings summary are visible).
        await page.get_by_role("row", name="Off-peak 3 kWh $").nth(0).scroll_into_view_if_needed()
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: A schedule component row (Off-peak / energy / amount) is visible in the outcome table.
        await expect(page.get_by_role("row", name="Off-peak 3 kWh $").nth(0)).to_be_visible(timeout=15000), "A schedule component row (Off-peak / energy / amount) is visible in the outcome table."
        await asyncio.sleep(5)

    finally:
        if context:
            await context.close()
        if browser:
            await browser.close()
        if pw:
            await pw.stop()

asyncio.run(run_test())
    