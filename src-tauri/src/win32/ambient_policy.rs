//! Pure, bounded analysis of the existing window ribbon; no capture or UI calls.
//! Only ambient::capture_edge_ribbon uses this policy (wallpaper is unaffected).

/// Reject text-like/frequently interleaved edges, preserving broad panels and
/// smooth gradients. Dominance means coverage of the sampled row, not the window.
pub(super) fn clutter_color(rgb: &[u8]) -> Option<[u8; 3]> {
    let count = rgb.len() / 3;
    if count < 32 || count > 1280 || rgb.len() % 3 != 0 {
        return None;
    }
    // 4 bits/channel groups antialiasing shades. Fixed 8 KiB, no allocation.
    let mut bins = [0u16; 4096];
    let mut edges = 0usize;
    let mut previous = &rgb[..3];
    for pixel in rgb.chunks_exact(3) {
        let distance = (0..3)
            .map(|c| pixel[c].abs_diff(previous[c]))
            .max()
            .unwrap();
        if distance >= 24 {
            edges += 1;
        }
        previous = pixel;
        bins[bin(pixel)] += 1;
    }
    // At least 12 strong edges and 4% of the ribbon. A split pane or isolated
    // button is not clutter; repeated text strokes / narrow stripes are.
    if edges < 12 || edges * 100 < (count - 1) * 4 {
        return None;
    }
    // First winner on ties keeps the result deterministic.
    let mut winner = 0;
    for i in 1..bins.len() {
        if bins[i] > bins[winner] {
            winner = i;
        }
    }
    let mut sum = [0u32; 3];
    for pixel in rgb.chunks_exact(3).filter(|pixel| bin(pixel) == winner) {
        for c in 0..3 {
            sum[c] += pixel[c] as u32;
        }
    }
    Some(sum.map(|value| (value / bins[winner] as u32) as u8))
}

fn bin(pixel: &[u8]) -> usize {
    ((pixel[0] as usize >> 4) << 8) | ((pixel[1] as usize >> 4) << 4) | (pixel[2] as usize >> 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(count: usize, color: impl Fn(usize) -> [u8; 3]) -> Vec<u8> {
        (0..count).flat_map(color).collect()
    }

    #[test]
    fn preserves_solid_gradients_and_broad_panels() {
        assert_eq!(clutter_color(&row(960, |_| [230, 220, 210])), None);
        assert_eq!(
            clutter_color(&row(960, |i| [(i * 255 / 959) as u8; 3])),
            None
        );
        assert_eq!(
            clutter_color(&row(960, |i| if i < 600 { [240; 3] } else { [20; 3] })),
            None
        );
    }

    #[test]
    fn text_strokes_choose_background_not_average() {
        let pixels = row(960, |i| match i % 24 {
            0..=2 => [30; 3],
            3 => [150; 3],
            _ => [242, 243, 244],
        });
        assert_eq!(clutter_color(&pixels), Some([242, 243, 244]));
    }

    #[test]
    fn mixed_colors_choose_largest_group_and_average_its_shades() {
        let pixels = row(960, |i| match i % 4 {
            0 => [240, 242, 244],
            1 => [244, 246, 248],
            2 => [180, 20, 20],
            _ => [20, 20, 180],
        });
        assert_eq!(clutter_color(&pixels), Some([242, 244, 246]));
    }

    #[test]
    fn bounds_and_low_contrast_noise_are_safe() {
        for pixels in [vec![], vec![0; 97], vec![0; 3843], row(31, |_| [0; 3])] {
            assert_eq!(clutter_color(&pixels), None);
        }
        assert_eq!(
            clutter_color(&row(1280, |i| [220 + (i % 10) as u8; 3])),
            None
        );
        assert_eq!(
            clutter_color(&row(1280, |i| if i % 4 == 0 { [0; 3] } else { [250; 3] })),
            Some([250; 3])
        );
    }

    #[test]
    #[ignore = "manual optimized policy timing, excludes capture/PNG/UI"]
    fn timing() {
        let pixels = row(1280, |i| if i % 4 == 0 { [20; 3] } else { [242; 3] });
        let start = std::time::Instant::now();
        for _ in 0..100_000 {
            std::hint::black_box(clutter_color(std::hint::black_box(&pixels)));
        }
        println!(
            "clutter policy: {:.2} us/call (1280 pixels, 100000 calls)",
            start.elapsed().as_secs_f64() * 10.0
        );
    }
}
