//! Mux PTS ordering. Ported from jub0t/Concat#68.

use anyhow::Result;
use ffmpeg_next as ffmpeg;

pub fn compare_pts(pts: Option<i64>, tb: ffmpeg::Rational) -> i64 {
    let pts = pts.unwrap_or(i64::MIN);
    if pts == i64::MIN {
        return i64::MIN;
    }
    pts.saturating_mul(tb.numerator() as i64) / tb.denominator() as i64
}

pub fn mux_stub() -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_pts_normalizes_timebase() {
        let tb = ffmpeg::Rational(1, 15360);
        assert_eq!(compare_pts(Some(15360), tb), 1);
        let tb = ffmpeg::Rational(1, 48000);
        assert_eq!(compare_pts(Some(48000), tb), 1);
    }

    #[test]
    fn compare_pts_handles_none() {
        let tb = ffmpeg::Rational(1, 1000);
        assert_eq!(compare_pts(None, tb), i64::MIN);
    }
}
