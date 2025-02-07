#![no_std]
#![no_main]

use embassy_executor::Spawner;
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let _p = embassy_rp::init(Default::default());
}
