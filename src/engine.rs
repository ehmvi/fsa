//! the real-time processing chain. everything in `process` must be
//! allocation-, lock- and io-free.

pub struct Engine {}

impl Engine {
    pub fn new() -> Self {
        Self {}
    }

    /// processes one mono block. `input` and `output` have the same length.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        output.copy_from_slice(input);
    }
}
