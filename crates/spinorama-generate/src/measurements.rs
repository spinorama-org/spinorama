//! Wide horizontal/vertical SPL measurement tables.

// Rust guideline compliant 2026-02-21

use anyhow::{Context, Result, bail};

/// One SPL plane: a frequency axis plus one column per measured angle.
///
/// Mirrors the wide `h_spl` / `v_spl` DataFrames of the Python pipeline:
/// column names are `On Axis`, `10°`, `-10°`, …, `180°`.
#[derive(Debug, Clone, PartialEq)]
pub struct SplPlane {
    /// Frequency axis in hertz, strictly increasing.
    pub freq: Vec<f64>,
    /// Angle columns as `(name, spl_db)` pairs aligned with `freq`.
    pub columns: Vec<(String, Vec<f64>)>,
    /// Optional on-axis phase in degrees, aligned with `freq`.
    pub phase_on_axis: Option<Vec<f64>>,
}

impl SplPlane {
    /// Validates shape invariants and returns the plane.
    ///
    /// # Errors
    ///
    /// Returns an error when a column length differs from the frequency
    /// axis or when fewer than two frequency points are present.
    pub fn new(freq: Vec<f64>, columns: Vec<(String, Vec<f64>)>) -> Result<Self> {
        if freq.len() < 2 {
            bail!(
                "SPL plane needs at least 2 frequency points, got {}",
                freq.len()
            );
        }
        for (name, values) in &columns {
            if values.len() != freq.len() {
                bail!(
                    "column {name} has {} points for {} frequencies",
                    values.len(),
                    freq.len()
                );
            }
        }
        Ok(Self {
            freq,
            columns,
            phase_on_axis: None,
        })
    }

