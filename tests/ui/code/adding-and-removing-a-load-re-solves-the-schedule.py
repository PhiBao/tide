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
        
        # -> Click the '+ Add a load' button to add a new load row to the list.
        # @@ts-step {"i":2,"type":"action","action":"click","selector":"xpath=/html/body/main/div[3]/button","desc":"Click '+ Add a load button'","input":null,"field":"11"}
        # + Add a load button
        elem = page.get_by_role("button", name="+ Add a load")
        await elem.click(timeout=10000)
        
        # -> Click the '×' (remove) button for 'Load 4' to delete the newly added load and trigger recompute.
        # @@ts-step {"i":3,"type":"action","action":"click","selector":"xpath=/html/body/main/div[2]/div[4]/div[4]/button[3]","desc":"Click 'remove Load 4 button'","input":null,"field":"492"}
        # remove Load 4 button
        elem = page.get_by_test_id("remove-load-4")
        await elem.click(timeout=10000)
        
        # --> Assertions to verify final state
        
        # --> After removing the added load, the Loads list contains only the original three rows: EV charger, Dishwasher, and Water heater.
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: EV charger row is present with its label.
        await expect(page.get_by_role("textbox", name="EV charger label").nth(0)).to_have_value("EV charger", timeout=15000), "EV charger row is present with its label."
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: Dishwasher row is present with its label.
        await expect(page.get_by_role("textbox", name="Dishwasher label").nth(0)).to_have_value("Dishwasher", timeout=15000), "Dishwasher row is present with its label."
        
        # --> After removal the Outcome component rows are visible again, showing the schedule details (Overnight and Midday components).
        await page.get_by_role("row", name="Overnight 17 kWh $").nth(0).scroll_into_view_if_needed()
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: Overnight component row is visible in the Outcome table.
        await expect(page.get_by_role("row", name="Overnight 17 kWh $").nth(0)).to_be_visible(timeout=15000), "Overnight component row is visible in the Outcome table."
        await page.get_by_role("row", name="Midday 15 kWh $").nth(0).scroll_into_view_if_needed()
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: Midday component row is visible in the Outcome table.
        await expect(page.get_by_role("row", name="Midday 15 kWh $").nth(0)).to_be_visible(timeout=15000), "Midday component row is visible in the Outcome table."
        await asyncio.sleep(5)

    finally:
        if context:
            await context.close()
        if browser:
            await browser.close()
        if pw:
            await pw.stop()

asyncio.run(run_test())
    