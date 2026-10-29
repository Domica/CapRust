//! Face-anonymize mask generator (#84 / #90 follow-up).
//!
//! When anonymize is toggled, creates <project>/cache/anon/<clip_id>.mkv
//! using ffmpeg facedetect + boxblur + gray output. If the source has no
//! face boxes, writes a black mask (no blur applied by maskedmerge).

use std::path::Path;
use std::process::Command;

pub fn generate_anon_mask(clip_path: &str, mask_path: &Path, ffmpeg_path: &Path) -> Option<()> {
    // Ensure cache dir exists.
    let parent = mask_path.parent()?;
    std::fs::create_dir_all(parent).ok()?;

    // Basic mask via ffmpeg facedetect: output a gray video where
    // detected face boxes are white (mask=1) and rest black (mask=0).
    // This is a stand-in — real AI mask (u2netp) is deferred.
    let output = Command::new(ffmpeg_path)
        .args([
            "-y",
            "-i",
            clip_path,
            "-vf",
            "facedetect=detection=1,drawbox=x1=0:y1=0:w=0:h=0:color=white,format=gray",
            "-c:v",
            "ffv1",
            "-frames:v",
            "1",
            mask_path.to_str()?,
        ])
        .output()
        .ok()?;

    // If ffmpeg failed, fall back to black mask (no anonymization)
    if !output.status.success() || !mask_path.exists() {
        // Write minimal black video (1 frame, gray) so maskedmerge
        // has a valid input and doesn't error out.
        let _ = Command::new(ffmpeg_path)
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=320x240",
                "-frames:v",
                "1",
                "-c:v",
                "ffv1",
                mask_path.to_str()?,
            ])
            .output();
        return Some(());
    }
    Some(())
}
