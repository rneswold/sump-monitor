#![no_std]
#![no_main]

use cyw43::Control;
use cyw43_pio::{PioSpi, DEFAULT_CLOCK_DIVIDER};
use defmt::unwrap;
use display_interface_i2c::I2CInterface;
use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts,
    gpio::{Level, Output},
    i2c::{self, Async, I2c},
    peripherals::{DMA_CH0, I2C1, PIO0},
    pio::{self, Pio},
};
use embedded_graphics::{
    image::{Image, ImageRaw},
    pixelcolor::BinaryColor,
    prelude::*,
    Drawable,
};
use ssd1306::{
    mode::{BufferedGraphicsModeAsync, DisplayConfigAsync}, prelude::DisplayRotation, size::DisplaySize128x64,
    I2CDisplayInterface, Ssd1306Async,
};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct PioIrqs {
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
});

bind_interrupts!(struct I2cIrqs {
    I2C1_IRQ => i2c::InterruptHandler<I2C1>;
});

// This project uses the CYW4349 WiFi interface. This function defines the
// background task that manages the hardware.

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

// Runs a task that is used as a heartbeat indicator. Eventually, all
// background tasks will need to periodically notify this task to prove
// they're still running. This task will flash the LED (and feed the
// watchdog?) while everything is healthy.
//
// Right now it simply flashes the onboard LED.

#[embassy_executor::task]
async fn heartbeat(mut control: Control<'static>) -> ! {
    use embassy_time::{Duration, Ticker};

    let delay = Duration::from_millis(100);
    let mut ticker = Ticker::every(delay);
    let mut state = false;

    loop {
        control.gpio_set(0, state).await;
        state = !state;

        ticker.next().await;
    }
}

#[embassy_executor::task]
async fn display_control(mut display: Ssd1306Async<I2CInterface<I2c<'static, I2C1, Async>>, DisplaySize128x64, BufferedGraphicsModeAsync<DisplaySize128x64>>) -> ! {

    display.init().await.unwrap();

    let raw: ImageRaw<BinaryColor> = ImageRaw::new(include_bytes!("./rust.raw"), 64);
    let mut offset_iter = (0..=64).chain((1..64).rev()).cycle();

    loop {
        use embedded_graphics::{
            mono_font::{ascii::FONT_9X15_BOLD, MonoTextStyle},
            text::{Alignment, Text},
        };

        if let Some(x) = offset_iter.next() {
        let top_left = Point::new(x, 0);
        let im = Image::new(&raw, top_left);

        im.draw(&mut display).unwrap();
        display.flush().await.unwrap();
        display.clear(BinaryColor::Off).unwrap();
        }
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
        let mut pio = Pio::new(p.PIO0, PioIrqs);
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

        static STATE: StaticCell<cyw43::State> = StaticCell::new();

        let state = STATE.init(cyw43::State::new());

        const FWARE: &[u8] = include_bytes!("firmware/43439A0.bin");
        const FWARE_CLM: &[u8] = include_bytes!("firmware/43439A0_clm.bin");

        let (_net_device, mut control, runner) = cyw43::new(state, pwr, spi, FWARE).await;

        unwrap!(spawner.spawn(cyw43_task(runner)));

        control.init(FWARE_CLM).await;
        control
            .set_power_management(cyw43::PowerManagementMode::Performance)
            .await;
        control
    };

    unwrap!(spawner.spawn(heartbeat(control)));

    // This section initializes the SDD1306 OLED hardware.

    let display = {
        let mut cfg = i2c::Config::default();

        cfg.frequency = 400_000;

        let interface =
            I2CDisplayInterface::new(I2c::new_async(p.I2C1, p.PIN_27, p.PIN_26, I2cIrqs, cfg));

        Ssd1306Async::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode()
    };

    unwrap!(spawner.spawn(display_control(display)));
}
