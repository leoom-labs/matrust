use super::{Filter, check_cancel};
use crate::Result;
use std::sync::atomic::AtomicBool;

impl Filter {
    pub(super) fn process_small<const ORDER: usize, const FEEDBACK: bool>(
        &mut self,
        data: &mut [f64],
        cancel: &AtomicBool,
    ) -> Result<()> {
        debug_assert!(ORDER == 1 || ORDER == 2);
        debug_assert_eq!(self.state.len(), ORDER);
        let (b0, b1, a1) = (self.b[0], self.b[1], self.a[1]);
        let (b2, a2) = if ORDER == 2 {
            (self.b[2], self.a[2])
        } else {
            (0., 0.)
        };
        let mut z0 = self.state[0];
        let mut z1 = if ORDER == 2 { self.state[1] } else { 0. };
        for chunk in data.chunks_mut(1024) {
            check_cancel(cancel)?;
            for value in chunk {
                let x = *value;
                let y = b0 * x + z0;
                // Preserve the general kernel's operation order, including
                // omitting feedback for FIR so NaNs leave after its delay.
                if ORDER == 2 {
                    z0 = z1 + b1 * x;
                    z1 = b2 * x;
                    if FEEDBACK {
                        z0 -= a1 * y;
                        z1 -= a2 * y;
                    }
                } else {
                    z0 = b1 * x;
                    if FEEDBACK {
                        z0 -= a1 * y;
                    }
                }
                *value = y;
            }
            // Commit at the same cancellation boundaries as the general loop.
            self.state[0] = z0;
            if ORDER == 2 {
                self.state[1] = z1;
            }
        }
        Ok(())
    }
}