    /// Returns the SPL column named `name`, when present.
    #[must_use]
    pub fn column(&self, name: &str) -> Option<&[f64]> {
        self.columns
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_slice())
    }

    /// Returns the SPL column named `name`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the missing column. Callers treat a missing
    /// required angle as a failed measurement, matching the Python
    /// `KeyError` drop behaviour at the whole-version level.
    pub fn require(&self, name: &str) -> Result<&[f64]> {
        self.column(name)
            .with_context(|| format!("missing SPL column {name}"))
    }

    /// Subtracts `offset_db` from every SPL value in place.
    pub fn shift(&mut self, offset_db: f64) {
        for (_, values) in &mut self.columns {
            for value in values.iter_mut() {
                *value -= offset_db;
            }
        }
    }

    /// Attaches on-axis phase values to this plane.
    ///
    /// # Errors
    ///
    /// Returns an error when the phase length differs from the frequency axis.
    pub fn with_phase(mut self, phase_on_axis: Vec<f64>) -> Result<Self> {
        if phase_on_axis.len() != self.freq.len() {
            bail!(
                "phase has {} points for {} frequencies",
                phase_on_axis.len(),
                self.freq.len()
            );
        }
        self.phase_on_axis = Some(phase_on_axis);
        Ok(self)
    }

    /// Returns a copy with every SPL column relative to `On Axis`.
    ///
    /// # Errors
    ///
    /// Returns an error when the `On Axis` column is missing.
    pub fn normalized(&self) -> Result<Self> {
        let on_axis = self.require("On Axis")?;
        let columns = self
            .columns
            .iter()
            .map(|(name, values)| {
                (
                    name.clone(),
                    values
                        .iter()
                        .zip(on_axis)
                        .map(|(value, reference)| value - reference)
                        .collect(),
                )
            })
            .collect();
        Ok(Self {
            freq: self.freq.clone(),
            columns,
            phase_on_axis: self.phase_on_axis.clone(),
        })
    }

    /// Returns a copy resampled onto `freq` in log-frequency.
    #[must_use]
    pub fn resampled(&self, freq: &[f64]) -> Self {
        if self.freq == freq {
            return self.clone();
        }
        Self {
            freq: freq.to_vec(),
            columns: self
                .columns
                .iter()
                .map(|(name, values)| (name.clone(), resample_log_freq(freq, &self.freq, values)))
                .collect(),
            phase_on_axis: self
                .phase_on_axis
                .as_ref()
                .map(|phase| resample_log_freq(freq, &self.freq, phase)),
        }
    }

    /// Sorts angle columns in the Python display order.
    ///
    /// Negative angles run from `-180°` upward, then `On Axis`, then positive
    /// angles. Unknown column names keep their relative order at the end.
    pub fn sort_by_angle(&mut self) {
        self.columns.sort_by(|(a, _), (b, _)| {
            angle_sort_key(a)
                .partial_cmp(&angle_sort_key(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
}

/// Parses an angle column name into degrees.
///
/// `On Axis` is zero; names such as `-30°` parse as signed degrees.
#[must_use]
pub fn angle_degrees(name: &str) -> Option<i32> {
    if name == "On Axis" {
        return Some(0);
    }
    name.strip_suffix('°')?.parse().ok()
}

fn angle_sort_key(name: &str) -> (i32, i32) {
    match angle_degrees(name) {
        Some(angle) => (0, angle),
        None => (1, 0),
    }
}

/// Resamples values onto `freq` by linear-frequency interpolation.
///
/// Values outside the source range are linearly extrapolated from the two
/// nearest edge points, matching pandas `slinear` with extrapolation when
/// unifying partial CEA2034 curve sets.
#[must_use]
pub fn resample_linear_freq(freq: &[f64], src_freq: &[f64], src_values: &[f64]) -> Vec<f64> {
    if src_freq.is_empty() || src_values.len() != src_freq.len() {
        return vec![f64::NAN; freq.len()];
    }
    let extrapolate = |target: f64, x0: f64, x1: f64, y0: f64, y1: f64| {
        if x1.partial_cmp(&x0) != Some(std::cmp::Ordering::Greater) {
            return y0;
        }
        y0 + (target - x0) / (x1 - x0) * (y1 - y0)
    };
    freq.iter()
        .map(|target| {
            if *target <= src_freq[0] {
                if src_freq.len() >= 2 {
                    return extrapolate(
                        *target,
                        src_freq[0],
                        src_freq[1],
                        src_values[0],
                        src_values[1],
                    );
                }
                return src_values[0];
            }
            if *target >= src_freq[src_freq.len() - 1] {
                let last = src_freq.len() - 1;
                if last >= 1 {
                    return extrapolate(
                        *target,
                        src_freq[last - 1],
                        src_freq[last],
                        src_values[last - 1],
                        src_values[last],
                    );
                }
                return src_values[last];
            }
            let upper = src_freq.partition_point(|f| *f < *target);
            let (x0, x1) = (src_freq[upper - 1], src_freq[upper]);
            if x1.partial_cmp(&x0) != Some(std::cmp::Ordering::Greater) {
                return src_values[upper - 1];
            }
            let t = (target - x0) / (x1 - x0);
            src_values[upper - 1] + t * (src_values[upper] - src_values[upper - 1])
        })
        .collect()
}

/// Resamples values onto `freq` by log-frequency interpolation.
///
/// Source frequencies must be increasing after sorting by the caller. Values
/// outside the source range clamp to the nearest edge, which matches the
/// loaders' need to align angle files that share nearly identical grids.
#[must_use]
pub fn resample_log_freq(freq: &[f64], src_freq: &[f64], src_values: &[f64]) -> Vec<f64> {
    if src_freq.is_empty() || src_values.len() != src_freq.len() {
        return vec![f64::NAN; freq.len()];
    }
    freq.iter()
        .map(|target| {
            if *target <= src_freq[0] {
                return src_values[0];
            }
            if *target >= src_freq[src_freq.len() - 1] {
                return src_values[src_values.len() - 1];
            }
            let upper = src_freq.partition_point(|f| *f < *target);
            let (x0, x1) = (src_freq[upper - 1].ln(), src_freq[upper].ln());
            if x1.partial_cmp(&x0) != Some(std::cmp::Ordering::Greater) {
                return src_values[upper - 1];
            }
            let t = (target.ln() - x0) / (x1 - x0);
            src_values[upper - 1] + t * (src_values[upper] - src_values[upper - 1])
        })
        .collect()
}

/// Horizontal and vertical SPL planes for one measurement version.
#[derive(Debug, Clone, PartialEq)]
pub struct HvSpl {
    /// Horizontal plane.
    pub horizontal: SplPlane,
    /// Vertical plane.
    pub vertical: SplPlane,
}

/// Applies metadata symmetry to HV planes.
///
/// Mirrors Python `symmetrise_speaker_measurements`: missing opposite-sign
/// angles are copied from the measured side for the selected plane(s).
///
/// # Errors
///
/// Returns an error for an unknown symmetry name.
pub fn symmetrise_hv(hv: &HvSpl, symmetry: Option<&str>) -> Result<HvSpl> {
    let Some(symmetry) = symmetry.map(str::to_lowercase) else {
        return Ok(hv.clone());
    };
    match symmetry.as_str() {
        "none" => Ok(hv.clone()),
        "horizontal" => Ok(HvSpl {
            horizontal: mirrored_plane(&hv.horizontal),
            vertical: hv.vertical.clone(),
        }),
        "vertical" => Ok(HvSpl {
            horizontal: hv.horizontal.clone(),
            vertical: mirrored_plane(&hv.vertical),
        }),
        "coaxial" => Ok(HvSpl {
            horizontal: mirrored_plane(&hv.horizontal),
            vertical: mirrored_plane(&hv.vertical),
        }),
        other => bail!("unknown symmetry: {other}"),
    }
}

fn mirrored_plane(plane: &SplPlane) -> SplPlane {
    let mut mirrored = plane.clone();
    let existing: std::collections::BTreeSet<String> =
        plane.columns.iter().map(|(name, _)| name.clone()).collect();
    for (name, values) in &plane.columns {
        let Some(angle) = angle_degrees(name) else {
            continue;
        };
        if angle == 0 || angle == 180 {
            continue;
        }
        let mirror = if angle == -180 {
            "180°".to_owned()
        } else {
            format!("{}°", -angle)
        };
        if !existing.contains(&mirror) {
            mirrored.columns.push((mirror, values.clone()));
        }
    }
    mirrored.sort_by_angle();
    mirrored
}
