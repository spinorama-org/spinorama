// Shared display labels and the catalogue of speaker graph names.
// This module intentionally has no dependencies: compare.js and
// similar.js need it during their initialisation.
export const labelShort = {
    'Linear Regression': 'Reg',
    'Band ±1.5dB': '±1.5dB',
    'Band ±3dB': '±3dB',
    'Estimated In-Room Response': 'PIR',
    'On Axis': 'ON',
    'Listening Window': 'LW',
    'Early Reflections': 'ER',
    'Sound Power': 'SP',
    'Early Reflections DI': 'ERDI',
    'Sound Power DI': 'SPDI',
    'Ceiling Bounce': 'CB',
    'Floor Bounce': 'FB',
    'Front Wall Bounce': 'FWB',
    'Rear Wall Bounce': 'RWB',
    'Side Wall Bounce': 'SWB',
    'Ceiling Reflection': 'CR',
    'Floor Reflection': 'FR',
    Front: 'F',
    Rear: 'R',
    Side: 'S',
    'Total Early Reflection': 'TER',
    'Total Horizontal Reflection': 'THR',
    'Total Vertical Reflection': 'TVR',
};

export const labelLong = Object.entries(labelShort).reduce((labels, [long, short]) => {
    labels[short] = long;
    return labels;
}, {});

export const knownMeasurements = [
    'CEA2034',
    'CEA2034 Normalized',
    'On Axis',
    'Estimated In-Room Response',
    'Early Reflections',
    'Horizontal Reflections',
    'Vertical Reflections',
    'SPL Horizontal',
    'SPL Horizontal Normalized',
    'SPL Vertical',
    'SPL Vertical Normalized',
    'SPL Horizontal Contour',
    'SPL Horizontal Contour Normalized',
    'SPL Vertical Contour',
    'SPL Vertical Contour Normalized',
    'SPL Horizontal Contour 3D',
    'SPL Horizontal Contour Normalized 3D',
    'SPL Vertical Contour 3D',
    'SPL Vertical Contour Normalized 3D',
    'SPL Horizontal Globe',
    'SPL Horizontal Globe Normalized',
    'SPL Vertical Globe',
    'SPL Vertical Globe Normalized',
    'SPL Horizontal Radar',
    'SPL Vertical Radar',
];
