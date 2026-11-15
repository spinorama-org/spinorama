// @vitest-environment jsdom
import { expect, it, vi } from 'vitest';
vi.mock('./download.js', () => ({ getMetadata: () => new Promise(() => {}) }));
import { contextFragment } from './scores.js';

it('renders contributor metadata as text and rejects executable review URLs', () => {
    const payload = '<img src=x onerror="window.injected=true">';
    const fragment = contextFragment({
        brand: payload, model: payload, price: payload,
        reviews: { reviews: [
            { url: 'javascript:alert(1)', originShort: payload, originLong: payload },
            { url: 'https://example.org/review', originShort: 'Review', originLong: 'Review' },
        ] },
        allMeasurements: [{ estimates: { ref_3dB: payload, ref_band: payload },
            scores: { pref_score: payload }, scoresEq: {}, isDefault: true }],
    }, 1);
    const container = document.createElement('div');
    container.append(fragment);
    expect(container.querySelector('img')).toBeNull();
    expect(container.textContent).toContain(payload);
    const links = container.querySelectorAll('a');
    expect(links[0].hasAttribute('href')).toBe(false);
    expect(links[1].href).toBe('https://example.org/review');
});
