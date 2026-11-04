//! Format-agnostic figure model built from computed curves.

// Rust guideline compliant 2026-02-21

use crate::compute::cea2034::{Cea2034, SpinData};
use crate::measurements::{SplPlane, angle_degrees};

/// Y axis a trace is plotted against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Primary SPL axis (`y`).
    Primary,
    /// Secondary directivity-index axis (`y2`).
    Secondary,
}

impl Axis {
    /// Returns the Plotly / trace-bundle axis identifier.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Primary => "y",
            Self::Secondary => "y2",
        }
    }
}

/// Plotly trace visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// Trace is shown and included in trace bundles.
    Visible,
    /// Trace is present in Plotly JSON but excluded from trace bundles.
    LegendOnly,
    /// Trace is hidden and excluded from trace bundles.
    Hidden,
}

/// One named curve of a figure.
#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    /// Trace name as shown in the legend.
    pub name: String,
    /// Optional per-trace x axis; `None` uses the figure frequency axis.
    pub x: Option<Vec<f64>>,
    /// dB values aligned with `x` or the figure frequency axis.
    pub values: Vec<f64>,
    /// Axis the trace belongs to.
    pub axis: Axis,
    /// Plotly visibility.
    pub visibility: Visibility,
    /// Whether the trace appears in the legend.
    pub show_legend: bool,
    /// Plotly line dash style, when not solid.
    pub dash: Option<String>,
    /// Plotly line/marker color, when fixed by the Python plot.
    pub color: Option<String>,
    /// Plotly fill mode for closed band traces.
    pub fill: Option<String>,
    /// Plotly trace opacity (band traces use 0.25 like Python).
    pub opacity: Option<f64>,
    /// Plotly mode override (`lines+markers`, `markers`, `none`).
    pub mode: Option<String>,
    /// Plotly marker symbol name, when markers are shown.
    pub marker_symbol: Option<String>,
    /// Plotly marker size in pixels, when markers are shown.
    pub marker_size: Option<f64>,
}

impl Trace {
    /// Creates a visible trace on the figure frequency axis.
    #[must_use]
    pub fn new(name: impl Into<String>, values: Vec<f64>, axis: Axis) -> Self {
        Self {
            name: name.into(),
            x: None,
            values,
            axis,
            visibility: Visibility::Visible,
            show_legend: true,
            dash: None,
            color: None,
            fill: None,
            opacity: None,
            mode: None,
            marker_symbol: None,
            marker_size: None,
        }
    }

    /// Creates a visible trace with its own x axis.
    #[must_use]
    pub fn with_x(name: impl Into<String>, x: Vec<f64>, values: Vec<f64>, axis: Axis) -> Self {
        Self {
            x: Some(x),
            ..Self::new(name, values, axis)
        }
    }

    /// Marks this trace `legendonly` in Plotly output.
    #[must_use]
    pub fn legend_only(mut self) -> Self {
        self.visibility = Visibility::LegendOnly;
        self
    }

    /// Sets the Plotly dash style.
    #[must_use]
    pub fn dash(mut self, dash: impl Into<String>) -> Self {
        self.dash = Some(dash.into());
        self
    }

    /// Sets the Plotly color.
    #[must_use]
    pub fn color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Sets the Plotly fill mode.
    #[must_use]
    pub fn fill(mut self, fill: impl Into<String>) -> Self {
        self.fill = Some(fill.into());
        self
    }

    /// Sets the Plotly trace opacity.
    #[must_use]
    pub fn opacity(mut self, opacity: f64) -> Self {
        self.opacity = Some(opacity);
        self
    }

    /// Sets the Plotly mode (`lines`, `markers`, `none`, …).
    #[must_use]
    pub fn mode(mut self, mode: impl Into<String>) -> Self {
        self.mode = Some(mode.into());
        self
    }

    /// Shows markers with the given Plotly symbol and size.
    #[must_use]
    pub fn marker(mut self, symbol: impl Into<String>, size: f64) -> Self {
        self.mode = Some("lines+markers".to_owned());
        self.marker_symbol = Some(symbol.into());
        self.marker_size = Some(size);
        self
    }

    /// Hides this trace from the legend while keeping it visible.
    #[must_use]
    pub fn no_legend(mut self) -> Self {
        self.show_legend = false;
        self
    }
}

/// Contour or surface data for one SPL plane.
#[derive(Debug, Clone, PartialEq)]
pub struct ContourData {
    /// Angle axis in degrees, ascending.
    pub angles: Vec<f64>,
    /// SPL matrix as rows per angle, columns per frequency.
    pub z: Vec<Vec<f64>>,
    /// Optional surface color matrix, same shape as `z`.
    pub surface_color: Option<Vec<Vec<f64>>>,
    /// Whether to emit a 3D surface instead of a 2D contour.
    pub surface: bool,
}

/// One polar trace in a radar subplot.
#[derive(Debug, Clone, PartialEq)]
pub struct RadarTrace {
    /// Trace name, usually a frequency label.
    pub name: String,
    /// Radial SPL values.
    pub r: Vec<f64>,
    /// Angular values in degrees.
    pub theta: Vec<f64>,
    /// One-based subplot row.
    pub row: usize,
    /// One-based subplot column.
    pub col: usize,
    /// Plotly marker color, when fixed.
    pub color: Option<String>,
}

/// One categorical bar trace.
#[derive(Debug, Clone, PartialEq)]
pub struct BarTrace {
    /// Trace name shown in the legend.
    pub name: String,
    /// Category labels.
    pub categories: Vec<String>,
    /// Bar values aligned with `categories`.
    pub values: Vec<f64>,
    /// Plotly marker color, when fixed.
    pub color: Option<String>,
}

/// Figure payload kind.
#[derive(Debug, Clone, PartialEq)]
pub enum FigureKind {
    /// Ordinary frequency-domain scatter traces.
    Scatter,
    /// Contour or 3D surface over frequency and angle.
    Contour(ContourData),
    /// Four-panel polar radar traces.
    Radar(Vec<RadarTrace>),
    /// Categorical bar traces.
    Bar(Vec<BarTrace>),
    /// Plotly `make_subplots` sheet of mini charts.
    Subplot(Vec<SubplotCell>),
}

