//! CEA2034 (spinorama) curves from horizontal/vertical SPL planes.
//!
//! Port of `spinorama/src/spinorama/compute/cea2034.py`: 105 dB pressure
//! reference, spherical-area sound-power weights, plain pressure RMS for
//! the listening window and early reflections, and the 0.12/0.44/0.44
//! estimated in-room mix. Hot path is the per-frequency RMS over angle
//! columns; it runs allocation-free over column slices.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};

use crate::measurements::{HvSpl, SplPlane, resample_linear_freq};

/// SPL reference baked into the Python pressure conversion, in dB.
const SPL_REFERENCE_DB: f64 = 105.0;
/// Lower edge of the mean band used for the pre-CEA shift, in hertz.
const MEAN_MIN_HZ: f64 = 300.0;
/// Upper edge of the mean band used for the pre-CEA shift, in hertz.
const MEAN_MAX_HZ: f64 = 3000.0;

/// The seven CEA2034 curves on a shared frequency axis.
#[derive(Debug, Clone, PartialEq)]
pub struct Cea2034 {
    /// Frequency axis in hertz.
    pub freq: Vec<f64>,
    /// On-axis response.
    pub on_axis: Vec<f64>,
    /// Listening window.
    pub listening_window: Vec<f64>,
    /// Total early reflections.
    pub early_reflections: Vec<f64>,
    /// Sound power.
    pub sound_power: Vec<f64>,
    /// Early-reflections directivity index (LW − ER).
    pub early_reflections_di: Vec<f64>,
    /// Sound-power directivity index (LW − SP).
    pub sound_power_di: Vec<f64>,
    /// Estimated in-room response (predicted in-room).
    pub estimated_inroom: Vec<f64>,
    /// Mean on-axis SPL in 300–3000 Hz before shifting, in dB.
    pub sensitivity_db: f64,
}

fn spl2pressure(spl: f64) -> f64 {
    10f64.powf((spl - SPL_REFERENCE_DB) / 20.0)
}

fn pressure2spl(pressure: f64) -> f64 {
    SPL_REFERENCE_DB + 20.0 * pressure.log10()
}

/// Spherical area between four lines at `alpha_d`/`beta_d` degrees.
///
/// Port of `cea2034.py::compute_area_q`.
fn area_q(alpha_d: f64, beta_d: f64) -> f64 {
    let alpha = alpha_d.to_radians();
    let beta = beta_d.to_radians();
    let gamma = (alpha.cos() * beta.cos()).acos();
    let a = (beta.sin() / alpha.tan()).atan();
    let b = (alpha.sin() / beta.tan()).atan();
    let c = (-a.cos() * b.cos() + a.sin() * b.sin() * gamma.cos()).acos();
    4.0 * c - 2.0 * std::f64::consts::PI
}

/// Returns the ten CEA2034 spherical sound-power weights.
///
/// Port of `cea2034.py::compute_weigths`: deltas of spherical areas at
/// 5°, 15°, …, 85°, 90°, with the 90° weight doubled.
#[must_use]
pub fn sound_power_weights() -> [f64; 10] {
    let mut areas = [0.0_f64; 10];
    for (i, area) in areas.iter_mut().enumerate() {
        #[expect(
            clippy::cast_precision_loss,
            reason = "angle index fits exactly in f64"
        )]
        let angle = if i == 9 { 90.0 } else { 10.0 * i as f64 + 5.0 };
        *area = area_q(angle, angle);
    }
    let mut weights = [0.0_f64; 10];
    weights[0] = areas[0];
    for i in 1..10 {
        weights[i] = areas[i] - areas[i - 1];
    }
    weights[9] *= 2.0;
    weights
}

fn weight_for_angle(weights: &[f64; 10], name: &str) -> Option<f64> {
    if name == "On Axis" {
        return Some(weights[0]);
    }
    let degrees: i32 = name.strip_suffix('°')?.parse().ok()?;
    let ring = match degrees.unsigned_abs() {
        180 => 0,
        170 | 10 => 1,
        160 | 20 => 2,
        150 | 30 => 3,
        140 | 40 => 4,
        130 | 50 => 5,
        120 | 60 => 6,
        110 | 70 => 7,
        100 | 80 => 8,
        90 => 9,
        _ => return None,
    };
    Some(weights[ring as usize])
}

