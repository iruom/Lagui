//! Host frame policy. Check dirty before acquisition; finish waits before sampling
//! input, then build, prepare, encode, submit and present. Button/key edges belong
//! to the event stream; a pointer probe only refreshes position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PresentPolicy {
    #[default]
    Vsync,
    LowLatency,
    Tearing,
}

#[cfg(feature = "renderer-wgpu")]
impl PresentPolicy {
    pub fn select(self, supported: &[wgpu::PresentMode]) -> wgpu::PresentMode {
        use wgpu::PresentMode::{Fifo, Immediate, Mailbox};
        let preference = match self {
            Self::Vsync => [Fifo, Fifo, Fifo],
            Self::LowLatency => [Mailbox, Immediate, Fifo],
            Self::Tearing => [Immediate, Mailbox, Fifo],
        };
        preference
            .into_iter()
            .find(|mode| supported.contains(mode))
            .unwrap_or(Fifo)
    }
}

#[cfg(all(test, feature = "renderer-wgpu"))]
mod tests {
    use super::*;
    #[test]
    fn policy_uses_supported_modes_in_preference_order() {
        use wgpu::PresentMode::*;
        assert_eq!(
            PresentPolicy::LowLatency.select(&[Fifo, Immediate, Mailbox]),
            Mailbox
        );
        assert_eq!(PresentPolicy::Tearing.select(&[Fifo, Mailbox]), Mailbox);
        assert_eq!(PresentPolicy::LowLatency.select(&[Fifo]), Fifo);
        assert_eq!(PresentPolicy::Vsync.select(&[Fifo, Immediate]), Fifo);
    }
}