/// One cell of a `make_subplots` sheet figure.
#[derive(Debug, Clone, PartialEq)]
pub struct SubplotCell {
    /// Subplot title (Plotly paper annotation above the cell).
    pub title: String,
    /// Scatter traces of the cell (empty for bar cells).
    pub traces: Vec<Trace>,
    /// Bar traces of the cell (empty for scatter cells).
    pub bars: Vec<BarTrace>,
    /// Whether the cell x axis is log-frequency (false: categorical).
    pub log_x: bool,
}

/// One figure: named traces over a shared log frequency axis.
#[derive(Debug, Clone, PartialEq)]
pub struct Figure {
    /// Graph stem, also the output file name without extension.
    pub name: String,
    /// Display title.
    pub title: String,
    /// Frequency axis in hertz.
    pub freq: Vec<f64>,
    /// Traces of the figure.
    pub traces: Vec<Trace>,
    /// Payload kind; non-scatter kinds carry their data here.
    pub kind: FigureKind,
    /// Explicit x grid/tick values (hertz) for dense-grid rendering.
    pub x_tickvals: Option<Vec<f64>>,
    /// Explicit y grid/tick values (dB) for dense-grid rendering.
    pub y_tickvals: Option<Vec<f64>>,
}

impl Figure {
    /// Creates a scatter figure.
    #[must_use]
    pub fn scatter(
        name: impl Into<String>,
        title: impl Into<String>,
        freq: Vec<f64>,
        traces: Vec<Trace>,
    ) -> Self {
        Self {
            name: name.into(),
            title: title.into(),
            freq,
            traces,
            kind: FigureKind::Scatter,
            x_tickvals: None,
            y_tickvals: None,
        }
    }
}

/// Marker symbols cycled across traces by the scan-hardening style.
pub const SCAN_SYMBOLS: &[&str] = &[
    "circle",
    "square",
    "diamond",
    "cross",
    "x",
    "triangle-up",
    "triangle-down",
    "triangle-left",
    "triangle-right",
    "star",
];

/// Applies the anti-scan test style to a scatter figure: every curve
/// in the same grey as the grid, distinguished by a different marker
/// symbol, plus a dense grid (every 1 dB, minor log ticks on the
/// frequency axis). Filled zones (bands, N/A) stay filled, in grey.
pub fn apply_scan_style(figure: &mut Figure) {
    if !matches!(figure.kind, FigureKind::Scatter) {
        return;
    }
    let mut symbol_index = 0usize;
    for trace in &mut figure.traces {
        if trace.fill.is_some() {
            trace.color = Some("#CCCCCC".to_owned());
            continue;
        }
        trace.color = Some("#7F7F7F".to_owned());
        if trace.mode.as_deref() != Some("none") {
            let symbol = SCAN_SYMBOLS[symbol_index % SCAN_SYMBOLS.len()];
            symbol_index += 1;
            trace.mode = Some("lines+markers".to_owned());
            trace.marker_symbol = Some(symbol.to_owned());
            trace.marker_size = Some(7.0);
        }
    }
    let mut x_tickvals = Vec::new();
    for decade in [10.0, 100.0, 1000.0, 10000.0] {
        for m in 1..=9 {
            let value = f64::from(m) * decade;
            if (20.0..=20000.0).contains(&value) {
                x_tickvals.push(value);
            }
        }
    }
    figure.x_tickvals = Some(x_tickvals);
    figure.y_tickvals = Some((-45..=5).map(f64::from).collect());
}

const SLOPE_MIN_HZ: f64 = 100.0;
const SLOPE_MAX_HZ: f64 = 12_000.0;
const MIDRANGE_MIN_HZ: f64 = 300.0;
const MIDRANGE_MAX_HZ: f64 = 5_000.0;

fn linregress_log10(freq: &[f64], values: &[f64]) -> Option<(f64, f64)> {
    let points: Vec<(f64, f64)> = freq
        .iter()
        .zip(values)
        .filter(|(f, v)| **f >= SLOPE_MIN_HZ && **f <= SLOPE_MAX_HZ && v.is_finite())
        .map(|(f, v)| (f.log10(), *v))
        .collect();
    if points.len() < 2 {
        return None;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "point counts fit exactly in f64"
    )]
    let n = points.len() as f64;
    let mean_x = points.iter().map(|point| point.0).sum::<f64>() / n;
    let mean_y = points.iter().map(|point| point.1).sum::<f64>() / n;
    let denominator: f64 = points.iter().map(|point| (point.0 - mean_x).powi(2)).sum();
    if denominator == 0.0 {
        return None;
    }
    let slope = points
        .iter()
        .map(|point| (point.0 - mean_x) * (point.1 - mean_y))
        .sum::<f64>()
        / denominator;
    Some((slope, mean_y - slope * mean_x))
}

fn band_trace(freq: &[f64], line: &[f64], offset_db: f64, name: &str) -> Trace {
    let mut x: Vec<f64> = freq.to_vec();
    x.extend(freq.iter().rev().copied());
    let mut y: Vec<f64> = line.iter().map(|value| value + offset_db).collect();
    y.extend(line.iter().rev().map(|value| value - offset_db));
    let trace = Trace::with_x(name, x, y, Axis::Primary)
        .color("#E2F705")
        .fill("toself");
    // Python shades the ±3 dB zone at 25% opacity; ±1.5 dB is opaque.
    if offset_db > 2.0 {
        trace.opacity(0.25)
    } else {
        trace
    }
}