/// Plain pressure RMS over the named columns of one or two planes.
///
/// Missing selections are dropped, matching Python `spatial_average1/2`,
/// which intersects the requested selection with available columns.
fn rms_average(selections: &[(&SplPlane, &[&str])], len: usize) -> Result<Vec<f64>> {
    let mut columns: Vec<&[f64]> = Vec::new();
    for (plane, names) in selections {
        for name in *names {
            if let Some(values) = plane.column(name) {
                columns.push(values);
            }
        }
    }
    if columns.is_empty() {
        // Python spatial_average substitutes a 1e-9 pressure floor
        // when a whole angle group is absent (e.g. Princeton front
        // hemisphere has no rear angles): a constant -75 dB curve
        // under the 105 dB internal reference.
        return Ok(vec![pressure2spl(1e-9); len]);
    }
    #[expect(clippy::cast_precision_loss, reason = "column counts are tiny")]
    let n = columns.len() as f64;
    let mut out = Vec::with_capacity(len);
    for row in 0..len {
        let mean_square = columns
            .iter()
            .map(|c| spl2pressure(c[row]).powi(2))
            .sum::<f64>()
            / n;
        out.push(pressure2spl(mean_square.sqrt()));
    }
    Ok(out)
}

/// Weighted pressure RMS over every column of both planes (sound power).
///
/// The vertical `On Axis` and `180°` columns are dropped, matching
/// `cea2034.py::sound_power`; all other columns take the spherical weight
/// of their angle ring.
fn sound_power(hv: &HvSpl, len: usize) -> Result<Vec<f64>> {
    let weights = sound_power_weights();
    let mut columns: Vec<(&[f64], f64)> = Vec::new();
    for (name, values) in &hv.horizontal.columns {
        if let Some(weight) = weight_for_angle(&weights, name) {
            columns.push((values, weight));
        }
    }
    for (name, values) in &hv.vertical.columns {
        if name == "On Axis" || name == "180°" {
            continue;
        }
        if let Some(weight) = weight_for_angle(&weights, name) {
            columns.push((values, weight));
        }
    }
    if columns.is_empty() {
        bail!("sound power needs at least one weighted angle column");
    }
    let weight_sum: f64 = columns.iter().map(|(_, w)| w).sum();
    let mut out = Vec::with_capacity(len);
    for row in 0..len {
        let mean_square = columns
            .iter()
            .map(|(values, w)| w * spl2pressure(values[row]).powi(2))
            .sum::<f64>()
            / weight_sum;
        out.push(pressure2spl(mean_square.sqrt()));
    }
    Ok(out)
}

fn mean_in_band(freq: &[f64], curve: &[f64]) -> Result<f64> {
    mean_between(freq, curve, MEAN_MIN_HZ, MEAN_MAX_HZ)
}

fn mean_between(freq: &[f64], curve: &[f64], min_hz: f64, max_hz: f64) -> Result<f64> {
    let (mut sum, mut count) = (0.0_f64, 0_u64);
    for (f, v) in freq.iter().zip(curve) {
        if *f > min_hz && *f < max_hz {
            sum += v;
            count += 1;
        }
    }
    if count == 0 {
        bail!("no frequency points in {min_hz}-{max_hz} Hz mean band");
    }
    #[expect(clippy::cast_precision_loss, reason = "point counts are tiny")]
    Ok(sum / count as f64)
}

/// All derived curves needed by the HV figure set.
#[derive(Debug, Clone, PartialEq)]
pub struct SpinData {
    /// CEA2034 curves after the Python mean-band pre-shift.
    pub cea: Cea2034,
    /// CEA2034 curves recomputed from per-plane normalized SPL.
    pub cea_normalized: Cea2034,
    /// Early-reflection component curves in display order.
    pub early_reflections: Vec<(String, Vec<f64>)>,
    /// Horizontal-reflection component curves in display order.
    pub horizontal_reflections: Vec<(String, Vec<f64>)>,
    /// Vertical-reflection component curves in display order.
    pub vertical_reflections: Vec<(String, Vec<f64>)>,
    /// Shifted horizontal SPL plane.
    pub horizontal: SplPlane,
    /// Shifted vertical SPL plane.
    pub vertical: SplPlane,
    /// Horizontal SPL relative to horizontal on-axis.
    pub horizontal_normalized: SplPlane,
    /// Vertical SPL relative to vertical on-axis.
    pub vertical_normalized: SplPlane,
}

