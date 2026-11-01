// -*- coding: utf-8 -*-
// Compare two speakers as pictures: a server-rendered overlay when the
// compare service can provide it, side-by-side single pictures otherwise.
//
// Copyright (C) 2020-2025 Pierre Aubert pierre(at)spinorama(dot)org
// Licensed under the GNU General Public License v3 or later.

/*eslint no-undef: "error"*/

import {
    getMetadata,
    assignOptions,
    getAllSpeakers,
    constructSpeakerImageUrl,
    constructCompareUrl,
    isCompareOverlayMeasurement,
} from './download.js';
import { knownMeasurements } from './plot.js';

function updateVersion(metaSpeakers, speaker, selector, origin, version) {
    const versions = Object.keys(metaSpeakers[speaker].measurements);
    const matches = new Set();
    versions.forEach((val) => {
        const current = metaSpeakers[speaker].measurements[val];
        if (current.origin === origin || origin === '' || origin == null) {
            matches.add(val);
            matches.add(val + '_eq');
        }
    });
    const [first] = matches;
    let correct = first;
    if (version != null && matches.has(version)) correct = version;
    else if (selector.value != null && matches.has(selector.value)) correct = selector.value;
    assignOptions(Array.from(matches), selector, correct);
}

function updateOriginAndVersion(metaSpeakers, speaker, originSelector, versionSelector, origin, version) {
    const measurements = Object.keys(metaSpeakers[speaker].measurements);
    const origins = new Set();
    for (const key in measurements) origins.add(metaSpeakers[speaker].measurements[measurements[key]].origin);
    const [first] = origins;
    const correctOrigin = origin != null && origins.has(origin) ? origin : first;
    assignOptions(Array.from(origins), originSelector, correctOrigin);
    updateVersion(metaSpeakers, speaker, versionSelector, correctOrigin, version);
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
        const urlCompare = window.location.pathname + '?';
        const nbSpeakers = 2;
        const urlParams = new URLSearchParams(window.location.search);

        const root = document.querySelector('[data-num="0"]');
        const plotError = root.querySelector('#plotError');
        const containers = [root.querySelector('#plot0'), root.querySelector('#plot1'), root.querySelector('#plot2')];
        const form = root.querySelector('#plotForm');
        const graphsSelector = form.querySelector('#compare-select-graph');

        const [metaSpeakers, speakers] = getAllSpeakers(metadata);
        const speakersSelector = [];
        const originsSelector = [];
        const versionsSelector = [];
        const fieldsetOriginsSelector = [];
        const fieldsetVersionsSelector = [];

        function initValue(param, fallback) {
            return urlParams.has(param) ? urlParams.get(param) : fallback;
        }

        function side(pos) {
            return {
                speaker: speakersSelector[pos].value,
                origin: originsSelector[pos].value,
                version: versionsSelector[pos].value,
            };
        }

        function clearPlots() {
            plotError.style.display = 'none';
            for (const c of containers) {
                c.style.display = 'none';
                c.replaceChildren();
            }
        }

        function showSideBySide(measurement, sides) {
            clearPlots();
            sides.forEach((s, i) => {
                containers[i].style.display = 'block';
                containers[i].appendChild(
                    makeImage(
                        constructSpeakerImageUrl(metaSpeakers, measurement, s.speaker, s.origin, s.version),
                        `${measurement} for ${s.speaker}`
                    )
                );
            });
        }

        function showComparison() {
            const measurement = graphsSelector.value;
            const sides = [side(0), side(1)];
            document.title = `Spinorama: compare ${measurement} graphs for speakers ${sides[0].speaker} v.s. ${sides[1].speaker}`;
            urlParams.set('measurement', measurement);
            window.history.pushState({ page: 1 }, 'Compare speakers', urlCompare + urlParams.toString());
            if (!isCompareOverlayMeasurement(measurement)) {
                showSideBySide(measurement, sides);
                return;
            }
            clearPlots();
            containers[0].style.display = 'block';
            const width = window.innerWidth * (window.devicePixelRatio || 1) > 1100 ? 1400 : 900;
            const img = makeImage(
                constructCompareUrl(metaSpeakers, measurement, sides[0], sides[1], width),
                `${measurement} comparison: ${sides[0].speaker} vs ${sides[1].speaker}`
            );
            img.addEventListener('error', () => showSideBySide(measurement, sides), { once: true });
            containers[0].appendChild(img);
        }

        let debounce = null;
        function updateSpeakers() {
            if (debounce) clearTimeout(debounce);
            debounce = setTimeout(showComparison, 400);
        }

        function updateSpeakerPos(pos) {
            updateOriginAndVersion(metaSpeakers, speakersSelector[pos].value, originsSelector[pos], versionsSelector[pos]);
            urlParams.set('speaker' + pos, speakersSelector[pos].value);
            updateOriginPos(pos);
        }

        function updateOriginPos(pos) {
            updateOriginAndVersion(
                metaSpeakers,
                speakersSelector[pos].value,
                originsSelector[pos],
                versionsSelector[pos],
                originsSelector[pos].value
            );
            fieldsetOriginsSelector[pos].disabled = originsSelector[pos].childElementCount === 1;
            urlParams.set('origin' + pos, originsSelector[pos].value);
            updateVersionPos(pos);
        }

        function updateVersionPos(pos) {
            updateVersion(
                metaSpeakers,
                speakersSelector[pos].value,
                versionsSelector[pos],
                originsSelector[pos].value,
                versionsSelector[pos].value
            );
            fieldsetVersionsSelector[pos].disabled = versionsSelector[pos].childElementCount === 1;
            urlParams.set('version' + pos, versionsSelector[pos].value);
            window.history.pushState({ page: 1 }, 'Compare speakers', urlCompare + urlParams.toString());
        }

        const initMeasurement = (() => {
            const m = initValue('measurement', knownMeasurements[0]);
            return knownMeasurements.includes(m) ? m : knownMeasurements[0];
        })();

        for (let pos = 0; pos < nbSpeakers; pos++) {
            const tpos = String(pos);
            speakersSelector[pos] = form.querySelector('#compare-select-speaker' + tpos);
            originsSelector[pos] = form.querySelector('#compare-select-origin' + tpos);
            versionsSelector[pos] = form.querySelector('#compare-select-version' + tpos);
            fieldsetOriginsSelector[pos] = form.querySelector('#compare-fieldset-origin' + tpos);
            fieldsetVersionsSelector[pos] = form.querySelector('#compare-fieldset-version' + tpos);
            const initSpeaker = initValue('speaker' + pos, speakers[Math.floor(Math.random() * speakers.length)]);
            assignOptions(speakers, speakersSelector[pos], initSpeaker);
            updateOriginAndVersion(
                metaSpeakers,
                speakersSelector[pos].value,
                originsSelector[pos],
                versionsSelector[pos],
                initValue('origin' + pos, null),
                initValue('version' + pos, null)
            );
            speakersSelector[pos].addEventListener('change', () => {
                updateSpeakerPos(pos);
                updateSpeakers();
            });
            originsSelector[pos].addEventListener('change', () => {
                updateOriginPos(pos);
                updateSpeakers();
            });
            versionsSelector[pos].addEventListener('change', () => {
                updateVersionPos(pos);
                updateSpeakers();
            });
        }
        assignOptions(knownMeasurements, graphsSelector, initMeasurement);
        graphsSelector.addEventListener('change', updateSpeakers);

        document.addEventListener('keydown', (event) => {
            if (event.key === 'a' || event.key === '1') speakersSelector[0].focus();
            else if (event.key === 'b' || event.key === '2') speakersSelector[1].focus();
            else if (event.key === 'g' || event.key === 'c' || event.key === '3') graphsSelector.focus();
        });

        showComparison();
    })
    .catch((err) => console.log(err.message));
