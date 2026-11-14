// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { initGraphOptions } from './graph_options.js';

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
});
afterEach(() => { document.body.innerHTML = ''; localStorage.clear(); vi.unstubAllGlobals(); });

it('toggles server-rendered images for reference and EQ while preserving contour images', () => {
    initGraphOptions();
    const [reference, eq, contour] = document.querySelectorAll('img');
    const staticReference = reference.getAttribute('src');
    const contourSource = contour.getAttribute('src');
    const annotations = document.querySelector('[name="annotations"]');
    annotations.checked = false;
    annotations.dispatchEvent(new Event('change', { bubbles: true }));
    for (const image of [reference, eq]) {
        expect(new URL(image.src).pathname).toBe('/graph-render/graph.webp');
        expect(new URL(image.src).searchParams.get('annotations')).toBe('0');
    }
    expect(new URL(eq.src).searchParams.get('version')).toBe('test_eq');
    expect(contour.getAttribute('src')).toBe(contourSource);
    annotations.checked = true;
    annotations.dispatchEvent(new Event('change', { bubbles: true }));
    expect(reference.getAttribute('src')).toBe(staticReference);
});

it('restores saved options and falls back to the static image after a server error', () => {
    localStorage.setItem('spinorama-graph-options', JSON.stringify({ trend: false, zones: true, annotations: false }));
    initGraphOptions();
    const image = document.querySelector('img');
    expect(document.querySelector('[name="trend"]').checked).toBe(false);
    expect(new URL(image.src).searchParams.get('trend')).toBe('0');
    image.dispatchEvent(new Event('error'));
    expect(image.getAttribute('src')).toBe('/speakers/Example/Lab/test/CEA2034.webp');
});

it('defers hidden speaker graphs until they enter the viewport', () => {
    let notify;
    vi.stubGlobal('IntersectionObserver', class {
        constructor(callback) { notify = callback; }
        observe() {}
    });
    initGraphOptions();
    const [reference, eq] = document.querySelectorAll('img');
    const eqSource = eq.getAttribute('src');
    const annotations = document.querySelector('[name="annotations"]');
    annotations.checked = false;
    annotations.dispatchEvent(new Event('change', { bubbles: true }));
    expect(reference.getAttribute('src')).toContain('CEA2034.webp');
    notify([{ target: reference, isIntersecting: true }]);
    expect(new URL(reference.src).searchParams.get('annotations')).toBe('0');
    expect(eq.getAttribute('src')).toBe(eqSource);
    notify([{ target: eq, isIntersecting: true }]);
    expect(new URL(eq.src).searchParams.get('annotations')).toBe('0');
});