fn shifted_planes(hv: &HvSpl) -> Result<(HvSpl, f64)> {
    let mut aligned = hv.clone();
    if aligned.horizontal.freq != aligned.vertical.freq {
        aligned.vertical = aligned.vertical.resampled(&aligned.horizontal.freq);
    }
    let freq = &aligned.horizontal.freq;
    let sensitivity_db = match aligned.horizontal.column("On Axis") {
        Some(on_axis) => mean_in_band(freq, on_axis)?,
        None => mean_in_band(freq, aligned.vertical.require("On Axis")?)?,
    };
    aligned.horizontal.shift(sensitivity_db);
    aligned.vertical.shift(sensitivity_db);
    Ok((aligned, sensitivity_db))
}

fn cea_from_shifted(shifted: &HvSpl, sensitivity_db: f64) -> Result<Cea2034> {
    let freq = shifted.horizontal.freq.clone();
    let len = freq.len();
    let (h, v) = (&shifted.horizontal, &shifted.vertical);

    let on_axis = rms_average(&[(h, &["On Axis"]), (v, &["On Axis"])], len)?;
    let listening_window = rms_average(
        &[
            (h, &["10°", "20°", "30°", "-10°", "-20°", "-30°"]),
            (v, &["On Axis", "10°", "-10°"]),
        ],
        len,
    )?;
    let floor = rms_average(&[(v, &["-20°", "-30°", "-40°"])], len)?;
    let ceiling = rms_average(&[(v, &["40°", "50°", "60°"])], len)?;
    let front = rms_average(
        &[(h, &["On Axis", "10°", "20°", "30°", "-10°", "-20°", "-30°"])],
        len,
    )?;
    let side = rms_average(
        &[(
            h,
            &[
                "-40°", "-50°", "-60°", "-70°", "-80°", "40°", "50°", "60°", "70°", "80°",
            ],
        )],
        len,
    )?;
    let rear = rms_average(
        &[(
            h,
            &[
                "-170°", "-160°", "-150°", "-140°", "-130°", "-120°", "-110°", "-100°", "-90°",
                "90°", "100°", "110°", "120°", "130°", "140°", "150°", "160°", "170°", "180°",
            ],
        )],
        len,
    )?;
    let early_reflections: Vec<f64> = (0..len)
        .map(|i| {
            let mean_square = [floor[i], ceiling[i], front[i], side[i], rear[i]]
                .iter()
                .map(|spl| spl2pressure(*spl).powi(2))
                .sum::<f64>()
                / 5.0;
            pressure2spl(mean_square.sqrt())
        })
        .collect();
    let sound_power = sound_power(shifted, len)?;

    let early_reflections_di: Vec<f64> = listening_window
        .iter()
        .zip(&early_reflections)
        .map(|(lw, er)| lw - er)
        .collect();
    let sound_power_di: Vec<f64> = listening_window
        .iter()
        .zip(&sound_power)
        .map(|(lw, sp)| lw - sp)
        .collect();
    let estimated_inroom =
        estimated_inroom_from(&listening_window, &early_reflections, &sound_power);

    Ok(Cea2034 {
        freq,
        on_axis,
        listening_window,
        early_reflections,
        sound_power,
        early_reflections_di,
        sound_power_di,
        estimated_inroom,
        sensitivity_db,
    })
}

/// Computes the CEA2034 curve set from raw horizontal/vertical SPL.
///
/// Applies the Python `filter_graphs` pre-shift first: every curve is
/// shifted by the mean horizontal on-axis SPL over 300–3000 Hz, and that
/// pre-shift mean is reported as `sensitivity_db`.
///
/// # Errors
///
/// Returns an error when no on-axis curve or weighted angle column exists.
pub fn compute_cea2034(hv: &HvSpl) -> Result<Cea2034> {
    let (shifted, sensitivity_db) = shifted_planes(hv)?;
    cea_from_shifted(&shifted, sensitivity_db)
}

