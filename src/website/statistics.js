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

import { getMetadata } from './download.js';

// Minimal SVG chart renderer covering the trace/layout subset used by
// this page (histogram, scatter markers/lines, bar, box, violin).
// It replaces the Plotly dependency: charts are static SVG with a
// <title> tooltip per point.
const SVG_NS = 'http://www.w3.org/2000/svg';

function svgEl(name, attrs = {}, text = null) {
    const node = document.createElementNS(SVG_NS, name);
    for (const [key, value] of Object.entries(attrs)) {
        node.setAttribute(key, value);
    }
    if (text !== null) node.textContent = text;
    return node;
}

function fmtTick(v) {
    if (!isFinite(v)) return '';
    const a = Math.abs(v);
    if (a >= 1000) return String(Math.round(v));
    if (a >= 100) return String(parseFloat(v.toFixed(1)));
    if (a >= 1) return String(parseFloat(v.toFixed(2)));
    return String(parseFloat(v.toFixed(3)));
}

function niceScale(min, max, count = 5) {
    if (!isFinite(min) || !isFinite(max) || min === max) {
        min = (min || 0) - 1;
        max = (max || 0) + 1;
    }
    const span = max - min;
    const step0 = span / count;
    const mag = Math.pow(10, Math.floor(Math.log10(step0)));
    const norm = step0 / mag;
    const step = (norm >= 5 ? 10 : norm >= 2 ? 5 : norm >= 1 ? 2 : 1) * mag;
    const lo = Math.floor(min / step) * step;
    const hi = Math.ceil(max / step) * step;
    const ticks = [];
    for (let v = lo; v <= hi + step * 1e-9; v += step) ticks.push(parseFloat(v.toFixed(10)));
    return { lo, hi, ticks };
}

function logScale(min, max) {
    const lo = Math.pow(10, Math.floor(Math.log10(Math.max(min, 1e-9))));
    const hi = Math.pow(10, Math.ceil(Math.log10(Math.max(max, lo * 10))));
    const ticks = [];
    for (let v = lo; v <= hi * 1.0001; v *= 10) ticks.push(v);
    return { lo, hi, ticks };
}

function extent(values, positiveOnly = false) {
    let lo = Infinity;
    let hi = -Infinity;
    for (const v of values) {
        if (typeof v !== 'number' || !isFinite(v)) continue;
        if (positiveOnly && v <= 0) continue;
        if (v < lo) lo = v;
        if (v > hi) hi = v;
    }
    return [lo, hi];
}

