//! third-party plugin hosting.

pub mod clap;
pub mod lv2;
pub mod scan;
#[cfg(target_os = "linux")]
mod x11;

/// averages a plugin's output channels back to mono
pub(crate) fn mixdown(outs: &[Box<[f32]>], buf: &mut [f32]) {
    let n = buf.len();
    match outs {
        [] => {}
        [mono] => buf.copy_from_slice(&mono[..n]),
        many => {
            let scale = 1.0 / many.len() as f32;
            for (i, s) in buf.iter_mut().enumerate() {
                *s = many.iter().map(|ch| ch[i]).sum::<f32>() * scale;
            }
        }
    }
}
