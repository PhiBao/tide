import { chromium } from 'playwright';
const b = await chromium.launch();
const p = await b.newPage({ viewport: { width: 1280, height: 1400 } });
await p.goto('https://tide.kiter0211.workers.dev/', { waitUntil: 'domcontentloaded', timeout: 60000 });
await p.getByTestId('saving').waitFor({ state: 'attached', timeout: 40000 });
await p.waitForTimeout(3500);

const read = async () => ({
  saving: (await p.getByTestId('saving').innerText()).trim(),
  scheduled: (await p.getByTestId('scheduled-total').innerText()).trim(),
  loads: (await p.locator('.load-name input').all()).length,
});

let s = await read();
console.log(`  start                 loads=${s.loads}  saving=${s.saving}  scheduled=${s.scheduled}`);

// 1. A re-solve with nothing changed. The app is deterministic, so this SHOULD
//    produce identical figures -- that is a feature, not a stall.
await p.getByRole('button', { name: /Re-solve/i }).click();
await p.waitForTimeout(3000);
const same = await read();
console.log(`  after Re-solve only   loads=${same.loads}  saving=${same.saving}  scheduled=${same.scheduled}   unchanged=${same.saving===s.saving && same.scheduled===s.scheduled}`);

// 2. A real change: add a load.
await p.getByRole('button', { name: /Add a load/i }).click();
await p.waitForTimeout(3500);
const added = await read();
console.log(`  after + Add a load    loads=${added.loads}  saving=${added.saving}  scheduled=${added.scheduled}   changed=${added.saving!==same.saving || added.scheduled!==same.scheduled}`);

// 3. And re-solve after a real change.
await p.getByRole('button', { name: /Re-solve/i }).click();
await p.waitForTimeout(3000);
const again = await read();
console.log(`  after Re-solve again  loads=${again.loads}  saving=${again.saving}  scheduled=${again.scheduled}`);
await b.close();
