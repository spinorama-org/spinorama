// -*- coding: utf-8 -*-
// A library to display spinorama charts
//
// Copyright (C) 2020-2025 Pierre Aubert pierre(at)spinorama(dot)org
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

/*eslint no-undef: "error"*/

// Display options for standalone graph pages. Non-default options are
// rendered on the fly by the API (/graph-render/graph.webp) and
// cached; the default view stays the pre-rendered static picture.

const DYNAMIC_MEASUREMENTS = new Set([
    'CEA2034',
    'CEA2034 Normalized',
    'On Axis',
    'Estimated In-Room Response',
    'Estimated In-Room Response Normalized',
    'Early Reflections',
    'Horizontal Reflections',
    'Vertical Reflections',
    'Horizontal Directivity',
    'Vertical Directivity',
    'SPL Horizontal',
    'SPL Vertical',
    'SPL Horizontal Normalized',
    'SPL Vertical Normalized',
]);

const STORAGE_KEY = 'spinorama-graph-options';

const DEFAULTS = { trend: true, zones: true, grid: true, legend: true, smooth: '0', splrange: '' };

function loadOptions() {
    try {
        return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(STORAGE_KEY) || '{}') };
    } catch {
        return { ...DEFAULTS };
    }
}

export function parseGraphPath(pathname) {
    const segments = pathname.split('/').filter(Boolean).map(decodeURIComponent);
    // /speakers/<speaker>/<origin>/<version>/<graph>.html
    if (segments.length !== 5 || segments[0] !== 'speakers' || !segments[4].endsWith('.html')) {
        return null;
    }
    return {
        speaker: segments[1],
        origin: segments[2],
        version: segments[3],
        measurement: segments[4].slice(0, -'.html'.length),
    };
}

export function buildGraphUrl(info, options, width) {
    const params = new URLSearchParams({
        measurement: info.measurement,
        speaker: info.speaker,
        origin: info.origin,
        version: info.version,
        w: String(width),
        trend: options.trend ? '1' : '0',
        zones: options.zones ? '1' : '0',
        grid: options.grid ? '1' : '0',
        legend: options.legend ? '1' : '0',
        smooth: options.smooth,
    });
    if (options.splrange) {
        params.set('splrange', options.splrange);
    }
    return `/graph-render/graph.webp?${params.toString()}`;
}

export function isDefaultOptions(options) {
    return (
        options.trend === DEFAULTS.trend &&
        options.zones === DEFAULTS.zones &&
        options.grid === DEFAULTS.grid &&
        options.legend === DEFAULTS.legend &&
        options.smooth === DEFAULTS.smooth &&
        options.splrange === DEFAULTS.splrange
    );
}

export function initGraphOptions() {
    const img = document.querySelector('.graph img');
    const bar = document.getElementById('graph-options');
    if (!img || !bar) {
        return;
    }
    const info = parseGraphPath(window.location.pathname);
    if (!info || !DYNAMIC_MEASUREMENTS.has(info.measurement)) {
        bar.remove();
        return;
    }
    const staticSrc = img.getAttribute('src');
    const options = loadOptions();
    const fields = {
        trend: bar.querySelector('[name="trend"]'),
        zones: bar.querySelector('[name="zones"]'),
        grid: bar.querySelector('[name="grid"]'),
        legend: bar.querySelector('[name="legend"]'),
        smooth: bar.querySelector('[name="smooth"]'),
        splrange: bar.querySelector('[name="splrange"]'),
    };
    for (const key of ['trend', 'zones', 'grid', 'legend']) {
        if (fields[key]) fields[key].checked = options[key];
    }
    if (fields.smooth) fields.smooth.value = options.smooth;
    if (fields.splrange) fields.splrange.value = options.splrange;

    const apply = () => {
        for (const key of ['trend', 'zones', 'grid', 'legend']) {
            options[key] = fields[key] ? fields[key].checked : DEFAULTS[key];
        }
        options.smooth = fields.smooth ? fields.smooth.value : '0';
        options.splrange = fields.splrange ? fields.splrange.value : '';
        try {
            localStorage.setItem(STORAGE_KEY, JSON.stringify(options));
        } catch {
            // storage unavailable: options last for this page only
        }
        if (isDefaultOptions(options)) {
            img.src = staticSrc;
            return;
        }
        const url = buildGraphUrl(info, options, 1400);
        img.onerror = () => {
            img.onerror = null;
            img.src = staticSrc;
        };
        img.src = url;
    };

    bar.addEventListener('change', apply);
    bar.querySelector('[data-action="reset"]')?.addEventListener('click', () => {
        for (const key of ['trend', 'zones', 'grid', 'legend']) {
            if (fields[key]) fields[key].checked = DEFAULTS[key];
        }
        if (fields.smooth) fields.smooth.value = DEFAULTS.smooth;
        if (fields.splrange) fields.splrange.value = DEFAULTS.splrange;
        apply();
    });

    if (!isDefaultOptions(options)) {
        apply();
    }
}

if (typeof document !== 'undefined') {
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', initGraphOptions);
    } else {
        initGraphOptions();
    }
}
