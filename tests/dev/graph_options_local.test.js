import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve, extname } from 'node:path';
import { chromium } from 'playwright';
import { expect, test } from 'vitest';

test.each(['grid', 'standalone'])('%s coalesces rapid changes, retries 503, and displays only matching controls', async type => {
    const root = resolve('dist');
    const speakerPath = type === 'grid' ? '/speakers/Topping%20MA4/ASR/index_asr.html' :
        '/speakers/Topping%20MA4/ASR/asr/CEA2034.html';
    const html = (await readFile(resolve(root, decodeURIComponent(speakerPath.slice(1))), 'utf8'))
        .replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, '')
        .replace('</body>', '<script type="module" src="/graph-options.js"></script></body>');
    const module = await readFile('src/website/graph_options.js');
    const image = await readFile('dist/speakers/Topping MA4/ASR/asr/CEA2034.webp');
    const server = createServer(async (request, response) => {
        try {
            const path = new URL(request.url, 'http://localhost').pathname;
            if (path === speakerPath) return response.writeHead(200, { 'Content-Type': 'text/html' }).end(html);
            if (path === '/graph-options.js') return response.writeHead(200, { 'Content-Type': 'text/javascript' }).end(module);
            const file = resolve(root, '.' + decodeURIComponent(path));
            if (!file.startsWith(root + '/')) return response.writeHead(404).end();
            const body = await readFile(file);
            response.writeHead(200, { 'Content-Type': ({ '.css': 'text/css', '.webp': 'image/webp' })[extname(file)] || 'application/octet-stream' }).end(body);
        } catch { response.writeHead(404).end(); }
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const browser = await chromium.launch();
    const page = await browser.newPage({ viewport: { width: 1280, height: 1000 } });
    const requests = [];
    let releaseFirst;
    let inFlight = 0;
    let maxInFlight = 0;
    try {
        await page.addInitScript(() => localStorage.setItem('spinorama-graph-options', JSON.stringify({ smooth: '0.1667' })));
        await page.route('**/graph-render/graph.webp?*', async route => {
            inFlight++;
            maxInFlight = Math.max(maxInFlight, inFlight);
            requests.push(new URL(route.request().url()));
            const number = requests.length;
            if (number === 1) await new Promise(resolve => { releaseFirst = resolve; });
            if (number <= 2) await route.fulfill({ status: 503, headers: { 'Retry-After': '1' }, body: 'renderer busy' });
            else await route.fulfill({ status: 200, contentType: 'image/webp', body: image });
            inFlight--;
        });
        await page.goto(`http://127.0.0.1:${server.address().port}${speakerPath}`);
        await page.locator(type === 'grid' ? '.graph-grid img[src$="/CEA2034.webp"]' : '.graph img').first().evaluate(image => { image.dataset.testGraph = 'reference'; });
        const graph = page.locator('[data-test-graph="reference"]');
        await graph.scrollIntoViewIfNeeded();
        await expect.poll(() => requests.length).toBe(1);
        const trend = page.locator('#graph-options [name="trend"]');
        await trend.uncheck();
        await page.locator('#graph-options [name="zones"]').uncheck();
        releaseFirst();
        await graph.scrollIntoViewIfNeeded();
        await expect.poll(() => graph.getAttribute('data-render-url')).toContain('trend=0');
        await expect.poll(() => graph.evaluate(image => image.complete && image.naturalWidth > 0 && getComputedStyle(image).visibility === 'visible')).toBe(true);
        const url = new URL(await graph.getAttribute('data-render-url'), page.url());
        expect(url.searchParams.get('smooth')).toBe('0.1667');
        expect(url.searchParams.get('zones')).toBe('0');
        expect(requests[2].href).toBe(requests[1].href);
        expect(maxInFlight).toBe(1);
        expect(await trend.isChecked()).toBe(false);
        await mkdir('output/playwright', { recursive: true });
        await page.screenshot({ path: `output/playwright/graph-options-${type}.png` });
    } finally {
        releaseFirst?.();
        await browser.close();
        await new Promise(resolve => server.close(resolve));
    }
});
