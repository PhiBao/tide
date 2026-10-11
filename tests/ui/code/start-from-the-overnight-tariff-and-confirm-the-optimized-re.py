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
        
        # -> Click the 'Re-solve' button to run the optimiser for the selected tariff and refresh the schedule.
        # @@ts-step {"i":2,"type":"action","action":"click","selector":"xpath=/html/body/main/div/button","desc":"Click 'Re-solve button'","input":null,"field":"10"}
        # Re-solve button
        elem = page.get_by_role("button", name="Re-solve")
        await elem.click(timeout=10000)
        
        # --> Assertions to verify final state
        
        # --> An optimized household schedule is displayed (Overnight component is present).
        await page.get_by_role("row", name="Overnight 17 kWh $").nth(0).scroll_into_view_if_needed()
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: The schedule component row is visible on the page.
        await expect(page.get_by_role("row", name="Overnight 17 kWh $").nth(0)).to_be_visible(timeout=15000), "The schedule component row is visible on the page."
        
        # --> The outcome area indicates the optimizer proved the schedule optimal.
        await page.get_by_role("cell", name="$0.77").nth(0).scroll_into_view_if_needed()
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: A scheduled amount is shown in the results, indicating the optimizer produced a computed cost.
        await expect(page.get_by_role("cell", name="$0.77").nth(0)).to_be_visible(timeout=15000), "A scheduled amount is shown in the results, indicating the optimizer produced a computed cost."
        await asyncio.sleep(5)

    finally:
        if context:
            await context.close()
        if browser:
            await browser.close()
        if pw:
            await pw.stop()

asyncio.run(run_test())
    