/// Builds Python regression/band traces around one measurement curve.
#[must_use]
pub fn regression_traces(
    freq: &[f64],
    values: &[f64],
    measurement: &str,
    line_title: Option<&str>,
) -> Vec<Trace> {
    let Some((slope, intercept)) = linregress_log10(freq, values) else {
        return vec![Trace::new(measurement, values.to_vec(), Axis::Primary)];
    };
    let line: Vec<f64> = freq.iter().map(|f| slope * f.log10() + intercept).collect();
    let restricted: Vec<usize> = freq
        .iter()
        .enumerate()
        .filter(|(_, f)| **f >= MIDRANGE_MIN_HZ && **f <= MIDRANGE_MAX_HZ)
        .map(|(index, _)| index)
        .collect();
    let restricted_freq: Vec<f64> = restricted.iter().map(|index| freq[*index]).collect();
    let restricted_line: Vec<f64> = restricted.iter().map(|index| line[*index]).collect();

    let mut traces = vec![
        band_trace(freq, &line, 3.0, "Band ±3dB"),
        band_trace(freq, &line, 1.5, "Band ±1.5dB"),
    ];
    let mut line_trace = Trace::new(
        line_title.unwrap_or("Linear interpolation"),
        line,
        Axis::Primary,
    )
    .dash("dot")
    .color("black");
    if line_title.is_none() {
        line_trace.show_legend = false;
    }
    traces.push(line_trace);
    traces.push(
        Trace::with_x(
            "Midrange Band +3dB",
            restricted_freq.clone(),
            restricted_line.iter().map(|value| value + 3.0).collect(),
            Axis::Primary,
        )
        .dash("dash")
        .color("black")
        .no_legend(),
    );
    traces.push(
        Trace::with_x(
            "Midrange Band -3dB",
            restricted_freq,
            restricted_line.iter().map(|value| value - 3.0).collect(),
            Axis::Primary,
        )
        .dash("dash")
        .color("black")
        .no_legend(),
    );
    traces.push(Trace::new(measurement, values.to_vec(), Axis::Primary));
    traces
}

fn cea_traces(cea: &Cea2034) -> Vec<Trace> {
    vec![
        Trace::new("On Axis", cea.on_axis.clone(), Axis::Primary),
        Trace::new(
            "Listening Window",
            cea.listening_window.clone(),
            Axis::Primary,
        ),
        Trace::new(
            "Early Reflections",
            cea.early_reflections.clone(),
            Axis::Primary,
        ),
        Trace::new("Sound Power", cea.sound_power.clone(), Axis::Primary),
        Trace::new(
            "Early Reflections DI",
            cea.early_reflections_di.clone(),
            Axis::Secondary,
        ),
        Trace::new(
            "Sound Power DI",
            cea.sound_power_di.clone(),
            Axis::Secondary,
        ),
    ]
}

/// Builds one CEA2034 spin figure.
#[must_use]
pub fn cea_figure(name: &str, title: String, cea: &Cea2034) -> Figure {
    Figure::scatter(name, title, cea.freq.clone(), cea_traces(cea))
}

fn derived_normalized(cea: &Cea2034) -> Cea2034 {
    let subtract_on = |curve: &[f64]| {
        curve
            .iter()
            .zip(&cea.on_axis)
            .map(|(value, on)| value - on)
            .collect()
    };
    Cea2034 {
        freq: cea.freq.clone(),
        on_axis: subtract_on(&cea.on_axis),
        listening_window: subtract_on(&cea.listening_window),
        early_reflections: subtract_on(&cea.early_reflections),
        sound_power: subtract_on(&cea.sound_power),
        early_reflections_di: cea.early_reflections_di.clone(),
        sound_power_di: cea.sound_power_di.clone(),
        estimated_inroom: subtract_on(&cea.estimated_inroom),
        sensitivity_db: cea.sensitivity_db,
    }
}

fn cea_spl_range(cea: &Cea2034) -> (f64, f64) {
    let max_spl = [
        &cea.on_axis,
        &cea.listening_window,
        &cea.early_reflections,
        &cea.sound_power,
    ]
    .iter()
    .flat_map(|curve| curve.iter().copied())
    .filter(|value| value.is_finite())
    .fold(0.0_f64, f64::max);
    let top = 5.0 + (max_spl / 5.0).trunc() * 5.0;
    (top - 50.0, top)
}

fn valid_range_traces(valid_freq_range: (f64, f64), spl_range: (f64, f64)) -> Vec<Trace> {
    let (min_freq, max_freq) = valid_freq_range;
    let (min_spl, max_spl) = spl_range;
    let mut traces = Vec::new();
    if min_freq > 30.0 {
        traces.push(
            Trace::with_x(
                "N/A",
                vec![20.0, min_freq, min_freq, 20.0],
                vec![min_spl, min_spl, max_spl, max_spl],
                Axis::Primary,
            )
            .color("LightGreen")
            .fill("toself")
            .opacity(0.3)
            .mode("none"),
        );
    }
    if max_freq < 19_500.0 {
        traces.push(
            Trace::with_x(
                "N/A",
                vec![max_freq, 20_000.0, 20_000.0, max_freq],
                vec![min_spl, min_spl, max_spl, max_spl],
                Axis::Primary,
            )
            .color("LightGreen")
            .fill("toself")
            .opacity(0.3)
            .mode("none")
            .no_legend(),
        );
    }
    traces
}

fn append_valid_range(figures: &mut [Figure], valid_freq_range: (f64, f64), spl_range: (f64, f64)) {
    let traces = valid_range_traces(valid_freq_range, spl_range);
    if traces.is_empty() {
        return;
    }
    for figure in figures {
        figure.traces.extend(traces.iter().cloned());
    }
}

