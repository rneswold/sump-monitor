#![no_std]
#![no_main]

use cyw43::Control;
use cyw43_pio::{PioSpi, DEFAULT_CLOCK_DIVIDER};
use defmt::unwrap;
use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts,
    gpio::{Level, Output},
    peripherals::{DMA_CH0, PIO0},
    pio::{InterruptHandler, Pio},
};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

// This project uses the CYW4349 WiFi interface. This function defines the
// background task that manages the hardware.

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn blink_led(mut control: Control<'static>) -> ! {
    use embassy_time::{Duration, Ticker};

    let delay = Duration::from_millis(1000);
    let mut ticker = Ticker::every(delay);
    let mut state = false;

    loop {
        control.gpio_set(0, state).await;
        state = !state;

        ticker.next().await;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // This section initializes the CYW43 Wifi hardware and returns a data
    // type that allows us to control the LED.

    let control = {
        let pwr = Output::new(p.PIN_23, Level::Low);
        let cs = Output::new(p.PIN_25, Level::High);
        let mut pio = Pio::new(p.PIO0, Irqs);
        let spi = PioSpi::new(
            &mut pio.common,
            pio.sm0,
            DEFAULT_CLOCK_DIVIDER,
            pio.irq0,
            cs,
            p.PIN_24,
            p.PIN_29,
            p.DMA_CH0,
        );

        const FWARE: &[u8] = include_bytes!("firmware/43439A0.bin");
        const FWARE_CLM: &[u8] = include_bytes!("firmware/43439A0_clm.bin");

        static STATE: StaticCell<cyw43::State> = StaticCell::new();

        let state = STATE.init(cyw43::State::new());
        let (_net_device, mut control, runner) = cyw43::new(state, pwr, spi, FWARE).await;

        unwrap!(spawner.spawn(cyw43_task(runner)));

        control.init(FWARE_CLM).await;
        control
            .set_power_management(cyw43::PowerManagementMode::PowerSave)
            .await;
        control
    };

    unwrap!(spawner.spawn(blink_led(control)));
}
