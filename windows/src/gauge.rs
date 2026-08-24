//! Draws the tray indicator, and owns every colour in the app.
//!
//! Port of `menubar/Sources/ClaudeQuotaBar/UsageGauge.swift`. macOS draws into
//! an `NSImage` with Core Graphics and hands it to `MenuBarExtra`; Windows wants
//! a raw RGBA buffer for `Shell_NotifyIcon`, so this rasterizes with `tiny-skia`
//! and hands back the pixels.
//!
//! The colour rules live here and nowhere else, so the tray icon and the popover
//! bars can never disagree.

use tiny_skia::{Color, FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::settings::{ColorTheme, IndicatorStyle};

/// Claude's usage-screen blue. Fixed rather than the system accent colour: it
/// is meant to look like Claude, not like whatever accent the user picked.
/// Saturated enough to hold up on both light and dark taskbars.
pub const CLAUDE_BLUE: [u8; 3] = [62, 99, 221];

const GREEN: [u8; 3] = [52, 199, 89];
const YELLOW: [u8; 3] = [255, 204, 0];
const ORANGE: [u8; 3] = [255, 149, 0];
const RED: [u8; 3] = [255, 59, 48];

/// The unfilled track, and the "no reading yet" dash.
///
/// macOS can use `tertiaryLabelColor` and let the system resolve it against the
/// menu bar. A tray icon is composited onto the taskbar with no tinting at all,
/// and the taskbar is dark under the default theme and light under the light
/// one, so the track has to be a mid grey that reads on both.
const TRACK: [u8; 4] = [128, 128, 128, 140];

pub fn rgb(percentage: f64, theme: ColorTheme) -> [u8; 3] {
    match theme {
        ColorTheme::Claude => CLAUDE_BLUE,
        ColorTheme::Usage => match percentage {
            p if p < 50.0 => GREEN,
            p if p < 80.0 => YELLOW,
            p if p < 95.0 => ORANGE,
            _ => RED,
        },
    }
}

fn color(percentage: f64, theme: ColorTheme, stale: bool) -> Color {
    let [r, g, b] = rgb(percentage, theme);
    let alpha = if stale { 0.45 } else { 1.0 };
    Color::from_rgba8(r, g, b, (alpha * 255.0) as u8)
}

fn track_color() -> Color {
    Color::from_rgba8(TRACK[0], TRACK[1], TRACK[2], TRACK[3])
}

/// A finished tray icon: RGBA8, `size` x `size`.
pub struct Icon {
    pub rgba: Vec<u8>,
    pub size: u32,
}

/// `size` is the tray icon size Windows asked for -- 16 logical pixels scaled by
/// the system DPI, so 16 at 100% and 32 at 200%.
pub fn render(
    percentage: Option<f64>,
    stale: bool,
    style: IndicatorStyle,
    theme: ColorTheme,
    size: u32,
) -> Icon {
    let size = size.clamp(16, 256);
    let mut pixmap = Pixmap::new(size, size).expect("a non-zero tray icon size");
    let scale = size as f32 / 16.0;

    match style {
        IndicatorStyle::Ring => ring(&mut pixmap, percentage, stale, theme, scale),
        IndicatorStyle::Bar => bar(&mut pixmap, percentage, stale, theme, scale),
        IndicatorStyle::Number => number(&mut pixmap, percentage, stale, theme, scale),
    }

    Icon {
        // Straight alpha, not premultiplied. `Pixmap` stores premultiplied
        // pixels and `Shell_NotifyIcon` expects the other kind: handing over
        // the raw buffer darkens every antialiased edge in proportion to how
        // transparent it is, which is most of a ring's outline.
        rgba: pixmap
            .pixels()
            .iter()
            .flat_map(|pixel| {
                let color = pixel.demultiply();
                [color.red(), color.green(), color.blue(), color.alpha()]
            })
            .collect(),
        size,
    }
}

fn ring(pixmap: &mut Pixmap, percentage: Option<f64>, stale: bool, theme: ColorTheme, scale: f32) {
    let center = 8.0 * scale;
    let radius = 6.5 * scale;
    let line_width = 2.5 * scale;

    let mut stroke = Stroke {
        width: line_width,
        ..Stroke::default()
    };
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };

    paint.set_color(track_color());
    if let Some(path) = arc(center, center, radius, 0.0, std::f32::consts::TAU) {
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }

    let Some(percentage) = percentage else {
        // Unknown state: a dash inside an empty ring.
        let mut builder = PathBuilder::new();
        builder.move_to(center - 3.0 * scale, center);
        builder.line_to(center + 3.0 * scale, center);
        if let Some(path) = builder.finish() {
            stroke.width = 1.5 * scale;
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
        return;
    };

    let clamped = percentage.clamp(0.0, 100.0);
    if clamped <= 0.0 {
        return;
    }

    // Start at 12 o'clock and sweep clockwise. Screen coordinates put y
    // downwards, so "up" is -90 degrees and clockwise is increasing.
    let start = -std::f32::consts::FRAC_PI_2;
    let sweep = (clamped / 100.0) as f32 * std::f32::consts::TAU;

    paint.set_color(color(clamped, theme, stale));
    stroke.width = line_width;
    stroke.line_cap = LineCap::Round;
    if let Some(path) = arc(center, center, radius, start, sweep) {
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// The capsule from Claude's usage screen, shrunk to tray size.
///
/// macOS gets a 24x16 label; a tray icon is square, so the capsule spans the
/// full width and the height carries the difference.
fn bar(pixmap: &mut Pixmap, percentage: Option<f64>, stale: bool, theme: ColorTheme, scale: f32) {
    let height = 6.0 * scale;
    let width = 15.0 * scale;
    let left = 0.5 * scale;
    let top = (16.0 * scale - height) / 2.0;
    let radius = height / 2.0;

    let clamped = percentage.map(|p| p.clamp(0.0, 100.0));
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };

    let track = match clamped {
        Some(clamped) if clamped > 0.0 => match theme {
            ColorTheme::Claude => {
                let [r, g, b] = rgb(clamped, theme);
                Color::from_rgba8(r, g, b, 72)
            }
            ColorTheme::Usage => track_color(),
        },
        _ => track_color(),
    };
    paint.set_color(track);
    if let Some(path) = capsule(left, top, width, height, radius) {
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    let Some(clamped) = clamped else { return };
    if clamped <= 0.0 {
        return;
    }

    // Never let a non-zero reading round away to an invisible sliver -- a bar
    // that looks empty at 2% is worse than one that overstates it.
    let filled = (width * (clamped / 100.0) as f32).max(height);
    paint.set_color(color(clamped, theme, stale));
    if let Some(path) = capsule(left, top, filled, height, radius) {
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// The percentage, drawn into the icon.
///
/// No macOS counterpart: a `MenuBarExtra` label can hold an image *and* text, so
/// the macOS app writes "23%" beside the ring. A tray icon is a bare square
/// bitmap with no label, so the number has to go inside it or nowhere.
fn number(
    pixmap: &mut Pixmap,
    percentage: Option<f64>,
    stale: bool,
    theme: ColorTheme,
    scale: f32,
) {
    let text = match percentage {
        Some(percentage) => format!("{}", percentage.clamp(0.0, 100.0).round() as i64),
        None => "--".to_string(),
    };
    let tint = match percentage {
        Some(percentage) => color(percentage.clamp(0.0, 100.0), theme, stale),
        None => track_color(),
    };

    if text::draw_centered(pixmap, &text, tint, scale).is_none() {
        // No usable system font: a ring still tells the truth, a blank icon
        // does not.
        ring(pixmap, percentage, stale, theme, scale);
    }
}

/// An arc as cubic segments. `tiny-skia` has no arc primitive, and the usual
/// 4/3*tan(d/4) control-point formula is exact enough well below one pixel.
fn arc(cx: f32, cy: f32, radius: f32, start: f32, sweep: f32) -> Option<tiny_skia::Path> {
    let mut builder = PathBuilder::new();
    let segments = (sweep.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = sweep / segments as f32;

    let point = |angle: f32| (cx + radius * angle.cos(), cy + radius * angle.sin());
    let (x, y) = point(start);
    builder.move_to(x, y);

    let k = 4.0 / 3.0 * (step / 4.0).tan();
    for index in 0..segments {
        let a0 = start + step * index as f32;
        let a1 = a0 + step;
        let (x0, y0) = point(a0);
        let (x1, y1) = point(a1);
        builder.cubic_to(
            x0 - k * radius * a0.sin(),
            y0 + k * radius * a0.cos(),
            x1 + k * radius * a1.sin(),
            y1 - k * radius * a1.cos(),
            x1,
            y1,
        );
    }
    builder.finish()
}

fn capsule(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Option<tiny_skia::Path> {
    let radius = radius.min(width / 2.0).min(height / 2.0);
    let mut builder = PathBuilder::new();
    let (right, bottom) = (x + width, y + height);

    builder.move_to(x + radius, y);
    builder.line_to(right - radius, y);
    builder.quad_to(right, y, right, y + radius);
    builder.line_to(right, bottom - radius);
    builder.quad_to(right, bottom, right - radius, bottom);
    builder.line_to(x + radius, bottom);
    builder.quad_to(x, bottom, x, bottom - radius);
    builder.line_to(x, y + radius);
    builder.quad_to(x, y, x + radius, y);
    builder.close();
    builder.finish()
}

/// Just enough text rendering for two or three digits in a 16-pixel square.
mod text {
    use ab_glyph::{Font, FontVec, Glyph, PxScale, ScaleFont};
    use std::sync::OnceLock;
    use tiny_skia::{Color, Pixmap};

    /// Loaded from the system rather than bundled: shipping a font would mean
    /// shipping its licence, for two digits.
    fn font() -> Option<&'static FontVec> {
        static FONT: OnceLock<Option<FontVec>> = OnceLock::new();
        FONT.get_or_init(|| {
            let root = std::env::var("WINDIR").unwrap_or_else(|_| "C:/Windows".to_string());
            ["segoeui.ttf", "tahoma.ttf", "arial.ttf", "verdana.ttf"]
                .iter()
                .filter_map(|name| std::fs::read(format!("{root}/Fonts/{name}")).ok())
                .find_map(|data| FontVec::try_from_vec(data).ok())
        })
        .as_ref()
    }

    /// Returns None when there is no usable font, so the caller can fall back.
    pub fn draw_centered(pixmap: &mut Pixmap, text: &str, tint: Color, scale: f32) -> Option<()> {
        let font = font()?;
        let box_size = 16.0 * scale;

        // Fit to the box: three digits need to be smaller than two.
        let px = PxScale::from(match text.chars().count() {
            0..=2 => 13.0 * scale,
            _ => 9.5 * scale,
        });
        let scaled = font.as_scaled(px);

        let glyphs: Vec<(Glyph, f32)> = text
            .chars()
            .map(|character| {
                let glyph_id = font.glyph_id(character);
                (glyph_id.with_scale(px), scaled.h_advance(glyph_id))
            })
            .collect();

        let total: f32 = glyphs.iter().map(|(_, advance)| advance).sum();
        let mut pen = (box_size - total) / 2.0;
        // Centre on the cap height rather than the full line, which would push
        // the digits low by the descender.
        let baseline = box_size / 2.0 + scaled.ascent() * 0.36;

        let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
        let pixels = pixmap.pixels_mut();

        for (glyph, advance) in glyphs {
            let mut positioned = glyph;
            positioned.position = ab_glyph::point(pen, baseline);
            pen += advance;

            let Some(outline) = font.outline_glyph(positioned) else {
                continue;
            };
            let bounds = outline.px_bounds();
            outline.draw(|x, y, coverage| {
                if coverage <= 0.0 {
                    return;
                }
                let px = bounds.min.x as i32 + x as i32;
                let py = bounds.min.y as i32 + y as i32;
                if px < 0 || py < 0 || px >= width || py >= height {
                    return;
                }
                let alpha = (coverage.clamp(0.0, 1.0) * tint.alpha()) as f32;
                // Premultiplied, which is what `PremultipliedColorU8` wants and
                // what the shell expects in the icon bitmap.
                let channel = |value: f32| (value * alpha * 255.0).round() as u8;
                if let Some(pixel) = tiny_skia::PremultipliedColorU8::from_rgba(
                    channel(tint.red()),
                    channel(tint.green()),
                    channel(tint.blue()),
                    (alpha * 255.0).round() as u8,
                ) {
                    pixels[(py * width + px) as usize] = pixel;
                }
            });
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque_pixels(icon: &Icon) -> usize {
        icon.rgba
            .chunks_exact(4)
            .filter(|pixel| pixel[3] > 0)
            .count()
    }

    fn colored_pixels(icon: &Icon, expected: [u8; 3]) -> usize {
        // Straight alpha, so a solid pixel carries its colour verbatim however
        // faint it is. Only the near-opaque ones are counted, to keep
        // antialiased edges from being read as fill.
        icon.rgba
            .chunks_exact(4)
            .filter(|pixel| {
                pixel[3] > 229
                    && (0..3).all(|channel| {
                        (pixel[channel] as i32 - expected[channel] as i32).abs() < 12
                    })
            })
            .count()
    }

    #[test]
    fn the_buffer_uses_straight_alpha_not_premultiplied() {
        // A half-transparent pixel of a bright colour keeps that colour under
        // straight alpha, and is dragged towards black under premultiplied.
        // Getting this wrong dims every antialiased edge of the icon.
        let stale = render(
            Some(100.0),
            true,
            IndicatorStyle::Bar,
            ColorTheme::Claude,
            64,
        );
        let brightest = stale
            .rgba
            .chunks_exact(4)
            .filter(|pixel| pixel[3] > 0)
            .map(|pixel| pixel[2] as u32)
            .max()
            .unwrap_or(0);
        assert!(
            brightest >= CLAUDE_BLUE[2] as u32 - 4,
            "brightest blue was {brightest}, expected about {} -- looks premultiplied",
            CLAUDE_BLUE[2]
        );
    }

    /// Writes `docs/windows-indicators.png`: every indicator style, in both
    /// colour themes, at three fill levels.
    ///
    /// Ignored by default because it writes into the repository rather than
    /// asserting anything. Run it deliberately when the drawing changes:
    ///
    /// ```text
    /// cargo test render_the_indicator_strip -- --ignored --nocapture
    /// ```
    ///
    /// The macOS app renders its own README images the same way, off-screen
    /// from a preview model -- a screenshot of an icon this small, scaled by
    /// whatever the display happens to be, is never as honest as the pixels the
    /// shell is actually handed.
    #[test]
    #[ignore = "writes docs/windows-indicators.png; run deliberately"]
    fn render_the_indicator_strip() {
        const ICON: u32 = 48;
        const CELL: u32 = 64;

        let columns: Vec<(ColorTheme, f64)> = [ColorTheme::Claude, ColorTheme::Usage]
            .into_iter()
            .flat_map(|theme| [15.0, 62.0, 93.0].map(|percent| (theme, percent)))
            .collect();
        let rows = IndicatorStyle::ALL;

        let mut sheet =
            image::RgbaImage::new(CELL * columns.len() as u32, CELL * rows.len() as u32);
        let inset = (CELL - ICON) / 2;

        for (row, style) in rows.iter().enumerate() {
            for (column, (theme, percent)) in columns.iter().enumerate() {
                let icon = render(Some(*percent), false, *style, *theme, ICON);
                for y in 0..ICON {
                    for x in 0..ICON {
                        let at = ((y * ICON + x) * 4) as usize;
                        sheet.put_pixel(
                            column as u32 * CELL + inset + x,
                            row as u32 * CELL + inset + y,
                            image::Rgba([
                                icon.rgba[at],
                                icon.rgba[at + 1],
                                icon.rgba[at + 2],
                                icon.rgba[at + 3],
                            ]),
                        );
                    }
                }
            }
        }

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("docs")
            .join("windows-indicators.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        sheet.save(&path).unwrap();
        println!("wrote {}", path.display());
    }

    #[test]
    fn the_claude_theme_is_one_blue_at_every_level() {
        for percentage in [1.0, 49.0, 79.0, 94.0, 100.0] {
            assert_eq!(
                rgb(percentage, ColorTheme::Claude),
                CLAUDE_BLUE,
                "at {percentage}%"
            );
        }
    }

    #[test]
    fn the_usage_ramp_changes_at_the_documented_thresholds() {
        assert_eq!(rgb(0.0, ColorTheme::Usage), GREEN);
        assert_eq!(rgb(49.9, ColorTheme::Usage), GREEN);
        assert_eq!(rgb(50.0, ColorTheme::Usage), YELLOW);
        assert_eq!(rgb(79.9, ColorTheme::Usage), YELLOW);
        assert_eq!(rgb(80.0, ColorTheme::Usage), ORANGE);
        assert_eq!(rgb(94.9, ColorTheme::Usage), ORANGE);
        assert_eq!(rgb(95.0, ColorTheme::Usage), RED);
        assert_eq!(rgb(100.0, ColorTheme::Usage), RED);
    }

    #[test]
    fn every_style_renders_at_every_dpi_without_panicking() {
        for size in [16, 20, 24, 32, 40, 64] {
            for style in IndicatorStyle::ALL {
                for percentage in [None, Some(0.0), Some(23.0), Some(100.0), Some(140.0)] {
                    let icon = render(percentage, false, style, ColorTheme::Usage, size);
                    assert_eq!(icon.size, size);
                    assert_eq!(icon.rgba.len(), (size * size * 4) as usize);
                }
            }
        }
    }

    #[test]
    fn a_fuller_ring_paints_more_of_itself() {
        let sweep = |percentage| {
            colored_pixels(
                &render(
                    Some(percentage),
                    false,
                    IndicatorStyle::Ring,
                    ColorTheme::Claude,
                    64,
                ),
                CLAUDE_BLUE,
            )
        };
        assert!(sweep(25.0) > 0);
        assert!(sweep(50.0) > sweep(25.0));
        assert!(sweep(100.0) > sweep(50.0));
    }

    #[test]
    fn a_window_at_zero_draws_a_track_and_no_fill() {
        let icon = render(
            Some(0.0),
            false,
            IndicatorStyle::Ring,
            ColorTheme::Claude,
            64,
        );
        assert!(
            opaque_pixels(&icon) > 0,
            "the empty track still has to be visible"
        );
        assert_eq!(colored_pixels(&icon, CLAUDE_BLUE), 0);
    }

    #[test]
    fn no_reading_at_all_still_draws_something() {
        // An icon that vanishes when the data is missing reads as a crash.
        for style in IndicatorStyle::ALL {
            let icon = render(None, false, style, ColorTheme::Claude, 64);
            assert!(opaque_pixels(&icon) > 0, "{style:?}");
        }
    }

    #[test]
    fn a_bar_at_one_percent_is_still_visible() {
        // Rounding a non-zero reading down to nothing is the bug this guards:
        // an empty-looking bar at 1% is indistinguishable from no data.
        let icon = render(Some(1.0), false, IndicatorStyle::Bar, ColorTheme::Usage, 64);
        assert!(
            colored_pixels(&icon, GREEN) > 20,
            "{}",
            colored_pixels(&icon, GREEN)
        );
    }

    #[test]
    fn a_stale_reading_is_dimmed_rather_than_hidden() {
        let fresh = render(
            Some(60.0),
            false,
            IndicatorStyle::Ring,
            ColorTheme::Claude,
            64,
        );
        let stale = render(
            Some(60.0),
            true,
            IndicatorStyle::Ring,
            ColorTheme::Claude,
            64,
        );
        assert!(opaque_pixels(&stale) > 0);
        // Same geometry, weaker ink.
        assert!(colored_pixels(&stale, CLAUDE_BLUE) < colored_pixels(&fresh, CLAUDE_BLUE));
    }

    #[test]
    fn an_overage_reading_does_not_wrap_the_ring_past_full() {
        let full = render(
            Some(100.0),
            false,
            IndicatorStyle::Ring,
            ColorTheme::Usage,
            64,
        );
        let over = render(
            Some(140.0),
            false,
            IndicatorStyle::Ring,
            ColorTheme::Usage,
            64,
        );
        assert_eq!(colored_pixels(&full, RED), colored_pixels(&over, RED));
    }
}
