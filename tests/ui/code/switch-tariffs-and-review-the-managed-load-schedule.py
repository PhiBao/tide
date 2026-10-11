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
        
        # -> Open the 'Tariff' dropdown to reveal tariff options (e.g., Flat rate, Overnight EV, Evening peak, Four-season peak).
        # @@ts-step {"i":2,"type":"action","action":"click","selector":"xpath=/html/body/main/div/select","desc":"Click 'Flat rate Overnight EV Evening peak Four-season... dropdown'","input":null,"field":"3"}
        # Flat rate Overnight EV Evening peak Four-season... dropdown
        elem = page.get_by_label("Tariff")
        await elem.click(timeout=10000)
        
        # -> Select the 'Flat rate' tariff from the Tariff dropdown.
        # @@ts-step {"i":3,"type":"action","action":"select","selector":"xpath=/html/body/main/div/select","desc":"Select option in 'Flat rate Overnight EV Evening peak Four-season... dropdown'","input":null,"field":"3"}
        # Flat rate Overnight EV Evening peak Four-season... dropdown
        elem = page.locator("xpath=/html/body/main/div/select").nth(0)
        await elem.wait_for(state="visible", timeout=10000)
        await elem.select_option("")
        
        # -> Click the 'Re-solve' button to re-run the optimisation with the selected 'Flat rate' tariff.
        # @@ts-step {"i":4,"type":"action","action":"click","selector":"xpath=/html/body/main/div/button","desc":"Click 'Re-solve button'","input":null,"field":"11"}
        # Re-solve button
        elem = page.get_by_role("button", name="Re-solve")
        await elem.click(timeout=10000)
        
        # --> Assertions to verify final state
        
        # --> Managed load inventory is displayed: EV charger, Dishwasher, and Water heater are present.
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: EV charger load input is present and labelled "EV charger".
        await expect(page.get_by_role("textbox", name="EV charger label").nth(0)).to_have_value("EV charger", timeout=15000), "EV charger load input is present and labelled \"EV charger\"."
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: Dishwasher load input is present and labelled "Dishwasher".
        await expect(page.get_by_role("textbox", name="Dishwasher label").nth(0)).to_have_value("Dishwasher", timeout=15000), "Dishwasher load input is present and labelled \"Dishwasher\"."
        
        # --> Optimization outcome updated after selecting the Flat rate tariff and re-solving.
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: Scheduled component amount is $4.25 after re-solving with Flat rate.
        await expect(page.locator("xpath=/html/body/main/div[6]/div[1]/table/tbody/tr/td[3]").nth(0)).to_have_text("$4.25", timeout=15000), "Scheduled component amount is $4.25 after re-solving with Flat rate."
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: "As you run it now" component amount is $3.75 after re-solving with Flat rate.
        await expect(page.locator("xpath=/html/body/main/div[6]/div[2]/table/tbody/tr/td[3]").nth(0)).to_have_text("$3.75", timeout=15000), "\"As you run it now\" component amount is $3.75 after re-solving with Flat rate."
        await asyncio.sleep(5)

    finally:
        if context:
            await context.close()
        if browser:
            await browser.close()
        if pw:
            await pw.stop()

asyncio.run(run_test())
    