/// Builds the five common figures for one CEA2034 curve set.
///
/// When `cea_normalized` is `None`, normalized curves are derived by
/// subtracting On Axis, matching the partial curve-loader path. HV callers
/// pass the recomputed normalized spin from [`SpinData`].
#[must_use]
pub fn common_figures(
    cea: &Cea2034,
    cea_normalized: Option<&Cea2034>,
    speaker: &str,
    measured_by: &str,
    valid_freq_range: (f64, f64),
) -> Vec<Figure> {
    let title = |graph: &str| format!("{graph} for {speaker} measured by {measured_by}");
    let fallback_normalized;
    let normalized = match cea_normalized {
        Some(normalized) => normalized,
        None => {
            fallback_normalized = derived_normalized(cea);
            &fallback_normalized
        }
    };
    let mut figures = vec![
        Figure::scatter(
            "CEA2034",
            title("CEA2034"),
            cea.freq.clone(),
            cea_traces(cea),
        ),
        Figure::scatter(
            "CEA2034 Normalized",
            title("CEA2034 Normalized"),
            normalized.freq.clone(),
            cea_traces(normalized),
        ),
        Figure::scatter(
            "On Axis",
            title("On Axis"),
            cea.freq.clone(),
            regression_traces(&cea.freq, &cea.on_axis, "On Axis", Some("Midrange ±3dB")),
        ),
        Figure::scatter(
            "Estimated In-Room Response",
            title("Estimated In-Room Response"),
            cea.freq.clone(),
            regression_traces(
                &cea.freq,
                &cea.estimated_inroom,
                "Estimated In-Room Response",
                Some("Midrange ±3dB"),
            ),
        ),
        Figure::scatter(
            "Estimated In-Room Response Normalized",
            title("Estimated In-Room Response Normalized"),
            cea.freq.clone(),
            // Python display_inroom_normalized keeps the non-normalized EIR
            // as the main curve; only the hidden confidence-zone spin changes.
            regression_traces(
                &cea.freq,
                &cea.estimated_inroom,
                "Estimated In-Room Response",
                Some("Midrange ±3dB"),
            ),
        ),
    ];
    append_valid_range(&mut figures[0..1], valid_freq_range, cea_spl_range(cea));
    append_valid_range(
        &mut figures[1..2],
        valid_freq_range,
        cea_spl_range(normalized),
    );
    append_valid_range(&mut figures[2..], valid_freq_range, (-40.0, 10.0));
    figures
}

fn named_curve_figure(
    name: &str,
    title: String,
    freq: &[f64],
    curves: &[(String, Vec<f64>)],
) -> Figure {
    Figure::scatter(
        name,
        title,
        freq.to_vec(),
        curves
            .iter()
            .map(|(curve_name, values)| {
                Trace::new(curve_name.clone(), values.clone(), Axis::Primary)
            })
            .collect(),
    )
}

fn spl_plane_figure(name: &str, title: String, plane: &SplPlane) -> Figure {
    let mut traces = Vec::new();
    let trend = plane
        .column("On Axis")
        .map(|on_axis| regression_traces(&plane.freq, on_axis, "On Axis", Some("Trend line")));
    for (column, values) in &plane.columns {
        let Some(angle) = angle_degrees(column) else {
            continue;
        };
        if column == "On Axis" {
            if let Some(trend_traces) = &trend
                && trend_traces.len() > 1
            {
                traces.extend(trend_traces[..trend_traces.len() - 1].iter().cloned());
            }
            traces.push(Trace::new("ON", values.clone(), Axis::Primary));
            continue;
        }
        let mut trace = Trace::new(column.clone(), values.clone(), Axis::Primary);
        if !(0..=60).contains(&angle) || angle < 0 {
            trace = trace.legend_only();
        }
        traces.push(trace);
    }
    Figure::scatter(name, title, plane.freq.clone(), traces)
}

/// Builds all scatter figures currently ported for HV measurements.
///
/// This covers the five common views, three reflection views, and four SPL
/// angle views. Contour, radar, directivity, and group-delay figures remain
/// separate figure kinds and are not represented by this scatter model yet.
#[must_use]
pub fn spin_figures(
    data: &SpinData,
    speaker: &str,
    measured_by: &str,
    valid_freq_range: (f64, f64),
) -> Vec<Figure> {
    let title = |graph: &str| format!("{graph} for {speaker} measured by {measured_by}");
    let mut figures = common_figures(
        &data.cea,
        Some(&data.cea_normalized),
        speaker,
        measured_by,
        valid_freq_range,
    );
    let mut hv_scatter = Vec::new();
    if let Some(group_delay) =
        group_delay_figure("Group Delay", title("Group Delay"), &data.horizontal)
    {
        hv_scatter.push(group_delay);
    }
    hv_scatter.push(named_curve_figure(
        "Early Reflections",
        title("Early Reflections"),
        &data.cea.freq,
        &data.early_reflections,
    ));
    hv_scatter.push(named_curve_figure(
        "Horizontal Reflections",
        title("Horizontal Reflections"),
        &data.cea.freq,
        &data.horizontal_reflections,
    ));
    hv_scatter.push(named_curve_figure(
        "Vertical Reflections",
        title("Vertical Reflections"),
        &data.cea.freq,
        &data.vertical_reflections,
    ));
    hv_scatter.push(spl_plane_figure(
        "SPL Horizontal",
        title("SPL Horizontal"),
        &data.horizontal,
    ));
    hv_scatter.push(spl_plane_figure(
        "SPL Vertical",
        title("SPL Vertical"),
        &data.vertical,
    ));
    hv_scatter.push(spl_plane_figure(
        "SPL Horizontal Normalized",
        title("SPL Horizontal Normalized"),
        &data.horizontal_normalized,
    ));
    hv_scatter.push(spl_plane_figure(
        "SPL Vertical Normalized",
        title("SPL Vertical Normalized"),
        &data.vertical_normalized,
    ));
    append_valid_range(&mut hv_scatter, valid_freq_range, (-40.0, 10.0));
    figures.extend(hv_scatter);

    let contour_h = contour_figure(
        "SPL Horizontal Contour",
        title("SPL Horizontal Contour"),
        &data.horizontal,
        false,
    );
    let contour_v = contour_figure(
        "SPL Vertical Contour",
        title("SPL Vertical Contour"),
        &data.vertical,
        false,
    );
    let contour_hn = contour_figure(
        "SPL Horizontal Contour Normalized",
        title("SPL Horizontal Contour Normalized"),
        &data.horizontal_normalized,
        false,
    );
    let contour_vn = contour_figure(
        "SPL Vertical Contour Normalized",
        title("SPL Vertical Contour Normalized"),
        &data.vertical_normalized,
        false,
    );
    if let Some(contour) = &contour_hn
        && let Some(figure) = directivity_figure(
            "Horizontal Directivity",
            title("Horizontal Directivity"),
            contour,
        )
    {
        figures.push(figure);
    }
    if let Some(contour) = &contour_vn
        && let Some(figure) = directivity_figure(
            "Vertical Directivity",
            title("Vertical Directivity"),
            contour,
        )
    {
        figures.push(figure);
    }
    figures.extend(contour_h);
    figures.extend(contour_v);
    figures.extend(contour_hn);
    figures.extend(contour_vn);
    figures.extend(contour_figure(
        "SPL Horizontal Contour 3D",
        title("SPL Horizontal Contour 3D"),
        &data.horizontal,
        true,
    ));
    figures.extend(contour_figure(
        "SPL Vertical Contour 3D",
        title("SPL Vertical Contour 3D"),
        &data.vertical,
        true,
    ));
    figures.extend(contour_figure(
        "SPL Horizontal Contour Normalized 3D",
        title("SPL Horizontal Contour Normalized 3D"),
        &data.horizontal_normalized,
        true,
    ));
    figures.extend(contour_figure(
        "SPL Vertical Contour Normalized 3D",
        title("SPL Vertical Contour Normalized 3D"),
        &data.vertical_normalized,
        true,
    ));
    figures.push(radar_figure(
        "SPL Horizontal Radar",
        title("SPL Horizontal Radar"),
        &data.horizontal,
    ));
    figures.push(radar_figure(
        "SPL Vertical Radar",
        title("SPL Vertical Radar"),
        &data.vertical,
    ));
    figures
}

