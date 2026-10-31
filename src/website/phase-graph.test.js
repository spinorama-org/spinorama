import { beforeEach, expect, it, vi } from 'vitest';

vi.mock('plotly.js-dist-min', () => ({
    default: { newPlot: vi.fn().mockResolvedValue(), react: vi.fn().mockResolvedValue() },
}));

import Plotly from 'plotly.js-dist-min';
import { displayGraph } from './graph.js';

beforeEach(() => {
    localStorage.clear();
    document.body.innerHTML = '<div id="graph"></div>';
    vi.clearAllMocks();
    const graph = document.getElementById('graph');
    Object.defineProperty(graph, 'offsetWidth', { value: 800 });
    Object.defineProperty(graph, 'offsetHeight', { value: 500 });
});

it('redraws the on-axis phase when its plot checkbox changes', async () => {
    const spec = {
        data: [
            { name: 'On Axis', type: 'scatter', x: [100, 1000, 2000], y: [1, 2, 3] },
            { name: 'Phase (deg)', type: 'scatter', yaxis: 'y2',
                meta: { spinorama_phase: { freq: [100, 1000, 2000],
                    measured: [-170, -190, -350], corrected: [10, 20, 30], delay_seconds: 0.001 } } },
        ],
        layout: { title: { text: 'On Axis' },
            xaxis: { type: 'log', range: [1.3, 4.3], title: { text: 'Hz' } },
            yaxis: { range: [-40, 10], title: { text: 'SPL' } },
            yaxis2: { range: [-180, 360], overlaying: 'y', side: 'right' } },
    };
    await displayGraph('On Axis', 'On Axis.json', 'graph', spec, true, 1);
    const box = document.getElementById('graph').previousElementSibling;
    const delay = box.querySelector('[data-phase-option="removeDelay"]');
    expect(delay.checked).toBe(true);
    delay.checked = false;
    delay.dispatchEvent(new Event('change', { bubbles: true }));
    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(Plotly.react).toHaveBeenCalled();
    const data = Plotly.react.mock.calls.at(-1)[1];
    expect(data.find((trace) => trace.meta?.spinorama_phase).y).toEqual([-170, null, 170, 10]);
});


it('synchronizes group delay legend clicks with the inline checkbox', async () => {
    const graph = document.getElementById('graph');
    const handlers = {};
    graph.on = (event, handler) => { handlers[event] = handler; };
    Plotly.newPlot.mockImplementationOnce(async (target, options) => { target.data = options.data; });
    const spec = {
        data: [{ name: 'Phase (deg)', type: 'scatter', yaxis: 'y2',
            meta: { spinorama_phase: { freq: [100, 1000],
                measured: [-36, -360], corrected: [0, 0], delay_seconds: 0.001 } } }],
        layout: { title: { text: 'On Axis' },
            xaxis: { type: 'log', range: [1.3, 4.3] }, yaxis: { range: [-40, 10] } },
    };
    await displayGraph('On Axis', 'On Axis.json', 'graph', spec, true, 1);
    const curveNumber = graph.data.findIndex(trace => trace.meta?.spinorama_group_delay);
    expect(curveNumber).toBeGreaterThanOrEqual(0);
    expect(handlers.plotly_legendclick({ curveNumber })).toBe(false);
    expect(graph.previousElementSibling.querySelector('[data-phase-option="groupDelay"]').checked).toBe(false);
    await new Promise(resolve => setTimeout(resolve, 40));
    expect(Plotly.react.mock.calls.at(-1)[1].find(trace => trace.meta?.spinorama_group_delay).visible).toBe('legendonly');
});


it('aligns reference/EQ plot rectangles and reserves space for the group-delay legend', async () => {
    const makeSpec = title => ({
        data: [
            { name: 'On Axis', type: 'scatter', x: [100, 1000], y: [0, 1] },
            { name: 'Phase (deg)', type: 'scatter', yaxis: 'y2',
                meta: { spinorama_phase: { freq: [100, 1000], measured: [-36, -360],
                    corrected: [0, 0], delay_seconds: 0.001 } } },
        ],
        layout: { title: { text: title },
            xaxis: { type: 'log', range: [1.3, 4.3] }, yaxis: { range: [-40, 10] },
            yaxis2: { overlaying: 'y', side: 'right' } },
    });
    for (const width of [400, 800]) {
        const ref = makeSpec('On Axis for Philips Fidelio SoundSphere DS9800W measured by SpeakerLab');
        const eq = makeSpec('On Axis for Philips Fidelio SoundSphere DS9800W measured by SpeakerLab (eq gain -2.8dB)');
        document.body.innerHTML = '<div id="ref"></div><div id="eq"></div>';
        for (const id of ['ref', 'eq']) {
            Object.defineProperty(document.getElementById(id), 'offsetWidth', { value: width });
            Object.defineProperty(document.getElementById(id), 'offsetHeight', { value: 500 });
        }
        await displayGraph('On Axis', 'ref.json', 'ref', ref, false, 1, eq);
        await displayGraph('On Axis', 'eq.json', 'eq', eq, false, 1, ref);
        const a = Plotly.newPlot.mock.calls.at(-2)[1];
        const b = Plotly.newPlot.mock.calls.at(-1)[1];
        expect(a.layout.margin).toEqual(b.layout.margin);
        expect(a.layout.height).toBe(b.layout.height);
        for (const options of [a, b]) {
            expect(options.layout.margin.t).toBeLessThan(100);
            expect(options.layout.title.yanchor).toBe('top');
            expect(options.layout.title.yref).toBe('container');
            expect(options.data.filter(t => t.meta?.spinorama_group_delay)).toHaveLength(1);
        }
    }
});
