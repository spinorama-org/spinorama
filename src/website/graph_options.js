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

const DEFAULTS = { trend: true, zones: true, annotations: true, grid: true, legend: true, smooth: '0', splrange: '' };

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
        annotations: (options.annotations ?? true) ? '1' : '0',
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
        (options.annotations ?? true) === DEFAULTS.annotations &&
        options.grid === DEFAULTS.grid &&
        options.legend === DEFAULTS.legend &&
        options.smooth === DEFAULTS.smooth &&
        options.splrange === DEFAULTS.splrange
    );
}

export function initGraphOptions() {
    const bar = document.getElementById('graph-options');
    if (!bar) return;
    const targets = [...document.querySelectorAll('.graph img, .graph-grid img')].map(img => {
        const staticSrc = img.getAttribute('src');
        const path = new URL(staticSrc, window.location.href).pathname.replace(/\.webp$/, '.html');
        return { img, staticSrc, info: parseGraphPath(path) };
    }).filter(target => target.info && DYNAMIC_MEASUREMENTS.has(target.info.measurement));
    if (!targets.length) { bar.remove(); return; }
    const options = loadOptions();
    const fields = {
        trend: bar.querySelector('[name="trend"]'),
        zones: bar.querySelector('[name="zones"]'),
        annotations: bar.querySelector('[name="annotations"]'),
        grid: bar.querySelector('[name="grid"]'),
        legend: bar.querySelector('[name="legend"]'),
        smooth: bar.querySelector('[name="smooth"]'),
        splrange: bar.querySelector('[name="splrange"]'),
    };
    for (const key of ['trend', 'zones', 'annotations', 'grid', 'legend']) {
        if (fields[key]) fields[key].checked = options[key];
    }
    if (fields.smooth) fields.smooth.value = options.smooth;
    if (fields.splrange) fields.splrange.value = options.splrange;

    let observer;
    const visible = new Set();
    let rendering = false;
    for (const target of targets) {
        target.wanted = target.staticSrc;
        target.applied = target.staticSrc;
        target.status = document.createElement('span');
        target.status.setAttribute('role', 'status');
        target.img.after(target.status);
    }
    const renderNext = async () => {
        if (rendering) return;
        rendering = true;
        try {
            let target;
            while ((target = targets.find(t => t.wanted !== t.applied && !t.failed &&
                (!observer || visible.has(t.img))))) {
                const { img, wanted: source, staticSrc } = target;
                target.loading = source;
                let picture = source;
                try {
                    if (source !== staticSrc) {
                        for (let attempt = 0; ; attempt++) {
                            const response = await fetch(source);
                            if (source !== target.wanted) break;
                            if (response.status === 503 && attempt < 4) {
                                const retry = response.headers.get('Retry-After');
                                const seconds = retry && /^\d+$/.test(retry) ? Number(retry) :
                                    retry ? (Date.parse(retry) - Date.now()) / 1000 : 2 ** attempt;
                                await new Promise(resolve => setTimeout(resolve,
                                    Math.max(0, Number.isFinite(seconds) ? seconds : 1) * 1000));
                                if (source !== target.wanted) break;
                                continue;
                            }
                            if (!response.ok) throw new Error(`HTTP ${response.status}`);
                            picture = URL.createObjectURL(await response.blob());
                            break;
                        }
                    }
                    if (source !== target.wanted) {
                        if (picture.startsWith('blob:')) URL.revokeObjectURL(picture);
                        continue;
                    }
                    const probe = new Image();
                    await new Promise((resolve, reject) => {
                        probe.onload = resolve;
                        probe.onerror = () => reject(new Error('Invalid picture'));
                        probe.src = picture;
                    });
                    if (source !== target.wanted) {
                        if (picture.startsWith('blob:')) URL.revokeObjectURL(picture);
                        continue;
                    }
                    img.src = picture;
                    if (target.objectUrl) URL.revokeObjectURL(target.objectUrl);
                    target.objectUrl = picture.startsWith('blob:') ? picture : null;
                    target.applied = source;
                    img.dataset.renderUrl = source;
                    if (source === target.wanted) {
                        img.style.visibility = '';
                        img.removeAttribute('aria-busy');
                        target.status.textContent = '';
                    }
                } catch {
                    if (picture.startsWith('blob:') && picture !== target.objectUrl) URL.revokeObjectURL(picture);
                    if (source === target.wanted) {
                        target.failed = true;
                        img.removeAttribute('aria-busy');
                        target.status.textContent = 'Graph could not be loaded. Change an option to retry.';
                    }
                } finally {
                    target.loading = null;
                    if (target.wanted === target.applied) {
                        img.style.visibility = '';
                        img.removeAttribute('aria-busy');
                        target.status.textContent = '';
                    }
                }
            }
        } finally {
            rendering = false;
        }
    };
    const updateImage = (target) => {
        const { img, staticSrc, info } = target;
        const effective = info.measurement.startsWith('CEA2034') ? options : { ...options, annotations: true };
        const source = isDefaultOptions(effective) ? staticSrc : buildGraphUrl(info, effective, 1400);
        target.wanted = source;
        target.failed = false;
        const pending = source !== target.applied || Boolean(target.loading);
        img.style.visibility = pending ? 'hidden' : '';
        img.setAttribute('aria-busy', String(pending));
        target.status.textContent = pending ? 'Loading graph…' : '';
        // Start after this change has updated every image's desired state.
        queueMicrotask(renderNext);
    };
    const apply = () => {
        for (const key of ['trend', 'zones', 'annotations', 'grid', 'legend']) {
            if (fields[key]) options[key] = fields[key].checked;
        }
        if (fields.smooth) options.smooth = fields.smooth.value;
        if (fields.splrange) options.splrange = fields.splrange.value;
        try {
            localStorage.setItem(STORAGE_KEY, JSON.stringify(options));
        } catch {
            // storage unavailable: options last for this page only
        }
        for (const target of targets) {
            updateImage(target);
        }
    };

    if (typeof IntersectionObserver !== 'undefined' && targets.length > 1) {
        const byImage = new Map(targets.map(target => [target.img, target]));
        observer = new IntersectionObserver(entries => {
            for (const entry of entries) {
                if (entry.isIntersecting) {
                    visible.add(entry.target);
                    updateImage(byImage.get(entry.target));
                } else {
                    visible.delete(entry.target);
                }
            }
        });
        for (const { img } of targets) observer.observe(img);
    }

    bar.addEventListener('change', apply);
    bar.querySelector('[data-action="reset"]')?.addEventListener('click', () => {
        for (const key of ['trend', 'zones', 'annotations', 'grid', 'legend']) {
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
