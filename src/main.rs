#![no_std]
#![no_main]

use cyw43::JoinOptions;
use cyw43_pio::{PioSpi, DEFAULT_CLOCK_DIVIDER};
use defmt::unwrap;
use embassy_executor::Spawner;
use embassy_net::{DhcpConfig, StackResources};
use embassy_rp::{
    bind_interrupts,
    clocks::RoscRng,
    gpio::{Level, Output},
    i2c::{self, I2c},
    peripherals::{DMA_CH0, I2C1, PIO0},
    pio::{self, Pio},
};
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    pubsub::{publisher::Publisher, subscriber::Subscriber, PubSubChannel},
};
use rand::RngCore;
use ssd1306::{
    prelude::DisplayRotation, size::DisplaySize128x64, I2CDisplayInterface, Ssd1306Async,
};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

const WIFI_NETWORK: &str = "***********";
const WIFI_PASSWORD: &[u8] = b"**********";

bind_interrupts!(struct PioIrqs {
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
});

bind_interrupts!(struct I2cIrqs {
    I2C1_IRQ => i2c::InterruptHandler<I2C1>;
});

#[derive(Clone)]
enum WifiState {
    Searching,
    AuthError,
    Configuring,
    Connected,
}

enum ServerState {
    NoClient,
    Client,
}

#[derive(Clone)]
enum Pump {
    Primary,
    Secondary,
}

#[derive(Clone)]
enum Message {
    PumpOn { stamp: u64, pump: Pump },
    PumpOff { stamp: u64, pump: Pump },
    ClientConnected { addr: u32 },
    ClientDisconnected,
    WifiUpdate { state: WifiState },
}

// Data types used to manage the PubSub channel. Since all tasks will be
// on one executor, it is safe to use the `NoopRawMutex` for synchronization.

type SysEvents = PubSubChannel<NoopRawMutex, Message, 8, 1, 1>;
type SysPublisher = Publisher<'static, NoopRawMutex, Message, 8, 1, 1>;
type SysSubscriber = Subscriber<'static, NoopRawMutex, Message, 8, 1, 1>;

mod display;
mod heartbeat;

// This project uses the CYW4349 WiFi interface. This function defines the
// background task that manages the hardware.

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    static SYS_CHAN: StaticCell<SysEvents> = StaticCell::new();
    let p = embassy_rp::init(Default::default());

    let sys_chan = SYS_CHAN.init(SysEvents::new());

    // This section initializes and spawns a task that uses the SDD1306 OLED
    // hardware to display the state of the sump monitor.

    {
        let display = {
            let mut cfg = i2c::Config::default();

            cfg.frequency = 400_000;

            let interface =
                I2CDisplayInterface::new(I2c::new_async(p.I2C1, p.PIN_27, p.PIN_26, I2cIrqs, cfg));

            Ssd1306Async::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
                .into_buffered_graphics_mode()
        };

        unwrap!(spawner.spawn(display::task(display, sys_chan.subscriber().unwrap())));
    }

    // This section initializes the CYW43 Wifi hardware and returns a data
    // type that allows us to control the LED.

    let (net_device, mut control) = {
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

        let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, FWARE).await;

        unwrap!(spawner.spawn(cyw43_task(runner)));

        control.init(FWARE_CLM).await;
        control
            .set_power_management(cyw43::PowerManagementMode::Performance)
            .await;

        (net_device, control)
    };

    // This section initializes the network stack. We reserve space for 2
    // sockets: 1 socket is used for DHCP and the other will be for incoming
    // client connections.

    let (stack, runner) = {
        static RESOURCES: StaticCell<StackResources<2>> = StaticCell::new();

        let mut rng = RoscRng;
        let seed = rng.next_u64();
        let config = embassy_net::Config::dhcpv4(DhcpConfig::default());

        embassy_net::new(
            net_device,
            config,
            RESOURCES.init(StackResources::new()),
            seed,
        )
    };
    
    unwrap!(spawner.spawn(net_task(runner)));

    match control
        .join(WIFI_NETWORK, JoinOptions::new(WIFI_PASSWORD))
        .await
    {
        Ok(()) => {
            defmt::info!("joined network");
        }
        Err(_) => {
            defmt::error!("failed to join network");
        }
    }

    unwrap!(spawner.spawn(heartbeat::task(control)));
}
