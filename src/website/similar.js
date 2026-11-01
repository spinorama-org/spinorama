// -*- coding: utf-8 -*-
// Similar speakers as pictures: server-rendered overlays when available,
// side-by-side single pictures otherwise.
//
// Copyright (C) 2020-2025 Pierre Aubert pierre(at)spinorama(dot)org
// Licensed under the GNU General Public License v3 or later.

/*eslint no-undef: "error"*/

import {
    getMetadata,
    assignOptions,
    constructSpeakerImageUrl,
    constructCompareUrl,
    isCompareOverlayMeasurement,
} from './download.js';
import { knownMeasurements } from './plot.js';

function getNearSpeakers(metadata) {
    const metaSpeakers = {};
    const speakers = [];
    metadata.forEach(function (value) {
        const speaker = value.brand + ' ' + value.model;
        metaSpeakers[speaker] = value;
        if (value.nearest && value.nearest.length > 0) {
            speakers.push(speaker);
        }
    });
    return [metaSpeakers, speakers.sort()];
}

function makeImage(src, alt) {
    const img = document.createElement('img');
    img.src = src;
    img.alt = alt;
    img.loading = 'lazy';
    img.decoding = 'async';
    img.style.maxWidth = '100%';
    img.style.height = 'auto';
    return img;
}

getMetadata()
    .then((metadata) => {
        const urlSimilar = '/similar.html?';
        const urlParams = new URLSearchParams(window.location.search);
        const plotContainer = document.querySelector('[data-num="0"]');
        const formContainer = plotContainer.querySelector('.plotForm');
        const graphSelector = formContainer.querySelector('#similar-select-graph');
        const speakerSelector = formContainer.querySelector('#similar-select-speaker');
        const [metaSpeakers, speakers] = getNearSpeakers(metadata);

        function defaultSide(speakerName) {
            return { speaker: speakerName, origin: null, version: null };
        }

        function pairInto(container, measurement, nameA, nameB) {
            container.replaceChildren();
            const showSideBySide = () => {
                container.replaceChildren(
                    makeImage(constructSpeakerImageUrl(metaSpeakers, measurement, nameA, null, null), `${measurement} for ${nameA}`),
                    makeImage(constructSpeakerImageUrl(metaSpeakers, measurement, nameB, null, null), `${measurement} for ${nameB}`)
                );
            };
            if (!isCompareOverlayMeasurement(measurement)) {
                showSideBySide();
                return;
            }
            const img = makeImage(
                constructCompareUrl(metaSpeakers, measurement, defaultSide(nameA), defaultSide(nameB), 900),
                `${measurement} comparison: ${nameA} vs ${nameB}`
            );
            img.addEventListener('error', showSideBySide, { once: true });
            container.appendChild(img);
        }

        function updatePlots() {
            const speakerName = speakerSelector.value;
            const graphName = graphSelector.value;
            const neighbors = [];
            const similars = metaSpeakers[speakerName]?.nearest || [];
            for (const [, neighborName] of similars) {
                if (!metaSpeakers[neighborName]) continue;
                neighbors.push(neighborName);
            }
            for (let i = 0; i < 10; i++) {
                const el = document.getElementById('plot' + i);
                if (!el) continue;
                if (i < neighbors.length) pairInto(el, graphName, speakerName, neighbors[i]);
                else el.replaceChildren();
            }
            urlParams.set('measurement', graphName);
            urlParams.set('speaker0', speakerName);
            history.pushState({ page: 1 }, 'Change measurement', urlSimilar + urlParams.toString());
        }

        const initSpeaker = (() => {
            const wanted = urlParams.get('speaker0');
            if (wanted && speakers.includes(wanted)) return wanted;
            return speakers[Math.floor(Math.random() * speakers.length)];
        })();
        assignOptions(speakers, speakerSelector, initSpeaker);
        assignOptions(knownMeasurements, graphSelector, knownMeasurements[0]);
        graphSelector.addEventListener('change', updatePlots, false);
        speakerSelector.addEventListener('change', updatePlots, false);
        updatePlots();
    })
    .catch((err) => console.log(err.message));