const FRONT: &[&str] = &["On Axis", "10°", "20°", "30°", "-10°", "-20°", "-30°"];
const SIDE: &[&str] = &[
    "-40°", "-50°", "-60°", "-70°", "-80°", "40°", "50°", "60°", "70°", "80°",
];
const REAR: &[&str] = &[
    "-170°", "-160°", "-150°", "-140°", "-130°", "-120°", "-110°", "-100°", "-90°", "90°", "100°",
    "110°", "120°", "130°", "140°", "150°", "160°", "170°", "180°",
];
const FLOOR: &[&str] = &["-20°", "-30°", "-40°"];
const CEILING: &[&str] = &["40°", "50°", "60°"];

fn pressure_rms(curves: &[&[f64]], len: usize) -> Vec<f64> {
    #[expect(clippy::cast_precision_loss, reason = "component counts are tiny")]
    let n = curves.len() as f64;
    (0..len)
        .map(|i| {
            let mean_square = curves
                .iter()
                .map(|curve| spl2pressure(curve[i]).powi(2))
                .sum::<f64>()
                / n;
            pressure2spl(mean_square.sqrt())
        })
        .collect()
}

/// Computes all derived HV curves used by the figure builders.
///
/// # Errors
///
/// Returns an error when CEA2034 or reflection averages have no inputs.
pub fn compute_spin_data(hv: &HvSpl) -> Result<SpinData> {
    let (shifted, sensitivity_db) = shifted_planes(hv)?;
    let cea = cea_from_shifted(&shifted, sensitivity_db)?;
    let horizontal_normalized = shifted.horizontal.normalized()?;
    let vertical_normalized = shifted.vertical.normalized()?;
    let cea_normalized = cea_from_shifted(
        &HvSpl {
            horizontal: horizontal_normalized.clone(),
            vertical: vertical_normalized.clone(),
        },
        sensitivity_db,
    )?;

    let len = shifted.horizontal.freq.len();
    let (h, v) = (&shifted.horizontal, &shifted.vertical);
    let floor = rms_average(&[(v, FLOOR)], len)?;
    let ceiling = rms_average(&[(v, CEILING)], len)?;
    let front = rms_average(&[(h, FRONT)], len)?;
    let side = rms_average(&[(h, SIDE)], len)?;
    let rear = rms_average(&[(h, REAR)], len)?;
    let total_early = pressure_rms(&[&floor, &ceiling, &front, &side, &rear], len);

    let total_vertical = rms_average(
        &[(v, &["On Axis", "-20°", "-30°", "-40°", "40°", "50°", "60°"])],
        len,
    )?;
    let all_horizontal: Vec<String> = h
        .columns
        .iter()
        .map(|(name, _)| name.clone())
        .filter(|name| {
            name == "On Axis"
                || crate::measurements::angle_degrees(name).is_some_and(|a| a % 10 == 0)
        })
        .collect();
    let all_horizontal_refs: Vec<&str> = all_horizontal.iter().map(String::as_str).collect();
    let total_horizontal = rms_average(&[(h, &all_horizontal_refs)], len)?;

    Ok(SpinData {
        cea,
        cea_normalized,
        early_reflections: vec![
            ("Floor Bounce".to_owned(), floor.clone()),
            ("Ceiling Bounce".to_owned(), ceiling.clone()),
            ("Front Wall Bounce".to_owned(), front.clone()),
            ("Side Wall Bounce".to_owned(), side.clone()),
            ("Rear Wall Bounce".to_owned(), rear.clone()),
            ("Total Early Reflection".to_owned(), total_early),
        ],
        horizontal_reflections: vec![
            ("Front".to_owned(), front),
            ("Side".to_owned(), side),
            ("Rear".to_owned(), rear),
            ("Total Horizontal Reflection".to_owned(), total_horizontal),
        ],
        vertical_reflections: vec![
            ("Floor Reflection".to_owned(), floor),
            ("Ceiling Reflection".to_owned(), ceiling),
            ("Total Vertical Reflection".to_owned(), total_vertical),
        ],
        horizontal: shifted.horizontal.clone(),
        vertical: shifted.vertical.clone(),
        horizontal_normalized,
        vertical_normalized,
    })
}