export function newPlot(divname, data, layout) {
    const target = document.getElementById(divname);
    if (!target) return;
    target.innerHTML = '';

    const width = 760;
    const height = layout.height || 400;
    const margin = { l: 58, r: 16, t: 36, b: 72 };
    const plotW = width - margin.l - margin.r;
    const plotH = height - margin.t - margin.b;

    const svg = svgEl('svg', {
        viewBox: `0 0 ${width} ${height}`,
        width: '100%',
        role: 'img',
        style: 'max-width:100%;height:auto;font-family:inherit;',
    });
    const gridColor = '#e5e5e5';
    const axisColor = '#999';
    const textColor = '#4a4a4a';
    const fontSize = 11;

    const title = layout.title?.text || '';
    if (title) {
        svg.appendChild(svgEl('text', { x: margin.l, y: 20, 'font-size': 14, 'font-weight': 600, fill: textColor }, title));
    }

    const kinds = new Set(data.map((t) => t.type || 'scatter'));
    const isHistogram = kinds.has('histogram');
    const isDistribution = kinds.has('box') || kinds.has('violin');

    let xTicks = [];
    let drawLegend = [];

    function drawAxes(xScale, yScale, xTickList, yTickList) {
        for (const t of yTickList) {
            const y = yScale(t);
            svg.appendChild(svgEl('line', { x1: margin.l, x2: margin.l + plotW, y1: y, y2: y, stroke: gridColor, 'stroke-width': 1 }));
            svg.appendChild(
                svgEl('text', { x: margin.l - 6, y: y + 4, 'text-anchor': 'end', 'font-size': fontSize, fill: textColor }, fmtTick(t))
            );
        }
        for (const t of xTickList) {
            const x = xScale(t);
            svg.appendChild(
                svgEl('text', { x, y: margin.t + plotH + 16, 'text-anchor': 'middle', 'font-size': fontSize, fill: textColor }, fmtTick(t))
            );
        }
        svg.appendChild(
            svgEl('line', { x1: margin.l, x2: margin.l, y1: margin.t, y2: margin.t + plotH, stroke: axisColor, 'stroke-width': 1 })
        );
        svg.appendChild(
            svgEl('line', {
                x1: margin.l,
                x2: margin.l + plotW,
                y1: margin.t + plotH,
                y2: margin.t + plotH,
                stroke: axisColor,
                'stroke-width': 1,
            })
        );
        const xTitle = typeof layout.xaxis?.title === 'string' ? layout.xaxis.title : '';
        const yTitle = typeof layout.yaxis?.title === 'string' ? layout.yaxis.title : '';
        if (xTitle) {
            svg.appendChild(
                svgEl(
                    'text',
                    { x: margin.l + plotW / 2, y: margin.t + plotH + 34, 'text-anchor': 'middle', 'font-size': 12, fill: textColor },
                    xTitle
                )
            );
        }
        if (yTitle) {
            svg.appendChild(
                svgEl(
                    'text',
                    {
                        x: 14,
                        y: margin.t + plotH / 2,
                        'text-anchor': 'middle',
                        'font-size': 12,
                        fill: textColor,
                        transform: `rotate(-90 14 ${margin.t + plotH / 2})`,
                    },
                    yTitle
                )
            );
        }
    }

    function makeScale(axisLayout, values, rangeLo, rangeHi, forceZero = false) {
        const isLog = axisLayout?.type === 'log';
        let [lo, hi] = axisLayout?.range ? [...axisLayout.range] : extent(values, isLog);
        if (forceZero && !isLog) lo = Math.min(lo, 0);
        const scaled = isLog ? logScale(lo, hi) : niceScale(axisLayout?.range ? lo : lo, axisLayout?.range ? hi : hi);
        if (axisLayout?.range && !isLog) {
            scaled.lo = lo;
            scaled.hi = hi;
            scaled.ticks = niceScale(lo, hi).ticks.filter((t) => t >= lo && t <= hi);
        }
        const domain = isLog ? [Math.log10(scaled.lo), Math.log10(scaled.hi)] : [scaled.lo, scaled.hi];
        const scale = (v) => {
            const x = isLog ? Math.log10(Math.max(v, 1e-12)) : v;
            return rangeLo + ((x - domain[0]) / (domain[1] - domain[0])) * (rangeHi - rangeLo);
        };
        return { scale, ticks: scaled.ticks, lo: scaled.lo, hi: scaled.hi };
    }

    if (isHistogram) {
        const allX = data.flatMap((t) => t.x || []);
        const [dataLo, dataHi] = layout.xaxis?.range ? [...layout.xaxis.range] : extent(allX);
        const first = data[0] || {};
        let binEdges;
        if (first.xbins?.size) {
            const start = first.xbins.start ?? dataLo;
            const end = first.xbins.end ?? dataHi;
            binEdges = [];
            for (let v = start; v < end; v += first.xbins.size) binEdges.push(v);
            binEdges.push(end);
        } else {
            const nbins = first.nbinsx || 20;
            binEdges = Array.from({ length: nbins + 1 }, (_, i) => dataLo + ((dataHi - dataLo) * i) / nbins);
        }
        const counts = data.map((trace) => {
            const bins = Array.from({ length: binEdges.length - 1 }, () => 0);
            for (const v of trace.x || []) {
                if (typeof v !== 'number' || !isFinite(v) || v < binEdges[0] || v > binEdges[binEdges.length - 1]) continue;
                let idx = binEdges.findIndex((e, i) => i > 0 && v <= e) - 1;
                if (idx < 0) idx = 0;
                bins[Math.min(idx, bins.length - 1)]++;
            }
            return bins;
        });
        const maxCount = Math.max(1, ...counts.flat());
        const xs = makeScale({ range: [binEdges[0], binEdges[binEdges.length - 1]] }, [], margin.l, margin.l + plotW);
        const ys = makeScale(null, [0, maxCount], margin.t + plotH, margin.t, true);
        drawAxes(xs.scale, ys.scale, xs.ticks, ys.ticks);
        data.forEach((trace, ti) => {
            const color = trace.marker?.color || '#1f77b4';
            counts[ti].forEach((count, bi) => {
                if (count === 0) return;
                const x0 = xs.scale(binEdges[bi]);
                const x1 = xs.scale(binEdges[bi + 1]);
                const y0 = ys.scale(count);
                const rect = svgEl('rect', {
                    x: x0 + 0.5,
                    y: y0,
                    width: Math.max(x1 - x0 - 1, 1),
                    height: margin.t + plotH - y0,
                    fill: color,
                    'fill-opacity': trace.opacity ?? 0.6,
                });
                rect.appendChild(svgEl('title', {}, `${trace.name || ''}: ${count} in [${fmtTick(binEdges[bi])}, ${fmtTick(binEdges[bi + 1])})`));
                svg.appendChild(rect);
            });
            if (trace.name) drawLegend.push({ name: trace.name, color });
        });
        xTicks = xs.ticks;
    } else if (isDistribution) {
        const allY = data.flatMap((t) => (t.y || []).filter((v) => typeof v === 'number' && isFinite(v)));
        const ys = makeScale(layout.yaxis, allY, margin.t + plotH, margin.t);
        const catW = plotW / Math.max(data.length, 1);
        const xs = (i) => margin.l + catW * (i + 0.5);
        drawAxes(() => 0, ys.scale, [], ys.ticks);
        data.forEach((trace, ti) => {
            const values = (trace.y || []).filter((v) => typeof v === 'number' && isFinite(v)).sort((a, b) => a - b);
            if (values.length === 0) return;
            const color = trace.marker?.color || '#1f77b4';
            const cx = xs(ti);
            const quantile = (q) => values[Math.min(values.length - 1, Math.floor(q * values.length))];
            if (trace.type === 'violin') {
                const mean = values.reduce((a, b) => a + b, 0) / values.length;
                const sd = Math.sqrt(values.reduce((a, b) => a + (b - mean) ** 2, 0) / values.length) || 1;
                const bw = Math.max(1.06 * sd * Math.pow(values.length, -0.2), (values[values.length - 1] - values[0]) / 50 || 1);
                const pts = [];
                let maxD = 0;
                for (let i = 0; i <= 40; i++) {
                    const v = values[0] + ((values[values.length - 1] - values[0]) * i) / 40;
                    let d = 0;
                    for (const u of values) d += Math.exp(-0.5 * ((v - u) / bw) ** 2);
                    pts.push([v, d]);
                    if (d > maxD) maxD = d;
                }
                const half = catW * 0.42;
                const left = pts.map(([v, d]) => `${cx - (d / maxD) * half},${ys.scale(v)}`).join(' ');
                const right = pts
                    .slice()
                    .reverse()
                    .map(([v, d]) => `${cx + (d / maxD) * half},${ys.scale(v)}`)
                    .join(' ');
                svg.appendChild(svgEl('polygon', { points: `${left} ${right}`, fill: color, 'fill-opacity': 0.5, stroke: color }));
            } else {
                const q1 = quantile(0.25);
                const med = quantile(0.5);
                const q3 = quantile(0.75);
                const w = catW * 0.5;
                svg.appendChild(
                    svgEl('line', { x1: cx, x2: cx, y1: ys.scale(values[0]), y2: ys.scale(values[values.length - 1]), stroke: color })
                );
                svg.appendChild(
                    svgEl('rect', {
                        x: cx - w / 2,
                        y: ys.scale(q3),
                        width: w,
                        height: Math.max(ys.scale(q1) - ys.scale(q3), 1),
                        fill: color,
                        'fill-opacity': 0.5,
                        stroke: color,
                    })
                );
                svg.appendChild(svgEl('line', { x1: cx - w / 2, x2: cx + w / 2, y1: ys.scale(med), y2: ys.scale(med), stroke: color, 'stroke-width': 2 }));
            }
            svg.appendChild(
                svgEl('text', { x: cx, y: margin.t + plotH + 16, 'text-anchor': 'middle', 'font-size': fontSize, fill: textColor }, trace.name || '')
            );
        });
    } else {
        const allX = data.flatMap((t) => t.x || []);
        const allY = data.flatMap((t) => t.y || []);
        const hasBar = kinds.has('bar');
        const xs = makeScale(layout.xaxis, allX, margin.l, margin.l + plotW);
        const ys = makeScale(layout.yaxis, allY, margin.t + plotH, margin.t, hasBar);
        drawAxes(xs.scale, ys.scale, xs.ticks, ys.ticks);
        xTicks = xs.ticks;
        for (const trace of data) {
            const color = trace.marker?.color || trace.line?.color || '#1f77b4';
            if (trace.name) drawLegend.push({ name: trace.name, color });
            if (trace.type === 'bar') {
                const sortedX = [...new Set(trace.x || [])].sort((a, b) => a - b);
                let minGap = plotW / 20;
                for (let i = 1; i < sortedX.length; i++) minGap = Math.min(minGap, Math.abs(xs.scale(sortedX[i]) - xs.scale(sortedX[i - 1])));
                const w = Math.max(minGap * 0.8, 2);
                (trace.x || []).forEach((xv, i) => {
                    const yv = trace.y?.[i];
                    if (!isFinite(xv) || !isFinite(yv)) return;
                    const y0 = ys.scale(Math.max(yv, 0));
                    const rect = svgEl('rect', {
                        x: xs.scale(xv) - w / 2,
                        y: y0,
                        width: w,
                        height: Math.max(margin.t + plotH - y0, 0),
                        fill: color,
                        'fill-opacity': trace.opacity ?? 0.8,
                    });
                    rect.appendChild(svgEl('title', {}, `${trace.text?.[i] || trace.name || ''}: ${fmtTick(xv)}, ${fmtTick(yv)}`));
                    svg.appendChild(rect);
                });
                continue;
            }
            const mode = trace.mode || 'lines';
            if (mode.includes('lines')) {
                const pts = (trace.x || [])
                    .map((xv, i) => [xv, trace.y?.[i]])
                    .filter(([xv, yv]) => isFinite(xv) && isFinite(yv) && (!layout.xaxis || layout.xaxis.type !== 'log' || xv > 0))
                    .map(([xv, yv]) => `${xs.scale(xv)},${ys.scale(yv)}`)
                    .join(' ');
                if (pts) {
                    svg.appendChild(
                        svgEl('polyline', {
                            points: pts,
                            fill: 'none',
                            stroke: trace.line?.color || color,
                            'stroke-width': trace.line?.width || 1.5,
                            'stroke-dasharray': trace.line?.dash === 'dash' ? '6 4' : trace.line?.dash === 'dot' ? '2 3' : 'none',
                        })
                    );
                }
            }
            if (mode.includes('markers')) {
                const r = (trace.marker?.size || 8) / 2;
                (trace.x || []).forEach((xv, i) => {
                    const yv = trace.y?.[i];
                    if (!isFinite(xv) || !isFinite(yv)) return;
                    if (layout.xaxis?.type === 'log' && xv <= 0) return;
                    const circle = svgEl('circle', {
                        cx: xs.scale(xv),
                        cy: ys.scale(yv),
                        r,
                        fill: color,
                        'fill-opacity': trace.opacity ?? 0.8,
                        stroke: trace.marker?.line?.color || 'none',
                        'stroke-width': trace.marker?.line?.width || 0,
                    });
                    circle.appendChild(svgEl('title', {}, `${trace.text?.[i] || trace.name || ''}: ${fmtTick(xv)}, ${fmtTick(yv)}`));
                    svg.appendChild(circle);
                });
            }
        }
    }

    if (!isDistribution && drawLegend.length > 0) {
        let lx = margin.l;
        let ly = height - 18;
        for (const entry of drawLegend) {
            const w = 18 + entry.name.length * 6.5 + 16;
            if (lx + w > width - margin.r) {
                lx = margin.l;
                ly -= 16;
            }
            svg.appendChild(svgEl('rect', { x: lx, y: ly - 9, width: 12, height: 12, fill: entry.color, 'fill-opacity': 0.8 }));
            svg.appendChild(svgEl('text', { x: lx + 16, y: ly + 2, 'font-size': fontSize, fill: textColor }, entry.name));
            lx += w;
        }
    }

    target.appendChild(svg);
    return xTicks;
}

