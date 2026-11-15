// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { initGraphOptions } from './graph_options.js';

const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };
const picture = () => ({ ok: true, status: 200, blob: async () => new Blob(['image']) });
function change(name, checked) {
    const field = document.querySelector(`[name="${name}"]`);
    field.checked = checked;
    field.dispatchEvent(new Event('change', { bubbles: true }));
}
beforeEach(() => {
    localStorage.clear();
    document.body.innerHTML = `
        <form id="graph-options">
            <input type="checkbox" name="trend" checked>
            <input type="checkbox" name="zones" checked>
            <input type="checkbox" name="annotations" checked>
        </form>
        <div class="graph-grid">
            <img src="/speakers/Example/Lab/test/CEA2034.webp">
            <img src="/speakers/Example/Lab/test_eq/CEA2034.webp">
            <img src="/speakers/Example/Lab/test/SPL%20Horizontal%20Contour.webp">
        </div>`;
    vi.stubGlobal('fetch', vi.fn(async () => picture()));
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:picture');
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
    // jsdom does not decode images; model the browser's successful load event.
    vi.spyOn(HTMLImageElement.prototype, 'src', 'set').mockImplementation(function(value) {
        this.setAttribute('src', value);
        queueMicrotask(() => this.dispatchEvent(new Event('load')));
    });
});
afterEach(() => {
    document.body.innerHTML = ''; localStorage.clear();
    vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals();
});

it('serializes a speaker grid, coalesces rapid changes, and preserves contour images', async () => {
    let finish;
    fetch.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    initGraphOptions();
    change('annotations', false);
    await flush();
    change('trend', false);
    change('zones', false);
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(document.querySelector('img').style.visibility).toBe('hidden');
    finish(picture());
    await flush();
    const [reference, eq, contour] = document.querySelectorAll('img');
    expect(fetch).toHaveBeenCalledTimes(3); // stale request, latest reference, latest EQ
    for (const image of [reference, eq]) {
        const url = new URL(image.dataset.renderUrl, location.href);
        expect(url.searchParams.get('trend')).toBe('0');
        expect(url.searchParams.get('zones')).toBe('0');
        expect(image.style.visibility).toBe('');
    }
    expect(new URL(eq.dataset.renderUrl, location.href).searchParams.get('version')).toBe('test_eq');
    expect(contour.getAttribute('src')).toContain('Contour.webp');
    change('annotations', true); change('trend', true); change('zones', true);
    await flush();
    expect(reference.getAttribute('src')).toBe('/speakers/Example/Lab/test/CEA2034.webp');
});

it('honors Retry-After on 503 and does not display a default graph on permanent failure', async () => {
    vi.useFakeTimers();
    fetch.mockResolvedValueOnce({ status: 503, headers: new Headers({ 'Retry-After': '2' }) });
    localStorage.setItem('spinorama-graph-options', JSON.stringify({ trend: false }));
    initGraphOptions();
    await flush();
    expect(fetch).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1999);
    expect(fetch).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    await flush();
    expect(document.querySelector('img').style.visibility).toBe('');
    fetch.mockResolvedValue({ status: 404, ok: false });
    change('zones', false);
    await flush();
    expect(document.querySelector('img').style.visibility).toBe('hidden');
    expect(document.querySelector('[role="status"]').textContent).toContain('could not be loaded');
    expect(document.querySelector('[name="zones"]').checked).toBe(false);
});

it('defers hidden speaker graphs until they enter the viewport', async () => {
    let notify;
    vi.stubGlobal('IntersectionObserver', class {
        constructor(callback) { notify = callback; }
        observe() {}
    });
    initGraphOptions();
    const [reference, eq] = document.querySelectorAll('img');
    change('annotations', false);
    await flush();
    expect(fetch).not.toHaveBeenCalled();
    notify([{ target: reference, isIntersecting: true }]);
    await flush();
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(eq.getAttribute('src')).toContain('CEA2034.webp');
    notify([{ target: eq, isIntersecting: true }]);
    await flush();
    expect(fetch).toHaveBeenCalledTimes(2);
});


it('reveals the matching static image when options reset during an outstanding fetch', async () => {
    let finish;
    fetch.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    initGraphOptions();
    change('annotations', false);
    await flush();
    const image = document.querySelector('img');
    change('annotations', true);
    finish(picture());
    await flush();
    expect(image.getAttribute('src')).toBe('/speakers/Example/Lab/test/CEA2034.webp');
    expect(image.style.visibility).toBe('');
    expect(document.querySelector('[role="status"]').textContent).toBe('');
    expect(fetch).toHaveBeenCalledTimes(1);
});


it('retains the last successful picture when a response cannot be decoded', async () => {
    vi.stubGlobal('Image', class {
        set src(value) { queueMicrotask(() => this.onerror()); }
    });
    initGraphOptions();
    change('annotations', false);
    await flush();
    const image = document.querySelector('img');
    expect(document.querySelector('[role="status"]').textContent).toContain('could not be loaded');
    expect(image.getAttribute('src')).toBe('/speakers/Example/Lab/test/CEA2034.webp');
    change('annotations', true);
    await flush();
    expect(image.style.visibility).toBe('');
    expect(URL.revokeObjectURL).toHaveBeenCalled();
});
