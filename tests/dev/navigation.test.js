import { mkdir } from 'node:fs/promises';
import { chromium } from 'playwright';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, test } from 'vitest';

const site = new URL(process.env.SPINORAMA_TEST_URL || 'https://dev.spinorama.org');
const cards = '[data-num="0"] .cell';
let browser;
let context;
let page;
let failures;
let modules;

beforeAll(async () => {
    browser = await chromium.launch();
});

afterAll(async () => {
    await browser?.close();
});

beforeEach(async () => {
    failures = [];
    modules = new Set();
    context = await browser.newContext({
        // Exercise the same nginx request policy as a regular Chrome visitor.
        userAgent: `Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${browser.version()} Safari/537.36`,
        viewport: { width: 1280, height: 900 },
    });
    page = await context.newPage();
    page.setDefaultTimeout(15_000);
    page.on('response', (response) => {
        const url = new URL(response.url());
        if (url.origin !== site.origin) return;
        const retryableRender = response.status() === 503 && url.pathname === '/graph-render/graph.webp';
        if (response.status() >= 400 && !retryableRender) failures.push(`${response.status()} ${response.url()}`);
        if (url.pathname.startsWith('/js/')) modules.add(url.pathname);
    });
    page.on('requestfailed', (request) => {
        const failure = request.failure()?.errorText;
        if (new URL(request.url()).origin === site.origin && failure !== 'net::ERR_ABORTED') {
            failures.push(`${failure} ${request.url()}`);
        }
    });
    page.on('pageerror', (error) => failures.push(error.stack || error.message));
    page.on('console', (message) => {
        if (message.type() === 'error' && !message.text().includes('server responded with a status of 503')) failures.push(message.text());
    });
});

afterEach(async ({ task }) => {
    try {
        if (page && (task.result?.state === 'fail' || failures.length)) {
            await mkdir('output/playwright/dev', { recursive: true });
            const name = task.name.replace(/[^a-z0-9]+/gi, '-');
            await page.screenshot({ path: `output/playwright/dev/${name}.png`, fullPage: true });
        }
        expect(failures, 'Deployed resources and browser scripts must load without errors').toEqual([]);
    } finally {
        await context?.close();
    }
});

async function openHome() {
    const response = await page.goto(site.href);
    expect(response.status()).toBe(200);
    await expect.poll(() => page.locator('#pagination').innerText()).toMatch(/\d+ speakers/);
    const total = Number((await page.locator('#pagination').innerText()).match(/(\d+) speakers/)[1]);
    const perPage = await page.locator(cards).count();
    expect(perPage).toBeGreaterThan(0);
    return { total, pages: Math.ceil(total / perPage) };
}

describe(`deployed website at ${site.origin}`, () => {
    test('trend toggles replace the picture with different server-rendered content', async () => {
        const metadataResponse = await context.request.get(new URL('/json/metadata.json', site).href);
        expect(metadataResponse.status()).toBe(200);
        const metadata = await metadataResponse.json();
        // Select from the live catalogue; speakers and default versions change.
        const candidate = Object.entries(metadata).find(([, speaker]) => {
            const measurement = speaker.measurements?.[speaker.default_measurement];
            return measurement?.quality === 'high' && measurement.format === 'klippel';
        });
        expect(candidate, 'A full Klippel spin is available').toBeTruthy();
        const [name, speaker] = candidate;
        const version = speaker.default_measurement;
        const origin = speaker.measurements[version].origin.replace(/^Vendors-/, '');
        const path = [name, origin, version, 'CEA2034.html'].map(encodeURIComponent).join('/');
        const response = await page.goto(new URL(`/speakers/${path}`, site).href);
        expect(response.status()).toBe(200);
        await page.waitForFunction(() => document.querySelector('.graph img')?.complete);
        const trend = page.locator('#graph-options [name="trend"]');
        const zones = page.locator('#graph-options [name="zones"]');
        const rendered = (value) => page.waitForResponse((picture) => {
            const url = new URL(picture.url());
            return picture.status() === 200 && url.pathname === '/graph-render/graph.webp'
                && url.searchParams.get('trend') === value
                && url.searchParams.get('zones') === '0';
        });
        const initialRender = rendered('1');
        await zones.uncheck();
        expect((await initialRender).status()).toBe(200);
        const withoutTrends = rendered('0');
        await trend.uncheck();
        const off = await withoutTrends;
        expect(off.status()).toBe(200);
        const offBody = await off.body();
        const withTrends = rendered('1');
        await trend.check();
        const on = await withTrends;
        expect(on.status()).toBe(200);
        expect(await on.body()).not.toEqual(offBody);
        await expect.poll(() => page.locator('.graph img').evaluate((image) => (
            image.complete && image.naturalWidth > 0
        ))).toBe(true);
        expect(new URL(await page.locator('.graph img').getAttribute('data-render-url'), site).searchParams.get('trend')).toBe('1');
    });

    test('loads the homepage and its misc and pagination modules', async () => {
        await openHome();
        expect(await page.locator(cards).count()).toBeGreaterThan(0);
        for (const module of ['misc', 'pagination', 'meta']) {
            expect(
                [...modules].some((path) => path.startsWith(`/js/${module}-`)),
                `${module} module loaded`
            ).toBe(true);
        }
        expect(await page.locator('#pagination').innerText()).toMatch(/\d+ speakers/);
    });

    test('Next changes the selected page and speaker cards across available pages', async () => {
        const { pages } = await openHome();
        const pagesToVisit = Math.min(5, pages);
        const firstSpeakers = new Set();
        for (let number = 1; number <= pagesToVisit; number++) {
            if (pages > 1) {
                await expect.poll(() => page.locator('.pagination-link.is-current').textContent()).toBe(String(number));
            }
            const first = await page.locator(cards).first().getAttribute('id');
            expect(first).toBeTruthy();
            expect(firstSpeakers.has(first), `page ${number} has different speakers`).toBe(false);
            firstSpeakers.add(first);
            if (number < pagesToVisit) await page.getByRole('link', { name: 'Next', exact: true }).click();
        }
        expect(new URL(page.url()).searchParams.get('page') || '1').toBe(String(pagesToVisit));
    });

    test('search filters speakers and resets pagination', async () => {
        const { total, pages } = await openHome();
        if (pages > 1) {
            await page.getByRole('link', { name: 'Next', exact: true }).click();
            await expect.poll(() => page.locator('.pagination-link.is-current').textContent()).toBe('2');
        }
        const speaker = page.locator(cards).first();
        const id = await speaker.getAttribute('id');
        const brand = (await speaker.locator('.card-brand').innerText()).trim();
        const model = (await speaker.locator('.card-model').innerText()).trim();
        const query = `${brand} ${model}`;
        expect(brand).toBeTruthy();
        expect(model).toBeTruthy();
        const search = page.getByRole('textbox', { name: 'brand or model ...' });
        await search.fill(query);
        await search.press('Enter');
        await page.waitForURL((url) => url.searchParams.get('search') === query);
        expect(new URL(page.url()).searchParams.get('page')).toBe('1');
        await expect
            .poll(async () => {
                const ids = await page.locator(cards).evaluateAll((elements) => elements.map((element) => element.id));
                return ids.includes(id);
            })
            .toBe(true);
        const matched = Number((await page.locator('#pagination').innerText()).match(/(\d+) speakers/)[1]);
        expect(matched).toBeGreaterThan(0);
        expect(matched).toBeLessThanOrEqual(total);
    });
});
