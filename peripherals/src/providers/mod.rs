//! Per-provider modules. One file per brand protocol.

use hidapi::HidDevice;

pub(crate) fn drain(dev: &HidDevice) {
    let mut buf = [0u8; 128];
    for _ in 0..16 {
        let _ = dev.read_timeout(&mut buf, 1);
    }
}

mod compx;
mod ninjutso;
mod rapoo;
mod wlmouse;
mod zynoo;
mod vgn;
mod scyrox;
mod sora_v3;
mod rog;
mod steelseries;
mod gwolves;
mod pulsar;
mod webbrands;
mod ajazz;
mod logitech;
mod razer;
mod corsair;
mod hyperx;

pub use compx::Compx;
pub use ninjutso::Ninjutso;
pub use rapoo::Rapoo;
pub use wlmouse::WlMouseCube;
pub use zynoo::Zynoo;
pub use vgn::VgnLd;
pub use scyrox::ScyroxS;
pub use sora_v3::SoraV3;
pub use rog::RogStandard;
pub use steelseries::SteelSeries;
pub use gwolves::GWolves;
pub use pulsar::Pulsar;
pub use webbrands::WebBrands;
pub use ajazz::Ajazz;
pub use logitech::Logitech;
pub use razer::Razer;
pub use corsair::Corsair;
pub use hyperx::HyperX;
