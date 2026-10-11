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
        
        # -> Open the 'Tariff' dropdown so a different tariff option can be selected (e.g., choose 'Flat rate').
        # @@ts-step {"i":2,"type":"action","action":"click","selector":"xpath=/html/body/main/div/select","desc":"Click 'Flat rate Overnight EV Evening peak Four-season... dropdown'","input":null,"field":"3"}
        # Flat rate Overnight EV Evening peak Four-season... dropdown
        elem = page.get_by_label("Tariff")
        await elem.click(timeout=10000)
        
        # -> Select the 'Flat rate' tariff from the Tariff dropdown so the scheduler recomputes for that tariff.
        # @@ts-step {"i":3,"type":"action","action":"select","selector":"xpath=/html/body/main/div/select","desc":"Select option in 'Flat rate Overnight EV Evening peak Four-season... dropdown'","input":null,"field":"3"}
        # Flat rate Overnight EV Evening peak Four-season... dropdown
        elem = page.locator("xpath=/html/body/main/div/select").nth(0)
        await elem.wait_for(state="visible", timeout=10000)
        await elem.select_option("")
        
        # --> Assertions to verify final state
        
        # --> Outcome panel shows the updated scheduled cost ($4.25) and the comparison value 'As you run it now' ($3.75) after selecting the Flat rate tariff.
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: Scheduled cost equals $4.25.
        await expect(page.locator("xpath=/html/body/main/div[6]/div[1]/table/tbody/tr/td[3]").nth(0)).to_have_text("$4.25", timeout=15000), "Scheduled cost equals $4.25."
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: 'As you run it now' amount equals $3.75.
        await expect(page.locator("xpath=/html/body/main/div[6]/div[2]/table/tbody/tr/td[3]").nth(0)).to_have_text("$3.75", timeout=15000), "'As you run it now' amount equals $3.75."
        
        # --> The schedule remains marked '✓ proved optimal'.
        await page.locator("div").filter(has_text=re.compile(r"^12 kWhby slot 957\.0 kW7 slots−\+×$")).locator("span").nth(0).scroll_into_view_if_needed()
        # Assert-outcome: not verified — the export-time check could not be evaluated
        # Assert: Schedule is marked '✓ proved optimal'.
        await expect(page.locator("div").filter(has_text=re.compile(r"^12 kWhby slot 957\.0 kW7 slots−\+×$")).locator("span").nth(0)).to_be_visible(timeout=15000), "Schedule is marked '\u2713 proved optimal'."
        await asyncio.sleep(5)

    finally:
        if context:
            await context.close()
        if browser:
            await browser.close()
        if pw:
            await pw.stop()

asyncio.run(run_test())
    