const RADAR_COLORS: &[(&str, &str)] = &[
    ("100 Hz", "#5c77a5"),
    ("125 Hz", "#dc842a"),
    ("160 Hz", "#c85857"),
    ("200 Hz", "#89b5b1"),
    ("250 Hz", "#71a152"),
    ("315 Hz", "#bab0ac"),
    ("400 Hz", "#e15759"),
    ("500 Hz", "#b07aa1"),
    ("1600 Hz", "#76b7b2"),
    ("2000 Hz", "#ff9da7"),
    ("2500 Hz", "#5c77a5"),
    ("3150 Hz", "#dc842a"),
    ("4000 Hz", "#c85857"),
    ("5000 Hz", "#89b5b1"),
    ("6000 Hz", "#71a152"),
    ("8000 Hz", "#bab0ac"),
];

fn contour_rows(plane: &SplPlane) -> Vec<(f64, Vec<f64>)> {
    let mut rows: Vec<(f64, Vec<f64>)> = plane
        .columns
        .iter()
        .filter_map(|(name, values)| {
            angle_degrees(name).map(|angle| (f64::from(angle), values.clone()))
        })
        .collect();
    rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if !rows.iter().any(|(angle, _)| *angle == -180.0)
        && let Some((_, values)) = rows.iter().find(|(angle, _)| *angle == 180.0)
    {
        rows.insert(0, (-180.0, values.clone()));
    }
    rows
}

fn contour_figure(name: &str, title: String, plane: &SplPlane, surface: bool) -> Option<Figure> {
    let rows = contour_rows(plane);
    if rows.len() < 2 {
        // A contour needs at least two angles; single-angle planes
        // (e.g. Princeton vertical) have no contour to draw.
        return None;
    }
    let angles: Vec<f64> = rows.iter().map(|(angle, _)| *angle).collect();
    let indices: Vec<usize> = plane
        .freq
        .iter()
        .enumerate()
        .filter(|(_, freq)| **freq >= 100.0 && **freq <= 20_000.0)
        .map(|(index, _)| index)
        .collect();
    let mut freq: Vec<f64> = indices.iter().map(|index| plane.freq[*index]).collect();
    let (z, surface_color) = if surface {
        let db_max = plane
            .column("On Axis")
            .map(|values| values.iter().copied().fold(f64::NEG_INFINITY, f64::max))
            .unwrap_or(0.0);
        // Python transform subtracts db_max from the whole frame, including
        // the frequency column; preserve that payload behaviour.
        for value in &mut freq {
            *value -= db_max;
        }
        let z: Vec<Vec<f64>> = rows
            .iter()
            .map(|(_, values)| {
                indices
                    .iter()
                    .map(|index| (values[*index] - db_max).clamp(-45.0, 5.0))
                    .collect()
            })
            .collect();
        let color: Vec<Vec<f64>> = z
            .iter()
            .map(|row| {
                row.iter()
                    .map(|value| (value.div_euclid(3.0) * 3.0).clamp(-30.0, 3.0))
                    .collect()
            })
            .collect();
        (z, Some(color))
    } else {
        let z: Vec<Vec<f64>> = rows
            .iter()
            .map(|(_, values)| {
                indices
                    .iter()
                    .map(|index| values[*index].clamp(-30.0, 3.0))
                    .collect()
            })
            .collect();
        (z, None)
    };
    Some(Figure {
        name: name.to_owned(),
        title,
        freq,
        traces: Vec::new(),
        x_tickvals: None,
        y_tickvals: None,
        kind: FigureKind::Contour(ContourData {
            angles,
            z,
            surface_color,
            surface,
        }),
    })
}

fn radar_figure(name: &str, title: String, plane: &SplPlane) -> Figure {
    let step = if plane.column("5°").is_some() && plane.column("25°").is_some() {
        5
    } else {
        10
    };
    let angles: Vec<i32> = (-180..180).step_by(step as usize).collect();
    let groups: [(&[i32], usize, usize); 4] = [
        (&[100, 125, 160, 200], 1, 1),
        (&[1600, 2000, 2500, 3150], 1, 2),
        (&[250, 315, 400, 500], 2, 1),
        (&[4000, 5000, 6300, 8000], 2, 2),
    ];
    let mut traces = Vec::new();
    for (freqs, row, col) in groups {
        for hz in freqs {
            let target = f64::from(*hz);
            let Some(index) = plane
                .freq
                .iter()
                .position(|freq| (freq - target).abs() < target * 0.05)
            else {
                continue;
            };
            // Python plot_radar zips the angle list with sorted columns in
            // order; preserve that pairing even when -180° is absent.
            let mut column_order: Vec<usize> = (0..plane.columns.len())
                .filter(|index| angle_degrees(&plane.columns[*index].0).is_some())
                .collect();
            column_order.sort_by_key(|index| angle_degrees(&plane.columns[*index].0).unwrap_or(0));
            let mut r: Vec<f64> = angles
                .iter()
                .zip(&column_order)
                .map(|(_, column_index)| plane.columns[*column_index].1[index])
                .collect();
            if r.is_empty() {
                continue;
            }
            let mut theta: Vec<f64> = angles[..r.len()]
                .iter()
                .map(|angle| f64::from(*angle))
                .collect();
            theta.push(theta[0]);
            r.push(r[0]);
            let label = format!("{hz} Hz");
            traces.push(RadarTrace {
                color: RADAR_COLORS
                    .iter()
                    .find(|(name, _)| *name == label)
                    .map(|(_, color)| (*color).to_owned()),
                name: label,
                r,
                theta,
                row,
                col,
            });
        }
    }
    Figure {
        name: name.to_owned(),
        title,
        freq: plane.freq.clone(),
        traces: Vec::new(),
        x_tickvals: None,
        y_tickvals: None,
        kind: FigureKind::Radar(traces),
    }
}

