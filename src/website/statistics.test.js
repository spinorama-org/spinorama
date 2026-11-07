// -*- coding: utf-8 -*-
/*eslint no-undef: "error"*/

import { describe, it, expect, beforeEach, vi } from 'vitest';

// statistics.js fetches metadata at import time; stub the network layer.
vi.mock('./download.js', () => ({
    getMetadata: () => new Promise(() => {}),
}));

import { newPlot } from './statistics.js';

function makeDiv(id) {
    const div = document.createElement('div');
    div.id = id;
    document.body.appendChild(div);
    return div;
}

describe('statistics SVG renderer', () => {
    beforeEach(() => {
        document.body.innerHTML = '';
    });

    it('renders an overlay histogram as SVG rects', () => {
        makeDiv('h');
        newPlot(
            'h',
            [
                { x: [1, 2, 2, 3, 3, 3, 4, 5], type: 'histogram', opacity: 0.5, marker: { color: 'blue' }, name: 'Score' },
                { x: [2, 3, 4, 4, 5, 6], type: 'histogram', opacity: 0.5, marker: { color: 'orange' }, name: 'Score w/EQ' },
            ],
            { height: 280, title: { text: 'Distribution of scores' }, xaxis: { title: 'Score', range: [0, 10] }, yaxis: { title: 'Count' } }
        );
        const svg = document.querySelector('#h svg');
        expect(svg).not.toBeNull();
        expect(svg.querySelectorAll('rect').length).toBeGreaterThan(4);
        expect(svg.textContent).toContain('Distribution of scores');
        expect(svg.textContent).toContain('Score w/EQ');
    });

    it('renders a log-x scatter with markers and a dashed line', () => {
        makeDiv('s');
        newPlot(
            's',
            [
                { x: [100, 500, 2000], y: [3, 5, 7], mode: 'markers', type: 'scatter', name: 'Other speakers', text: ['A', 'B', 'C'], marker: { size: 8, color: 'gray' } },
                { x: [100, 2000], y: [3, 7], mode: 'lines', type: 'scatter', name: 'Pareto Frontier', line: { color: 'red', dash: 'dash' } },
            ],
            { title: { text: 'Value Chart' }, xaxis: { title: 'Price', type: 'log' }, yaxis: { title: 'Score', range: [0, 10] } }
        );
        const svg = document.querySelector('#s svg');
        expect(svg.querySelectorAll('circle').length).toBe(3);
        expect(svg.querySelectorAll('polyline').length).toBe(1);
        expect(svg.textContent).toContain('100');
        expect(svg.textContent).toContain('1000');
    });

    it('renders box and violin distributions', () => {
        makeDiv('d');
        const ys = Array.from({ length: 50 }, (_, i) => i / 5);
        newPlot('d', [{ y: ys, type: 'box', name: 'Boxed', marker: { color: '#1f77b4' } }], { title: { text: 'Box' }, yaxis: { title: 'Score' } });
        let svg = document.querySelector('#d svg');
        expect(svg.querySelectorAll('rect').length).toBeGreaterThan(0);
        expect(svg.textContent).toContain('Boxed');

        makeDiv('v');
        newPlot('v', [{ y: ys, type: 'violin', name: 'Viol', marker: { color: '#ff7f0e' } }], { title: { text: 'Violin' }, yaxis: { title: 'Score' } });
        svg = document.querySelector('#v svg');
        expect(svg.querySelectorAll('polygon').length).toBe(1);
    });

    it('renders bars from zero', () => {
        makeDiv('b');
        newPlot(
            'b',
            [{ x: [1, 2, 3], y: [4, 6, 5], type: 'bar', name: 'Bars', marker: { color: '#2ca02c' } }],
            { title: { text: 'Bars' }, xaxis: { title: 'X' }, yaxis: { title: 'Y' } }
        );
        const svg = document.querySelector('#b svg');
        const rects = svg.querySelectorAll('rect');
        expect(rects.length).toBeGreaterThanOrEqual(3);
    });
});
