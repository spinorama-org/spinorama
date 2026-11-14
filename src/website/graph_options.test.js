// -*- coding: utf-8 -*-
/*eslint no-undef: "error"*/

import { describe, it, expect } from 'vitest';
import { parseGraphPath, buildGraphUrl, isDefaultOptions } from './graph_options.js';

describe('graph options', () => {
    it('parses speaker graph paths', () => {
        expect(parseGraphPath('/speakers/KEF%20R3/ASR/asr/CEA2034.html')).toEqual({
            speaker: 'KEF R3',
            origin: 'ASR',
            version: 'asr',
            measurement: 'CEA2034',
        });
        expect(parseGraphPath('/index.html')).toBeNull();
        expect(parseGraphPath('/speakers/KEF R3/ASR/index_asr.html')).toBeNull();
    });

    it('builds dynamic graph urls', () => {
        const url = buildGraphUrl(
            { speaker: 'KEF R3', origin: 'ASR', version: 'asr', measurement: 'On Axis' },
            { trend: false, zones: true, grid: true, legend: false, smooth: '0.0833', splrange: '30' },
            1400
        );
        expect(url).toContain('/graph-render/graph.webp?');
        expect(url).toContain('measurement=On+Axis');
        expect(url).toContain('trend=0');
        expect(url).toContain('legend=0');
        expect(url).toContain('smooth=0.0833');
        expect(url).toContain('splrange=30');
    });

    it('detects default options', () => {
        expect(isDefaultOptions({ trend: true, zones: true, grid: true, legend: true, smooth: '0', splrange: '' })).toBe(true);
        expect(isDefaultOptions({ trend: false, zones: true, grid: true, legend: true, smooth: '0', splrange: '' })).toBe(false);
        expect(isDefaultOptions({ trend: true, zones: true, annotations: false, grid: true, legend: true, smooth: '0', splrange: '' })).toBe(false);
    });

    it('requests annotation visibility from the server', () => {
        const url = buildGraphUrl(
            { speaker: 'Example', origin: 'Lab', version: 'test', measurement: 'CEA2034' },
            { trend: true, zones: true, annotations: false, grid: true, legend: true, smooth: '0' },
            1400
        );
        expect(new URLSearchParams(url.split('?')[1]).get('annotations')).toBe('0');
    });
});