fn directivity_figure(name: &str, title: String, contour: &Figure) -> Option<Figure> {
    let FigureKind::Contour(data) = &contour.kind else {
        return None;
    };
    let crossing = |indices: &[usize], fi: usize| -> Option<f64> {
        for &ai in indices {
            if data.z[ai][fi] >= -6.0 {
                continue;
            }
            let prev = if data.angles[ai] > 0.0 {
                ai.checked_sub(1)?
            } else {
                ai + 1
            };
            if prev < data.angles.len()
                && data.z[prev][fi] >= -6.0
                && data.z[ai][fi] != data.z[prev][fi]
            {
                let ratio = (-6.0 - data.z[prev][fi]) / (data.z[ai][fi] - data.z[prev][fi]);
                return Some(data.angles[prev] + ratio * (data.angles[ai] - data.angles[prev]));
            }
            return Some(data.angles[ai]);
        }
        None
    };
    let pos_idx: Vec<usize> = data
        .angles
        .iter()
        .enumerate()
        .filter(|(_, angle)| **angle > 0.0)
        .map(|(index, _)| index)
        .collect();
    let neg_idx: Vec<usize> = data
        .angles
        .iter()
        .enumerate()
        .filter(|(_, angle)| **angle < 0.0)
        .map(|(index, _)| index)
        .rev()
        .collect();
    let (mut pos_x, mut pos_y, mut neg_x, mut neg_y) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (fi, freq) in contour.freq.iter().enumerate() {
        if data.z.iter().any(|row| row.len() <= fi) {
            continue;
        }
        if let Some(angle) = crossing(&pos_idx, fi) {
            pos_x.push(*freq);
            pos_y.push(angle);
        }
        if let Some(angle) = crossing(&neg_idx, fi) {
            neg_x.push(*freq);
            neg_y.push(angle);
        }
    }
    if pos_x.is_empty() && neg_x.is_empty() {
        return None;
    }
    Some(Figure::scatter(
        name,
        title,
        contour.freq.clone(),
        vec![
            Trace::with_x("+", pos_x, pos_y, Axis::Primary)
                .color("#4758a9")
                .no_legend(),
            Trace::with_x("-", neg_x, neg_y, Axis::Primary)
                .color("#745470")
                .dash("dot")
                .no_legend(),
        ],
    ))
}

fn group_delay_figure(name: &str, title: String, plane: &SplPlane) -> Option<Figure> {
    let phase_deg = plane.phase_on_axis.as_ref()?;
    if plane.freq.len() < 2 || phase_deg.len() != plane.freq.len() {
        return None;
    }
    let mut phase_rad: Vec<f64> = phase_deg.iter().map(|value| value.to_radians()).collect();
    let mut offset = 0.0;
    for i in 1..phase_rad.len() {
        let previous = phase_deg[i - 1].to_radians() + offset;
        let mut current = phase_deg[i].to_radians() + offset;
        let diff = current - previous;
        if diff > std::f64::consts::PI {
            offset -= 2.0 * std::f64::consts::PI;
            current -= 2.0 * std::f64::consts::PI;
        } else if diff < -std::f64::consts::PI {
            offset += 2.0 * std::f64::consts::PI;
            current += 2.0 * std::f64::consts::PI;
        }
        phase_rad[i] = current;
    }
    let freq = &plane.freq;
    let mut group_delay = Vec::with_capacity(freq.len());
    for i in 0..freq.len() {
        let derivative = if i == 0 {
            (phase_rad[1] - phase_rad[0]) / (freq[1] - freq[0])
        } else if i + 1 == freq.len() {
            (phase_rad[i] - phase_rad[i - 1]) / (freq[i] - freq[i - 1])
        } else {
            let h_prev = freq[i] - freq[i - 1];
            let h_next = freq[i + 1] - freq[i];
            ((h_prev * h_prev) * phase_rad[i + 1]
                + (h_next * h_next - h_prev * h_prev) * phase_rad[i]
                - (h_next * h_next) * phase_rad[i - 1])
                / (h_next * h_prev * (h_next + h_prev))
        };
        // Python plot_graph_group_delay uses -1000 * d(phase_rad)/df directly.
        group_delay.push(-1000.0 * derivative);
    }
    Some(Figure::scatter(
        name,
        title,
        freq.clone(),
        vec![Trace::new("Group Delay (ms)", group_delay, Axis::Primary)],
    ))
}

/// Plotly default color cycle, used by the AutoEQ filter figures.
const PLOTLY_COLORS: [&str; 10] = [
    "#636EFA", "#EF553B", "#00CC96", "#AB63FA", "#FFA15A", "#19D3F3", "#FF6692", "#B6E880",
    "#FF97FF", "#FECB52",
];

/// Slope band for regression lines (Python `SLOPE_MIN/MAX_FREQ`).
const SLOPE_BAND: (f64, f64) = (100.0, 12_000.0);
/// Midrange band for the restricted error histogram.
const MIDRANGE_BAND: (f64, f64) = (300.0, 5_000.0);

