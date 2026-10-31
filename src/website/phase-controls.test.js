import { beforeEach, describe, expect, it } from 'vitest';
import { applyConfig, applyPhaseOptions, createDefaultConfig, initGlobalConfigPanel, initPhaseControls, loadConfigFromStorage } from './plot-config.js';
import { setGraph } from './plot.js';

function phaseTrace(measured, corrected, delay = 0.001) {
    return {
        name: 'Phase (deg)', type: 'scatter', yaxis: 'y2',
        meta: { spinorama_phase: { freq: [100, 1000, 2000], measured, corrected, delay_seconds: delay } },
    };
}

describe('On Axis phase controls', () => {
    beforeEach(() => {
        localStorage.clear();
        document.body.innerHTML = '<div id="global-config-panel"></div><div id="plot"></div>';
    });

    it('applies both options to every phase trace, without changing SPL', () => {
        const spl = { name: 'On Axis', x: [100, 1000, 2000], y: [1, 2, 3] };
        const options = { data: [
            spl, phaseTrace([-170, -190, -350], [10, 20, 30]),
            phaseTrace([-160, -200, -340], [40, 50, 60]),
        ], layout: {} };
        applyPhaseOptions(options, { wrap: true, removeDelay: false });
        expect(options.data[1].y).toEqual([-170, null, 170, 10]);
        expect(options.data[2].y).toEqual([-160, null, 160, 20]);
        expect(options.data[0].y).toEqual([1, 2, 3]);
        expect(options.layout.yaxis2.range).toEqual([-180, 540]);
        expect(options.layout.yaxis2.showgrid).toBe(false);
        expect(options.layout.yaxis2.zeroline).toBe(false);
        expect(options.layout.yaxis2.minor.showgrid).toBe(false);
        applyPhaseOptions(options, { wrap: false, removeDelay: true });
        expect(options.data[1].y).toEqual([10, 20, 30]);
        expect(options.data[2].y).toEqual([40, 50, 60]);
        expect(options.layout.yaxis2.autorange).toBe(false);
        expect(options.layout.yaxis2.range).toEqual([7.5, 117.5]);
    });

    it('keeps plot controls and the options menu synchronized and persisted', () => {
        initGlobalConfigPanel(createDefaultConfig());
        const target = document.getElementById('plot');
        initPhaseControls(target, { data: [phaseTrace([1, 2, 3], [4, 5, 6])] });
        const box = target.previousElementSibling;
        const plotWrap = box.querySelector('[data-phase-option="wrap"]');
        const plotDelay = box.querySelector('[data-phase-option="removeDelay"]');
        const menuWrap = document.querySelector('[name="config-phase-wrap"]');
        const menuDelay = document.querySelector('[name="config-phase-remove-delay"]');
        expect(plotWrap.checked && plotDelay.checked && menuWrap.checked && menuDelay.checked).toBe(true);
        plotWrap.checked = false;
        plotWrap.dispatchEvent(new Event('change', { bubbles: true }));
        expect(menuWrap.checked).toBe(false);
        expect(loadConfigFromStorage().phase.wrap).toBe(false);
        menuDelay.checked = false;
        menuDelay.dispatchEvent(new Event('change', { bubbles: true }));
        expect(plotDelay.checked).toBe(false);
        expect(loadConfigFromStorage().phase.removeDelay).toBe(false);
        initPhaseControls(target, { data: [] });
        expect(box.style.display).toBe('none');
    });

    it('preserves phase metadata in comparison graphs and handles missing phase', () => {
        const graph = (data) => ({
            data, layout: {
                title: { text: 'On Axis' },
                xaxis: { type: 'log', range: [1.3, 4.3], title: { text: 'Hz' } },
                yaxis: { range: [-40, 10], title: { text: 'SPL' } },
            },
        });
        const withPhase = graph([{ name: 'On Axis', x: [100, 1000], y: [0, 1] },
            phaseTrace([-170, -190, -350], [10, 20, 30])]);
        const withoutPhase = graph([{ name: 'On Axis', x: [100, 1000], y: [0, 1] }]);
        const result = setGraph('On Axis', ['A', 'B'], [withoutPhase, withPhase], 1200, 800)[0];
        applyPhaseOptions(result, { wrap: true, removeDelay: false });
        expect(result.data.find((trace) => trace.meta?.spinorama_phase).y).toEqual([-170, null, 170, 10]);
        expect(result.layout.yaxis2.range).toEqual([-180, 540]);
    });
});


it('draws vertical dotted wrap markers in the phase color and removes them when unwrapped', () => {
    const options = { data: [phaseTrace([-170, -190, -350], [10, 20, 30])], layout: {} };
    const config = createDefaultConfig();
    config.phase.removeDelay = false;
    config.colors.palette = 'vibrant';
    options.data[0].type = 'scatter';
    applyConfig(options, config);
    const marker = options.data.find((trace) => trace.meta?.spinorama_phase_jump);
    expect(marker.x).toEqual([1000, 1000, null]);
    expect(marker.y).toEqual([-180, 180, null]);
    expect(marker.line.dash).toBe('dot');
    expect(marker.line.color).toBe(options.data[0].line.color);
    expect(marker.showlegend).toBe(false);
    applyConfig(options, config);
    expect(options.data.filter((trace) => trace.meta?.spinorama_phase_jump)).toHaveLength(1);
    config.phase.wrap = false;
    applyConfig(options, config);
    expect(options.data.filter((trace) => trace.meta?.spinorama_phase_jump)).toHaveLength(0);
});


