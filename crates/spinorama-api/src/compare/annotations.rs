//! Server-side fills and original collision-solver annotation callouts.

use super::*;

pub(super) fn fill_zone(img: &mut RgbImage, polygon: &[(f64, f64)], color: Rgb<u8>, opacity: f64) {
    if polygon.len() < 3 {
        return;
    }
    let lo = polygon
        .iter()
        .map(|p| p.1)
        .fold(f64::INFINITY, f64::min)
        .max(0.0)
        .ceil() as u32;
    let hi = polygon
        .iter()
        .map(|p| p.1)
        .fold(f64::NEG_INFINITY, f64::max)
        .min(f64::from(img.height() - 1))
        .floor() as u32;
    let alpha = opacity.clamp(0.0, 1.0);
    for y in lo..=hi {
        let mut xs = Vec::new();
        for index in 0..polygon.len() {
            let (a, b) = (polygon[index], polygon[(index + 1) % polygon.len()]);
            let yf = f64::from(y);
            if (a.1 <= yf && yf < b.1) || (b.1 <= yf && yf < a.1) {
                xs.push(a.0 + (yf - a.1) / (b.1 - a.1) * (b.0 - a.0));
            }
        }
        xs.sort_by(f64::total_cmp);
        for pair in xs.as_chunks::<2>().0 {
            let x0 = pair[0].max(0.0).ceil() as u32;
            let x1 = pair[1].min(f64::from(img.width() - 1)).floor() as u32;
            for x in x0..=x1 {
                for (channel, target) in img.get_pixel_mut(x, y).0.iter_mut().zip(color.0) {
                    *channel = (f64::from(*channel) * (1.0 - alpha) + f64::from(target) * alpha)
                        .round() as u8;
                }
            }
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "drawing uses the graph's existing axis geometry"
)]
pub(super) fn draw_annotations(
    img: &mut RgbImage,
    font: &Font,
    bundle: &Bundle,
    opts: &RenderOptions,
    plot: (f64, f64, f64, f64),
    xr: (f64, f64),
    yr: (f64, f64),
    y2r: (f64, f64),
    font_px: f32,
) {
    if bundle.annotation_requests.is_empty() {
        return;
    }
    let x_of = |x: f64| plot.0 + (x - xr.0) / (xr.1 - xr.0) * (plot.2 - plot.0);
    let y_of = |y: f64, range: (f64, f64)| {
        plot.3 - (y - range.0) / (range.1 - range.0) * (plot.3 - plot.1)
    };
    let requests: Vec<_> = bundle
        .annotation_requests
        .iter()
        .filter(|r| {
            let range = if r.3 == "y2" { y2r } else { yr };
            r.1 >= xr.0 && r.1 <= xr.1 && r.2 >= range.0 && r.2 <= range.1
        })
        .cloned()
        .collect();
    let freqs = bundle.freqs();
    let segments = bundle
        .visible_traces(opts, false)
        .iter()
        .flat_map(|t| {
            let range = if t.axis == "y2" { y2r } else { yr };
            freqs
                .windows(2)
                .zip(t.values.windows(2))
                .filter(|(_, y)| y.iter().all(|v| v.is_finite()))
                .map(|(f, y)| {
                    (
                        x_of(f[0].log10()),
                        y_of(y[0], range),
                        x_of(f[1].log10()),
                        y_of(y[1], range),
                        t.axis.clone(),
                        Some(t.name.clone()),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let padding = 5.0 * f64::from(SS);
    let placements = annotations_rust::place_annotations(
        requests.clone(),
        f64::from(img.width()),
        f64::from(img.height()),
        (
            plot.0,
            f64::from(img.width()) - plot.2,
            plot.1,
            f64::from(img.height()) - plot.3,
        ),
        xr,
        vec![("y".into(), yr.0, yr.1), ("y2".into(), y2r.0, y2r.1)],
        true,
        f64::from(font_px),
        padding,
        (0.0, 1.0),
        (0.0, 1.0),
        vec![100f64.log10(), 1000f64.log10(), 10000f64.log10()],
        vec![
            ("y".into(), nice_ticks(yr.0, yr.1)),
            ("y2".into(), nice_ticks(y2r.0, y2r.1)),
        ],
        Vec::new(),
        Vec::new(),
        segments,
    );
    let fallback = requests
        .iter()
        .zip(&placements)
        .any(|(r, (center, hidden))| {
            let range = if r.3 == "y2" { y2r } else { yr };
            *hidden
                || center.is_none_or(|c| {
                    (c.0 - x_of(r.1)).hypot(c.1 - y_of(r.2, range)) > 260.0 * f64::from(SS)
                })
        });
    for (r, (center, _)) in requests.iter().zip(placements) {
        let range = if r.3 == "y2" { y2r } else { yr };
        let anchor = (x_of(r.1), y_of(r.2, range));
        let size = (
            text_width(font, &r.4, font_px) + padding * 2.0 + 2.0,
            f64::from(font_px) * 1.35 + padding * 2.0 + 2.0,
        );
        let center = if fallback {
            let direction = if r.7.as_deref() == Some("above") {
                -1.0
            } else {
                1.0
            };
            (
                anchor.0 - size.0 / 2.0,
                anchor.1 + direction * (20.0 * f64::from(SS) + size.1 / 2.0),
            )
        } else {
            center.expect("complete placement checked")
        };
        let center = (
            center.0.clamp(plot.0 + size.0 / 2.0, plot.2 - size.0 / 2.0),
            center.1.clamp(plot.1 + size.1 / 2.0, plot.3 - size.1 / 2.0),
        );
        let (dx, dy) = (anchor.0 - center.0, anchor.1 - center.1);
        let edge_t = ((size.0 / 2.0) / dx.abs())
            .min((size.1 / 2.0) / dy.abs())
            .min(1.0);
        let start = (center.0 + dx * edge_t, center.1 + dy * edge_t);
        draw_segment(img, start, anchor, SINGLE_COLOR, f64::from(SS));
        let length = dx.hypot(dy).max(1.0);
        let (ux, uy) = (dx / length, dy / length);
        let head = 6.0 * f64::from(SS);
        fill_polygon(
            img,
            &[
                anchor,
                (
                    anchor.0 - ux * head - uy * head / 2.0,
                    anchor.1 - uy * head + ux * head / 2.0,
                ),
                (
                    anchor.0 - ux * head + uy * head / 2.0,
                    anchor.1 - uy * head - ux * head / 2.0,
                ),
            ],
            SINGLE_COLOR,
        );
        let (l, t, right, b) = (
            center.0 - size.0 / 2.0,
            center.1 - size.1 / 2.0,
            center.0 + size.0 / 2.0,
            center.1 + size.1 / 2.0,
        );
        fill_zone(
            img,
            &[(l, t), (right, t), (right, b), (l, b)],
            Rgb([255, 255, 255]),
            0.86,
        );
        for (a, b) in [
            ((l, t), (right, t)),
            ((right, t), (right, b)),
            ((right, b), (l, b)),
            ((l, b), (l, t)),
        ] {
            draw_segment(img, a, b, SINGLE_COLOR, f64::from(SS));
        }
        draw_text(
            img,
            font,
            &r.4,
            font_px,
            l + padding,
            centered_baseline(font, &r.4, font_px, center.1),
            SINGLE_COLOR,
        );
    }
}