/// Least-squares line of `values` against log10(freq) over a band.
fn linregress_band(freq: &[f64], values: &[f64], band: (f64, f64), strict: bool) -> (f64, f64) {
    let mut n = 0.0;
    let mut sx = 0.0;
    let mut sy = 0.0;
    for (f, v) in freq.iter().zip(values) {
        let inside = if strict {
            *f > band.0 && *f < band.1
        } else {
            *f >= band.0 && *f <= band.1
        };
        if inside && f.is_finite() && v.is_finite() {
            n += 1.0;
            sx += f.log10();
            sy += v;
        }
    }
    if n < 2.0 {
        return (0.0, if n > 0.0 { sy / n } else { 0.0 });
    }
    let mx = sx / n;
    let my = sy / n;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    for (f, v) in freq.iter().zip(values) {
        let inside = if strict {
            *f > band.0 && *f < band.1
        } else {
            *f >= band.0 && *f <= band.1
        };
        if inside && f.is_finite() && v.is_finite() {
            let dx = f.log10() - mx;
            sxy += dx * (v - my);
            sxx += dx * dx;
        }
    }
    if sxx == 0.0 {
        return (0.0, my);
    }
    let slope = sxy / sxx;
    (slope, my - slope * mx)
}

/// Error histogram of a curve against its target-band regression.
///
/// Ports `compute_statistics`: deviation counts in the fixed bins
/// [0, .5, 1, 1.5, 2, 2.5, 3, 5] dB over the histogram band, plus the
/// slope-band slope in dB/octave and the maximum deviation.
fn eq_statistics(freq: &[f64], values: &[f64], hist_band: (f64, f64)) -> (f64, Vec<f64>, f64) {
    let (slope, intercept) = linregress_band(freq, values, (20.0, 20_000.0), true);
    let bins = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 5.0];
    let mut counts = vec![0.0; bins.len() - 1];
    let mut max_dev = 0.0_f64;
    for (f, v) in freq.iter().zip(values) {
        if *f >= hist_band.0 && *f <= hist_band.1 && f.is_finite() && v.is_finite() {
            let dev = (v - (slope * f.log10() + intercept)).abs();
            if dev > max_dev {
                max_dev = dev;
            }
            for i in 0..bins.len() - 1 {
                let in_bin = if i + 1 == bins.len() - 1 {
                    dev >= bins[i] && dev <= bins[i + 1]
                } else {
                    dev >= bins[i] && dev < bins[i + 1]
                };
                if in_bin {
                    counts[i] += 1.0;
                    break;
                }
            }
        }
    }
    // Slope in dB/octave from the slope band (compute_slope_smoothness).
    let (band_slope, _) = crate::compute::estimates::slope_smoothness(freq, values);
    (band_slope, counts, max_dev)
}

fn hist_labels() -> Vec<String> {
    // One label per np.histogram count. Python appends a dangling "5.0+"
    // category with no count (Plotly drops it); the GPUI bar renderer
    // pairs categories with counts by index, so it is omitted here.
    let bins = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 5.0];
    (0..bins.len() - 1)
        .map(|i| format!("{:.1}-{:.1}", bins[i], bins[i + 1]))
        .collect()
}

/// Linear interpolation on an ascending frequency axis (np.interp).
fn interp_at(freq: &[f64], values: &[f64], f: f64) -> f64 {
    if freq.is_empty() || values.is_empty() {
        return 0.0;
    }
    if f <= freq[0] {
        return values[0];
    }
    if f >= freq[freq.len() - 1] {
        return values[values.len() - 1];
    }
    let idx = freq.partition_point(|x| *x < f);
    let (f0, f1) = (freq[idx - 1], freq[idx]);
    let (v0, v1) = (values[idx - 1], values[idx]);
    if f1 == f0 {
        v0
    } else {
        v0 + (v1 - v0) * (f - f0) / (f1 - f0)
    }
}

fn logspace_20_20k(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| 20.0 * 1000f64.powf(i as f64 / (n - 1) as f64))
        .collect()
}