/// Returns the estimated in-room response from LW/ER/SP curves.
///
/// Pressure-domain mix `sqrt(0.12·LW² + 0.44·ER² + 0.44·SP²)`, the
/// same formula [`compute_cea2034`] applies. Exposed so consumers of
/// emitted CEA2034 figures (which carry no EIR trace) can rebuild the
/// curve identically.
#[must_use]
pub fn estimated_inroom_from(
    listening_window: &[f64],
    early_reflections: &[f64],
    sound_power: &[f64],
) -> Vec<f64> {
    listening_window
        .iter()
        .zip(early_reflections)
        .zip(sound_power)
        .map(|((lw, er), sp)| {
            pressure2spl(
                (0.12 * spl2pressure(*lw).powi(2)
                    + 0.44 * spl2pressure(*er).powi(2)
                    + 0.44 * spl2pressure(*sp).powi(2))
                .sqrt(),
            )
        })
        .collect()
}

/// Builds a CEA2034 set from already measured spin curves.
///
/// This is the partial-measurement path used by curve loaders: mandatory
/// `On Axis`, `Listening Window`, `Early Reflections`, and `Sound Power`
/// curves are unified onto the union frequency grid, DI curves and EIR are
/// recomputed, and all SPL curves are shifted by the 300–5000 Hz mean.
///
/// # Errors
///
/// Returns an error when a mandatory curve is missing or empty.
pub fn cea2034_from_curves(curves: &BTreeMap<String, (Vec<f64>, Vec<f64>)>) -> Result<Cea2034> {
    let get = |name: &str| -> Result<&(Vec<f64>, Vec<f64>)> {
        curves
            .get(name)
            .with_context(|| format!("partial CEA2034 input is missing curve {name}"))
    };
    let mandatory = [
        get("On Axis")?,
        get("Listening Window")?,
        get("Early Reflections")?,
        get("Sound Power")?,
    ];
    let mut freq: Vec<f64> = mandatory
        .iter()
        .flat_map(|(freq, _)| freq.iter().copied())
        .filter(|value| value.is_finite() && *value > 0.0)
        .collect();
    freq.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    freq.dedup();
    if freq.len() < 2 {
        bail!("partial CEA2034 input has no usable frequency grid");
    }

    let mut unified = mandatory
        .iter()
        .map(|(src_freq, src_spl)| resample_linear_freq(&freq, src_freq, src_spl));
    let on_raw = unified.next().unwrap_or_default();
    let lw_raw = unified.next().unwrap_or_default();
    let er_raw = unified.next().unwrap_or_default();
    let sp_raw = unified.next().unwrap_or_default();

    // Python filter_graphs_partial iterates On Axis then Listening Window
    // without breaking, so Listening Window wins when both are present.
    let sensitivity_db = mean_in_band(&freq, &lw_raw)
        .or_else(|_| mean_in_band(&freq, &on_raw))
        .unwrap_or(0.0);
    let shift = mean_between(&freq, &lw_raw, 300.0, 5000.0)
        .or_else(|_| mean_between(&freq, &on_raw, 300.0, 5000.0))
        .unwrap_or(0.0);
    let shift_curve = |curve: &[f64]| curve.iter().map(|value| value - shift).collect::<Vec<_>>();
    let on_axis = shift_curve(&on_raw);
    let listening_window = shift_curve(&lw_raw);
    let early_reflections = shift_curve(&er_raw);
    let sound_power = shift_curve(&sp_raw);
    let early_reflections_di: Vec<f64> = listening_window
        .iter()
        .zip(&early_reflections)
        .map(|(lw, er)| lw - er)
        .collect();
    let sound_power_di: Vec<f64> = listening_window
        .iter()
        .zip(&sound_power)
        .map(|(lw, sp)| lw - sp)
        .collect();
    let estimated_inroom =
        estimated_inroom_from(&listening_window, &early_reflections, &sound_power);

    Ok(Cea2034 {
        freq,
        on_axis,
        listening_window,
        early_reflections,
        sound_power,
        early_reflections_di,
        sound_power_di,
        estimated_inroom,
        sensitivity_db,
    })
}