it('persists an absolute SPL span and preserves phase and contour scales', () => {
    localStorage.clear();
    document.body.innerHTML = '<div id="global-config-panel"></div>';
    const config = loadConfigFromStorage();
    expect(config.layout.splRange).toBe('50 dB');
    initGlobalConfigPanel(config);
    const select = document.querySelector('[name="config-spl-range"]');
    expect([...select.options].map(option => option.value)).toEqual(['20 dB', '30 dB', '40 dB', '50 dB']);
    const options = { data: [], _graphType: { isGraph: true }, layout: {
        yaxis: { range: [-40, 10] }, yaxis2: { range: [-180, 360] },
    } };
    applyConfig(options, config);
    expect(options.layout.yaxis.range).toEqual([-40, 10]);
    expect(options.layout.yaxis2.range).toEqual([-180, 360]);
    select.value = '50 dB';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    applyConfig(options, loadConfigFromStorage());
    expect(options.layout.yaxis.range).toEqual([-40, 10]);
    expect(loadConfigFromStorage().layout.splRange).toBe('50 dB');
    for (const span of [20, 30, 40, 50]) {
        select.value = `${span} dB`;
        select.dispatchEvent(new Event('change', { bubbles: true }));
        applyConfig(options, loadConfigFromStorage());
        expect(options.layout.yaxis.range).toEqual([10 - span, 10]);
        expect(options.layout.yaxis2.range).toEqual([-180, 360]);
    }
    const contour = { data: [], _graphType: { isSurface: true }, layout: {
        yaxis: { range: [-180, 180] },
    } };
    applyConfig(contour, config);
    expect(contour.layout.yaxis.range).toEqual([-180, 180]);
    const spin = { data: [], _graphType: { isGraph: true, isSpin: true }, layout: {
        yaxis: { range: [-45, 5] }, yaxis2: { range: [-5, 45] },
    } };
    applyConfig(spin, createDefaultConfig());
    expect(spin.layout.yaxis.range).toEqual([-45, 5]);
    expect(spin.layout.yaxis2.range).toEqual([-5, 45]);
});


it('derives group delay from unwrapped phase on nonuniform frequency intervals', () => {
    const trace = phaseTrace([-36, -360, -720], [0, 0, 0]);
    const options = { data: [trace], layout: {} };
    applyPhaseOptions(options, { removeDelay: false, wrap: true });
    const gd = () => options.data.find(t => t.meta?.spinorama_group_delay);
    expect(gd().y).toEqual([1, 1]);
    expect(gd().x).toEqual([550, 1500]);
    expect(gd().yaxis).toBe('y3');
    expect(options.layout.yaxis3.visible).toBe(false);
    applyPhaseOptions(options, { removeDelay: false, wrap: false, groupDelay: false, show: false });
    expect(gd().y).toEqual([1, 1]);
    expect(gd().visible).toBe('legendonly');
    expect(trace.visible).toBe('legendonly');
    expect(options.data.filter(t => t.meta?.spinorama_group_delay)).toHaveLength(1);
    applyPhaseOptions(options, { removeDelay: true });
    expect(gd().y.every(value => value === 0)).toBe(true);
});

it('does not differentiate across missing phase or duplicate frequencies', () => {
    const trace = phaseTrace([0, null, -720], [0, null, -720]);
    const options = { data: [trace], layout: {} };
    applyPhaseOptions(options);
    expect(options.data.find(t => t.meta?.spinorama_group_delay).y).toEqual([null, null]);
    trace.meta.spinorama_phase.freq = [100, 100, 2000];
    trace.meta.spinorama_phase.corrected = [0, 0, -720];
    applyPhaseOptions(options);
    expect(options.data.find(t => t.meta?.spinorama_group_delay).y[0]).toBeNull();
});

it('synchronizes group delay visibility between the inline controls and menu', () => {
    localStorage.clear();
    document.body.innerHTML = '<div id="global-config-panel"></div><div id="gd-plot"></div>';
    initGlobalConfigPanel(createDefaultConfig());
    const target = document.getElementById('gd-plot');
    initPhaseControls(target, { data: [phaseTrace([0, 0, 0], [0, 0, 0])] });
    const inline = target.previousElementSibling.querySelector('[data-phase-option="groupDelay"]');
    const menu = document.querySelector('[name="config-phase-group-delay"]');
    expect(inline.checked && menu.checked).toBe(true);
    inline.click();
    expect(menu.checked).toBe(false);
    expect(loadConfigFromStorage().phase.groupDelay).toBe(false);
    menu.click();
    expect(inline.checked).toBe(true);
});


it('places phase and group delay in the bottom half with a truthful delay legend', () => {
    const trace = phaseTrace([-1440, -14400, -28800], [0, 0, 0]);
    const options = { data: [trace], layout: {} };
    applyPhaseOptions(options, { removeDelay: false });
    const gd = options.data.find(t => t.meta?.spinorama_group_delay);
    expect(gd.name).toBe('Group Delay (0–40 ms)');
    expect(options.layout.yaxis3.range).toEqual([0, 80]);
    expect(options.layout.yaxis3.visible).toBe(false);
    expect(options.layout.yaxis2.range).toEqual([-180, 540]);
    const fraction = (value, range) => (value - range[0]) / (range[1] - range[0]);
    expect(fraction(40, options.layout.yaxis3.range)).toBe(0.5);
    expect(fraction(180, options.layout.yaxis2.range)).toBe(0.5);
    applyPhaseOptions(options, { removeDelay: false, wrap: false });
    expect(Math.max(...trace.y.map(value => fraction(value, options.layout.yaxis2.range)))).toBeLessThanOrEqual(0.5);
    applyPhaseOptions(options, { removeDelay: true });
    expect(options.data.find(t => t.meta?.spinorama_group_delay).name).toBe('Group Delay (0–1 ms)');
});