getMetadata()
    .then((metadata) => {
        function plotScoreDistribution(scores, scoresEQ) {
            const traceScores = {
                x: scores,
                type: 'histogram',
                opacity: 0.5,
                marker: {
                    color: 'blue',
                },
                name: 'Score',
            };
            const traceScoresEQ = {
                x: scoresEQ,
                type: 'histogram',
                opacity: 0.5,
                marker: {
                    color: 'orange',
                },
                name: 'Score w/EQ',
            };
            const data = [traceScores, traceScoresEQ];
            const layout = {
                autosize: true,
                height: 280,
                title: { text: 'Distribution of scores' },
                xaxis: {
                    title: 'Score',
                    range: [0, 10],
                },
                yaxis: { title: 'Count', nticks: 5 },
                barmode: 'overlay',
                legend: {
                    orientation: 'h',
                    y: -0.2,
                },
                margin: { l: 50, r: 20, t: 40, b: 40 },
            };
            newPlot('visScoreDistribution', data, layout, { responsive: true });
        }

        function plotScoreDistributionWsub(scores, scoresEQ) {
            const traceScores = {
                x: scores,
                type: 'histogram',
                opacity: 0.5,
                marker: {
                    color: 'blue',
                },
                name: 'Score w/Sub',
            };
            const traceScoresEQ = {
                x: scoresEQ,
                type: 'histogram',
                opacity: 0.5,
                marker: {
                    color: 'orange',
                },
                name: 'Score w/Sub+w/EQ',
            };
            const data = [traceScores, traceScoresEQ];
            const layout = {
                autosize: true,
                height: 280,
                title: { text: 'Distribution of scores with a perfect subwoofer' },
                xaxis: {
                    title: 'Score',
                    range: [0, 10],
                },
                yaxis: {
                    title: 'Count',
                    nticks: 5,
                },
                barmode: 'overlay',
                legend: {
                    orientation: 'h',
                    y: -0.2,
                },
                margin: { l: 50, r: 20, t: 40, b: 40 },
            };
            newPlot('visScoreDistributionWsub', data, layout, { responsive: true });
        }

        function computeParetoFrontier(points) {
            const sorted = points.filter((p) => p.price > 0 && p.score > 0).sort((a, b) => a.price - b.price);
            const pareto = [];
            let maxScore = -Infinity;
            for (const p of sorted) {
                if (p.score > maxScore) {
                    pareto.push(p);
                    maxScore = p.score;
                }
            }
            return pareto;
        }

        function computeLinearRegression(xValues, yValues, options = {}) {
            const { logX = false, logY = false } = options;
            const n = xValues.length;
            if (n < 2) return null;

            const validPairs = [];
            for (let i = 0; i < n; i++) {
                const x = logX ? (xValues[i] > 0 ? Math.log10(xValues[i]) : NaN) : xValues[i];
                const y = logY ? (yValues[i] > 0 ? Math.log10(yValues[i]) : NaN) : yValues[i];
                if (!isNaN(x) && !isNaN(y)) {
                    validPairs.push({ x, y, origX: xValues[i] });
                }
            }

            if (validPairs.length < 2) return null;

            const validX = validPairs.map((p) => p.x);
            const validY = validPairs.map((p) => p.y);
            const origXValues = validPairs.map((p) => p.origX);

            const sumX = validX.reduce((a, b) => a + b, 0);
            const sumY = validY.reduce((a, b) => a + b, 0);
            const sumXY = validX.reduce((acc, x, i) => acc + x * validY[i], 0);
            const sumX2 = validX.reduce((acc, x) => acc + x * x, 0);

            const slope = (validX.length * sumXY - sumX * sumY) / (validX.length * sumX2 - sumX * sumX);
            const intercept = (sumY - slope * sumX) / validX.length;

            const minX = Math.min(...origXValues);
            const maxX = Math.max(...origXValues);

            let yMin, yMax;
            if (logX) {
                yMin = Math.pow(10, slope * Math.log10(minX) + intercept);
                yMax = Math.pow(10, slope * Math.log10(maxX) + intercept);
            } else {
                yMin = slope * minX + intercept;
                yMax = slope * maxX + intercept;
            }

            if (logY) {
                yMin = Math.pow(10, yMin);
                yMax = Math.pow(10, yMax);
            }

            return {
                slope,
                intercept,
                x: [minX, maxX],
                y: [yMin, yMax],
            };
        }

        function computeParetoCurveForData(xValues, yValues, options = {}) {
            const { direction = 'max' } = options;
            const points = xValues.map((x, i) => ({ x, y: yValues[i] }));
            const sorted = points
                .filter((p) => p.x !== null && p.y !== null && !isNaN(p.x) && !isNaN(p.y))
                .sort((a, b) => a.x - b.x);

            const pareto = [];
            if (direction === 'max') {
                let maxY = -Infinity;
                for (const p of sorted) {
                    if (p.y > maxY) {
                        pareto.push(p);
                        maxY = p.y;
                    }
                }
            } else {
                let minY = Infinity;
                for (const p of sorted) {
                    if (p.y < minY) {
                        pareto.push(p);
                        minY = p.y;
                    }
                }
            }
            return {
                x: pareto.map((p) => p.x),
                y: pareto.map((p) => p.y),
            };
        }

        function plotValueChart(prices, scores, names, divname) {
            const points = prices.map((p, i) => ({ price: p, score: scores[i], name: names[i] }));
            const pareto = computeParetoFrontier(points);
            const paretoSet = new Set(pareto.map((p) => p.name));

            const nonPareto = {
                x: points.filter((p) => !paretoSet.has(p.name)).map((p) => p.price),
                y: points.filter((p) => !paretoSet.has(p.name)).map((p) => p.score),
                mode: 'markers',
                type: 'scatter',
                name: 'Other speakers',
                text: points.filter((p) => !paretoSet.has(p.name)).map((p) => p.name),
                marker: {
                    size: 8,
                    color: 'rgba(100, 100, 100, 0.5)',
                },
            };

            const paretoTrace = {
                x: pareto.map((p) => p.price),
                y: pareto.map((p) => p.score),
                mode: 'markers',
                type: 'scatter',
                name: 'Best Value (Pareto)',
                text: pareto.map((p) => p.name),
                marker: {
                    size: 12,
                    color: 'red',
                    line: {
                        color: 'darkred',
                        width: 2,
                    },
                },
            };

            const paretoLine = {
                x: pareto.map((p) => p.price),
                y: pareto.map((p) => p.score),
                mode: 'lines',
                type: 'scatter',
                name: 'Pareto Frontier',
                line: {
                    color: 'red',
                    width: 2,
                    dash: 'dash',
                },
                hoverinfo: 'skip',
            };

            const data = [nonPareto, paretoTrace, paretoLine];
            const layout = {
                autosize: true,
                title: { text: 'Value Chart: Price vs Preference Score' },
                xaxis: {
                    title: 'Price (USD, each)',
                    type: 'log',
                },
                yaxis: {
                    title: 'Preference Score',
                    range: [0, 10],
                },
                legend: {
                    orientation: 'h',
                    y: -0.2,
                },
                hovermode: 'closest',
                margin: { l: 50, r: 20, t: 40, b: 40 },
            };
            newPlot(divname, data, layout, { responsive: true });
        }

        function plotParameters(name, title, param, scores, names, divname) {
            const trace = {
                x: scores,
                y: param,
                mode: 'markers',
                type: 'scatter',
                name: name,
                text: names,
            };
            const data = [trace];
            const layout = {
                autosize: true,
                title: { text: title },
                legend: {
                    orientation: 'h',
                    y: -0.2,
                },
                xaxis: {
                    title: 'Score',
                },
                yaxis: {
                    title: name,
                },
                margin: { l: 50, r: 20, t: 40, b: 40 },
            };
            newPlot(divname, data, layout, { responsive: true });
        }

        function getFilterFromURL() {
            const params = new URLSearchParams(window.location.search);
            return {
                brand: params.get('brand') || '',
                shape: params.get('shape') || '',
                power: params.get('power') || '',
                quality: params.get('quality')
                    ? params
                          .get('quality')
                          .split(',')
                          .filter((v) => v !== '')
                    : [],
                priceMin: parseFloat(params.get('priceMin')) || 0,
                priceMax: parseFloat(params.get('priceMax')) || Infinity,
                scoreMin: parseFloat(params.get('scoreMin')) || 0,
                scoreMax: parseFloat(params.get('scoreMax')) || 10,
            };
        }

        function matchesFilter(speaker, filter) {
            if (filter.brand && speaker.brand.toLowerCase() !== filter.brand.toLowerCase()) {
                return false;
            }
            if (filter.shape && speaker.shape !== filter.shape) {
                return false;
            }
            if (filter.power && speaker.type !== filter.power) {
                return false;
            }
            if (filter.quality.length > 0) {
                const msr = speaker.measurements[speaker.default_measurement];
                if (!msr || !filter.quality.includes(msr.quality?.toLowerCase())) {
                    return false;
                }
            }
            return true;
        }

        function stats() {
            const filter = getFilterFromURL();
            const scores = [];
            const scoresEQ = [];
            const scoresWsub = [];
            const scoresWsubEQ = [];
            const lfx = [];
            const nbdON = [];
            const nbdPIR = [];
            const smPIR = [];
            const names = [];
            const prices = [];
            const scoresForPrice = [];
            metadata.forEach((value) => {
                if (!matchesFilter(value, filter)) {
                    return;
                }
                if (
                    value.measurements &&
                    value.measurements[value.default_measurement] &&
                    value.measurements[value.default_measurement].pref_rating &&
                    value.measurements[value.default_measurement].pref_rating.pref_score
                ) {
                    // gather various scores
                    scores.push(value.measurements[value.default_measurement].pref_rating.pref_score);
                    scoresWsub.push(value.measurements[value.default_measurement].pref_rating.pref_score_wsub);
                    // components of the score
                    lfx.push(value.measurements[value.default_measurement].pref_rating.lfx_hz);
                    nbdON.push(value.measurements[value.default_measurement].pref_rating.nbd_on_axis);
                    nbdPIR.push(value.measurements[value.default_measurement].pref_rating.nbd_pred_in_room);
                    smPIR.push(value.measurements[value.default_measurement].pref_rating.sm_pred_in_room);
                    // price (each speaker, not pair)
                    let price = 0;
                    let hasPrice = false;
                    if (value.price && value.price !== '') {
                        const parsedPrice = parseFloat(value.price);
                        if (!isNaN(parsedPrice) && parsedPrice > 0) {
                            price = parsedPrice;
                            if (!value.amount || value.amount === 'pair') {
                                price /= 2.0;
                            }
                            hasPrice = true;
                        }
                    }
                    const score = value.measurements[value.default_measurement].pref_rating.pref_score;
                    if (
                        hasPrice &&
                        price >= filter.priceMin &&
                        price <= filter.priceMax &&
                        score >= filter.scoreMin &&
                        score <= filter.scoreMax
                    ) {
                        prices.push(price);
                        names.push(value.brand + ' ' + value.model);
                        scoresForPrice.push(score);
                    }
                }
                if (
                    value.measurements &&
                    value.measurements[value.default_measurement] &&
                    value.measurements[value.default_measurement].pref_rating_eq &&
                    value.measurements[value.default_measurement].pref_rating_eq.pref_score
                ) {
                    // gather various scores
                    scoresEQ.push(value.measurements[value.default_measurement].pref_rating_eq.pref_score);
                    scoresWsubEQ.push(value.measurements[value.default_measurement].pref_rating_eq.pref_score_wsub);
                }
            });
            // console.log('found ' + scores.length + ' scores')
            plotScoreDistribution(scores, scoresEQ);
            plotScoreDistributionWsub(scoresWsub, scoresWsubEQ);
            plotValueChart(prices, scoresForPrice, names, 'visValueChart');
            plotParameters('LFX (Hz)', 'Low Frequency eXtension (LFX) v.s. Score', lfx, scores, names, 'visDistributionLfxHz');
            plotParameters(
                'NBD ON',
                'Narrow Bandwidth On Axis (NBD ON) v.s. Score',
                nbdON,
                scores,
                names,
                'visDistributionNbdOn'
            );
            plotParameters(
                'NBD PIR',
                'Narrow Bandwidth Predicted In Room (NBD PIR) v.s. Score',
                nbdPIR,
                scores,
                names,
                'visDistributionNbdPir'
            );
            plotParameters(
                'SM PIR',
                'Smoothness Predicted In Room (SM PIR) v.s. Score',
                smPIR,
                scores,
                names,
                'visDistributionSmPir'
            );
        }

        function getFieldValue(speaker, field) {
            const msr = speaker.measurements[speaker.default_measurement];
            switch (field) {
                case 'brand':
                    return speaker.brand;
                case 'model':
                    return speaker.model;
                case 'price': {
                    if (!speaker.price || speaker.price === '') return null;
                    let price = parseFloat(speaker.price);
                    if (isNaN(price)) return null;
                    return !speaker.amount || speaker.amount === 'pair' ? price / 2 : price;
                }
                case 'type':
                    return speaker.type;
                case 'shape':
                    return speaker.shape;
                case 'score':
                    return msr?.pref_rating?.pref_score ?? null;
                case 'scoreWsub':
                    return msr?.pref_rating?.pref_score_wsub ?? null;
                case 'scoreEQ':
                    return msr?.pref_rating_eq?.pref_score ?? null;
                case 'lfx':
                    return msr?.pref_rating?.lfx_hz ?? null;
                case 'sensitivity':
                    return (msr?.computed_sensitivity ?? msr?.sensitivity)?.computed ?? null;
                case 'impedance':
                    return msr?.specifications?.impedance ? parseInt(msr.specifications.impedance) : null;
                case 'weight':
                    return msr?.specifications?.weight ? parseFloat(msr.specifications.weight) : null;
                case 'width':
                    return msr?.specifications?.size?.width ? parseInt(msr.specifications.size.width) : null;
                case 'height':
                    return msr?.specifications?.size?.height ? parseInt(msr.specifications.size.height) : null;
                case 'depth':
                    return msr?.specifications?.size?.depth ? parseInt(msr.specifications.size.depth) : null;
                case 'f3':
                    return msr?.estimates?.ref_3dB ? parseFloat(msr.estimates.ref_3dB) : null;
                case 'f6':
                    return msr?.estimates?.ref_6dB ? parseFloat(msr.estimates.ref_6dB) : null;
                case 'bandwidth':
                    return msr?.estimates?.ref_band ? parseFloat(msr.estimates.ref_band) : null;
                case 'quality':
                    return msr?.quality ?? null;
                default:
                    return null;
            }
        }

        function getUniqueBrands() {
            const brands = new Set();
            metadata.forEach((s) => {
                if (s.brand) brands.add(s.brand);
            });
            return Array.from(brands).sort();
        }

        function populateFilterOptions() {
            const brandSelect = document.getElementById('filterBrand');
            if (brandSelect) {
                const brands = getUniqueBrands();
                brands.forEach((b) => {
                    const opt = document.createElement('option');
                    opt.value = b;
                    opt.textContent = b;
                    brandSelect.appendChild(opt);
                });
            }
            const shapeSelect = document.getElementById('filterShape');
            if (shapeSelect) {
                const shapes = getUniqueShapes();
                shapes.forEach((s) => {
                    const opt = document.createElement('option');
                    opt.value = s;
                    opt.textContent = s;
                    shapeSelect.appendChild(opt);
                });
            }
        }

        function getUniqueShapes() {
            const shapes = new Set();
            metadata.forEach((s) => {
                if (s.shape) shapes.add(s.shape);
            });
            return Array.from(shapes).sort();
        }

        function renderCustomChart() {
            const chartType = document.getElementById('chartType').value;
            const xField = document.getElementById('xAxis').value;
            const yField = document.getElementById('yAxis').value;
            const xScale = document.getElementById('xScale').value;
            const yScale = document.getElementById('yScale').value;
            const colorField = document.getElementById('colorBy').value;
            const filterBrand = document.getElementById('filterBrand').value;
            const filterShape = document.getElementById('filterShape').value;
            const showTrendLine = document.getElementById('showTrendLine')?.checked ?? false;
            const showParetoCurve = document.getElementById('showParetoCurve')?.checked ?? false;

            const supportsCurves = chartType === 'scatter' || chartType === 'bar';

            const histogramOptions = {
                nbins: parseInt(document.getElementById('numBins').value) || 20,
                binSize: parseFloat(document.getElementById('binSize').value) || null,
                start: parseFloat(document.getElementById('binStart').value) || null,
                end: parseFloat(document.getElementById('binEnd').value) || null,
            };

            const data = [];
            const groups = new Map();

            metadata.forEach((speaker) => {
                if (filterBrand && speaker.brand !== filterBrand) return;
                if (filterShape && speaker.shape !== filterShape) return;

                const xVal = getFieldValue(speaker, xField);
                const yVal = getFieldValue(speaker, yField);

                if (xVal === null || yVal === null) return;

                const groupKey = colorField !== 'none' ? getFieldValue(speaker, colorField) : 'all';

                if (!groups.has(groupKey)) {
                    groups.set(groupKey, { x: [], y: [], text: [] });
                }
                groups.get(groupKey).x.push(xVal);
                groups.get(groupKey).y.push(yVal);
                groups.get(groupKey).text.push(speaker.brand + ' ' + speaker.model);
            });

            const colors = [
                '#1f77b4',
                '#ff7f0e',
                '#2ca02c',
                '#d62728',
                '#9467bd',
                '#8c564b',
                '#e377c2',
                '#7f7f7f',
                '#bcbd22',
                '#17becf',
            ];
            let colorIdx = 0;

            groups.forEach((group, key) => {
                const trace = {
                    x: group.x,
                    y: group.y,
                    text: group.text,
                    name: key === 'all' ? 'All' : key,
                };

                if (chartType === 'scatter') {
                    trace.mode = 'markers';
                    trace.type = 'scatter';
                    trace.marker = { size: 8, color: colors[colorIdx % colors.length] };
                } else if (chartType === 'histogram') {
                    trace.type = 'histogram';
                    trace.marker = { color: colors[colorIdx % colors.length] };
                    if (histogramOptions.nbins) {
                        trace.nbinsx = histogramOptions.nbins;
                    }
                    if (histogramOptions.binSize) {
                        trace.xbins = {
                            start: histogramOptions.start,
                            end: histogramOptions.end,
                            size: histogramOptions.binSize,
                        };
                    } else if (histogramOptions.start !== null || histogramOptions.end !== null) {
                        trace.xbins = { start: histogramOptions.start, end: histogramOptions.end };
                    }
                } else if (chartType === 'box') {
                    trace.type = 'box';
                    trace.marker = { color: colors[colorIdx % colors.length] };
                } else if (chartType === 'violin') {
                    trace.type = 'violin';
                    trace.marker = { color: colors[colorIdx % colors.length] };
                } else if (chartType === 'bar') {
                    trace.type = 'bar';
                    trace.marker = { color: colors[colorIdx % colors.length] };
                }

                data.push(trace);
                colorIdx++;
            });

            if (supportsCurves) {
                if (showTrendLine) {
                    const allX = [];
                    const allY = [];
                    groups.forEach((group) => {
                        allX.push(...group.x);
                        allY.push(...group.y);
                    });
                    const regression = computeLinearRegression(allX, allY, {
                        logX: xScale === 'log',
                        logY: yScale === 'log',
                    });
                    if (regression) {
                        data.push({
                            x: regression.x,
                            y: regression.y,
                            mode: 'lines',
                            type: 'scatter',
                            name: 'Trend Line',
                            line: {
                                color: 'black',
                                width: 2,
                                dash: 'dot',
                            },
                            hoverinfo: 'skip',
                        });
                    }
                }

                if (showParetoCurve) {
                    const allX = [];
                    const allY = [];
                    groups.forEach((group) => {
                        allX.push(...group.x);
                        allY.push(...group.y);
                    });

                    const paretoBest = computeParetoCurveForData(allX, allY, { direction: 'max' });
                    if (paretoBest.x.length > 0) {
                        data.push({
                            x: paretoBest.x,
                            y: paretoBest.y,
                            mode: 'lines',
                            type: 'scatter',
                            name: 'Pareto (Best Value)',
                            line: {
                                color: 'red',
                                width: 2,
                                dash: 'dash',
                            },
                            hoverinfo: 'skip',
                        });
                    }

                    const paretoWorst = computeParetoCurveForData(allX, allY, { direction: 'min' });
                    if (paretoWorst.x.length > 0) {
                        data.push({
                            x: paretoWorst.x,
                            y: paretoWorst.y,
                            mode: 'lines',
                            type: 'scatter',
                            name: 'Pareto (Worst Value)',
                            line: {
                                color: 'gray',
                                width: 2,
                                dash: 'dash',
                            },
                            hoverinfo: 'skip',
                        });
                    }
                }
            }

            const xLabel = document.getElementById('xAxis').selectedOptions[0].text;
            const yLabel = document.getElementById('yAxis').selectedOptions[0].text;

            const layout = {
                autosize: true,
                title: { text: `Custom Chart: ${yLabel} vs ${xLabel}` },
                xaxis: { title: xLabel, type: xScale },
                yaxis: { title: chartType === 'histogram' ? 'Count' : yLabel, type: yScale },
                legend: { orientation: 'h', y: -0.2 },
                hovermode: 'closest',
                margin: { l: 50, r: 20, t: 40, b: 40 },
            };

            if (chartType === 'histogram') {
                layout.barmode = 'overlay';
            }

            newPlot('visCustomChart', data, layout, { responsive: true });
        }

        function updateHistogramOptions() {
            const chartType = document.getElementById('chartType').value;
            const histOptions = document.getElementById('histogramOptions');
            const curveOptions = document.getElementById('curveOptions');
            if (chartType === 'histogram') {
                histOptions.style.display = 'flex';
            } else {
                histOptions.style.display = 'none';
            }
            if (chartType === 'scatter' || chartType === 'bar') {
                curveOptions.style.display = 'flex';
            } else {
                curveOptions.style.display = 'none';
            }
        }

        document.getElementById('chartType')?.addEventListener('change', updateHistogramOptions);

        function resetCustomChart() {
            document.getElementById('chartType').value = 'scatter';
            document.getElementById('xAxis').value = 'score';
            document.getElementById('yAxis').value = 'score';
            document.getElementById('xScale').value = 'linear';
            document.getElementById('yScale').value = 'linear';
            document.getElementById('colorBy').value = 'none';
            document.getElementById('filterBrand').value = '';
            document.getElementById('filterShape').value = '';
            document.getElementById('numBins').value = 20;
            document.getElementById('binSize').value = '';
            document.getElementById('binStart').value = '';
            document.getElementById('binEnd').value = '';
            document.getElementById('showTrendLine').checked = false;
            document.getElementById('showParetoCurve').checked = false;
            updateHistogramOptions();
            document.getElementById('visCustomChart').innerHTML = '';
        }

        populateFilterOptions();

        document.getElementById('renderCustomChart')?.addEventListener('click', renderCustomChart);
        document.getElementById('resetCustomChart')?.addEventListener('click', resetCustomChart);

        stats();
    })
    .catch((err) => console.log(err.message));