/// Builds the AutoEQ `filters_*` figure family for one measurement.
///
/// Ports `graph_results`/`print_auto_graphs_seq` of Python
/// `src/autoeq/auto_plot.py`: filter responses, PEQ vs target, spin
/// without/with EQ, per-curve regression sheets, deviation histograms,
/// and the combined `filters_auto_eq` subplot sheet. The target is the
/// speaker-linearization ideal (the curve's own slope-band regression
/// line), as in AutoEQ, not a room curve.
#[must_use]
pub fn eq_filter_figures(
    cea: &Cea2034,
    cea_eq: &Cea2034,
    peq: &crate::filters::biquad::Peq,
    speaker: &str,
) -> Vec<Figure> {
    use crate::filters::biquad::peq_response_db;
    let freq = &cea.freq;
    let n_full = freq.len().max(200);
    let full_freq = logspace_20_20k(n_full);

    // filters_eq: one trace per filter plus the total response.
    let mut eq_traces: Vec<Trace> = peq
        .iter()
        .enumerate()
        .map(|(i, (weight, biquad))| {
            let single: crate::filters::biquad::Peq = vec![(*weight, biquad.clone())];
            Trace::with_x(
                format!(
                    "{}{}: {:.0}Hz",
                    biquad.filter_type.short_name(),
                    i,
                    biquad.freq
                ),
                full_freq.clone(),
                peq_response_db(&full_freq, &single),
                Axis::Primary,
            )
            .color(PLOTLY_COLORS[i % PLOTLY_COLORS.len()])
        })
        .collect();
    eq_traces.push(
        Trace::with_x(
            "autoEQ",
            full_freq.clone(),
            peq_response_db(&full_freq, peq),
            Axis::Primary,
        )
        .color(PLOTLY_COLORS[1]),
    );

    // filters_eq_full: PEQ against the On Axis ideal (its regression).
    let (t_slope, t_intercept) = linregress_band(freq, &cea.on_axis, SLOPE_BAND, false);
    let ideal: Vec<f64> = freq
        .iter()
        .map(|f| t_slope * f.log10() + t_intercept)
        .collect();
    let error: Vec<f64> = ideal.iter().zip(&cea.on_axis).map(|(i, v)| i - v).collect();
    let interp = |values: &[f64]| -> Vec<f64> {
        full_freq
            .iter()
            .map(|f| interp_at(freq, values, *f))
            .collect()
    };
    let eq_full_traces = vec![
        Trace::with_x(
            "autoEQ",
            full_freq.clone(),
            peq_response_db(&full_freq, peq),
            Axis::Primary,
        )
        .color(PLOTLY_COLORS[1]),
        Trace::with_x("error ON", full_freq.clone(), interp(&error), Axis::Primary)
            .color(PLOTLY_COLORS[2]),
        Trace::with_x("ideal ON", full_freq.clone(), interp(&ideal), Axis::Primary)
            .color(PLOTLY_COLORS[3]),
    ];

    let mut figures = vec![
        Figure::scatter("filters_eq", "PEQ details", full_freq.clone(), eq_traces),
        Figure::scatter(
            "filters_eq_full",
            "PEQ v.s. Target",
            full_freq.clone(),
            eq_full_traces,
        ),
        Figure::scatter(
            "filters_spin_noeq",
            format!("{speaker} spinorama"),
            freq.clone(),
            cea_traces(cea),
        ),
        Figure::scatter(
            "filters_spin_auto",
            format!("{speaker} spinorama with EQ"),
            freq.clone(),
            cea_traces(cea_eq),
        ),
    ];

    // Per-curve regression sheets and histograms.
    let curves: [(&str, &str, &[f64], &[f64]); 3] = [
        ("on", "On Axis", &cea.on_axis, &cea_eq.on_axis),
        (
            "lw",
            "Listening Window",
            &cea.listening_window,
            &cea_eq.listening_window,
        ),
        (
            "pir",
            "Estimated In-Room Response",
            &cea.estimated_inroom,
            &cea_eq.estimated_inroom,
        ),
    ];
    let mut cells: Vec<SubplotCell> = vec![
        SubplotCell {
            title: "PEQ details".to_owned(),
            traces: figures[0].traces.clone(),
            bars: Vec::new(),
            log_x: true,
        },
        SubplotCell {
            title: "PEQ v.s. Target".to_owned(),
            traces: figures[1].traces.clone(),
            bars: Vec::new(),
            log_x: true,
        },
        SubplotCell {
            title: "Spinorama (noEQ)".to_owned(),
            traces: figures[2].traces.clone(),
            bars: Vec::new(),
            log_x: true,
        },
        SubplotCell {
            title: "Spinorama (autoEQ)".to_owned(),
            traces: figures[3].traces.clone(),
            bars: Vec::new(),
            log_x: true,
        },
    ];
    for (short, name, base, eq) in curves {
        let (slope_noeq, counts_noeq_full, max_noeq) = eq_statistics(freq, base, (20.0, 20_000.0));
        let (slope_auto, counts_auto_full, max_auto) = eq_statistics(freq, eq, (20.0, 20_000.0));
        let (_, counts_mid_noeq, _) = eq_statistics(freq, base, MIDRANGE_BAND);
        let (_, counts_mid_auto, _) = eq_statistics(freq, eq, MIDRANGE_BAND);

        let fig_noeq = Figure::scatter(
            format!("filters_{short}_noeq"),
            format!("{name} slope {slope_noeq:+.2}dB/Octave error max={max_noeq:.1}dB"),
            freq.clone(),
            regression_traces(freq, base, name, None),
        );
        let fig_auto = Figure::scatter(
            format!("filters_{short}_auto"),
            format!("{name} with EQ slope {slope_auto:+.2}dB/Octave error max={max_auto:.1}dB"),
            freq.clone(),
            regression_traces(freq, eq, name, None),
        );
        let labels = hist_labels();
        let bar_fig =
            |stem: &str, title: &str, counts_noeq: &[f64], counts_auto: &[f64]| -> Figure {
                Figure {
                    name: stem.to_owned(),
                    title: title.to_owned(),
                    freq: Vec::new(),
                    traces: Vec::new(),
                    kind: FigureKind::Bar(vec![
                        BarTrace {
                            name: "noEQ".to_owned(),
                            categories: labels.clone(),
                            values: counts_noeq.to_vec(),
                            color: Some(PLOTLY_COLORS[0].to_owned()),
                        },
                        BarTrace {
                            name: "autoEQ".to_owned(),
                            categories: labels.clone(),
                            values: counts_auto.to_vec(),
                            color: Some(PLOTLY_COLORS[1].to_owned()),
                        },
                    ]),
                    x_tickvals: None,
                    y_tickvals: None,
                }
            };
        let hist_full = bar_fig(
            &format!("filters_{short}_hist_fullrange"),
            &format!("Histogram of errors (fullrange) for {name}"),
            &counts_noeq_full,
            &counts_auto_full,
        );
        let hist_mid = bar_fig(
            &format!("filters_{short}_hist_midrange"),
            &format!("Histogram of errors (midrange) for {name}"),
            &counts_mid_noeq,
            &counts_mid_auto,
        );

        cells.push(SubplotCell {
            title: fig_noeq.title.clone(),
            traces: fig_noeq.traces.clone(),
            bars: Vec::new(),
            log_x: true,
        });
        cells.push(SubplotCell {
            title: fig_auto.title.clone(),
            traces: fig_auto.traces.clone(),
            bars: Vec::new(),
            log_x: true,
        });
        cells.push(SubplotCell {
            title: hist_full.title.clone(),
            traces: Vec::new(),
            bars: match &hist_full.kind {
                FigureKind::Bar(bars) => bars.clone(),
                _ => Vec::new(),
            },
            log_x: false,
        });
        cells.push(SubplotCell {
            title: hist_mid.title.clone(),
            traces: Vec::new(),
            bars: match &hist_mid.kind {
                FigureKind::Bar(bars) => bars.clone(),
                _ => Vec::new(),
            },
            log_x: false,
        });

        figures.push(fig_noeq);
        figures.push(fig_auto);
        figures.push(hist_full);
        figures.push(hist_mid);
    }

    figures.push(Figure {
        name: "filters_auto_eq".to_owned(),
        title: format!("{speaker} (autoEQ)"),
        freq: full_freq,
        traces: Vec::new(),
        kind: FigureKind::Subplot(cells),
        x_tickvals: None,
        y_tickvals: None,
    });
    figures
}
