use super::{
    types::{Message, Pump, PumpState, ServerState, WifiState},
    SysSubscriber,
};
use embassy_rp::{
    i2c::{Async, I2c},
    peripherals::I2C1,
};
use embedded_graphics::{
    image::Image,
    mono_font::{ascii::FONT_9X18_BOLD, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Alignment, Text},
};
use futures::future::FutureExt;

enum LoopEvent {
    Lagging,
    Message(Message),
}

// Determines the amount of time to use a layout. OLEDs can get dim over
// time -- especially when the image is static, like this application's.
// When this number of milliseconds has elapsed, we use a different layout.

const FLIP_LAYOUT: u64 = 1_000 * 60 * 15;

fn pump_message(pri: &PumpState, sec: &PumpState) -> Option<&'static str> {
    match (pri, sec) {
        (PumpState::On(_), PumpState::On(_)) => Some("Both"),
        (_, PumpState::On(_)) => Some("Secondary"),
        (PumpState::On(_), _) => Some("Primary"),
        (_, _) => None,
    }
}

// This task is responsible for updating the OLED display. It has a `Ticker`
// which fires every 1/4 second. This is used to blink icons, if necessary.
// It also waits for messages from the PubSub channel. The messages are used
// to update internal state which determines what goes on the display.

#[embassy_executor::task]
pub async fn task(i2c: I2c<'static, I2C1, Async>, mut rx: SysSubscriber) -> ! {
    use embassy_time::{Duration, Instant, Ticker};
    use ssd1306::{
        mode::DisplayConfigAsync, prelude::DisplayRotation, size::DisplaySize128x64,
        I2CDisplayInterface, Ssd1306Async,
    };
    use tinybmp::Bmp;

    let mut display = Ssd1306Async::new(
        I2CDisplayInterface::new(i2c),
        DisplaySize128x64,
        DisplayRotation::Rotate0,
    )
    .into_buffered_graphics_mode();

    // These assignments create the bitmaps. The yse of `.unwrap()` is safe
    // here because the bitmap data is compiled into the executable and it
    // didn't fail while developing the code, so it can't fail in the
    // production version.

    let wifi_data = Bmp::from_slice(include_bytes!("assets/wifi.bmp")).unwrap();
    let wifi_search_data = Bmp::from_slice(include_bytes!("assets/wifi_search.bmp")).unwrap();
    let wifi_error_data = Bmp::from_slice(include_bytes!("assets/wifi_error.bmp")).unwrap();
    let client_data = Bmp::from_slice(include_bytes!("assets/client.bmp")).unwrap();
    let no_client_data = Bmp::from_slice(include_bytes!("assets/noclient.bmp")).unwrap();

    // Initialize the display hardware.

    display.init().await.unwrap();

    // Create the ticker that drives our 1.4 second update rate (for flashing
    // icons.)

    let mut tick = Ticker::every(Duration::from_millis(250));

    // The task's "global" state. These variables are updated by the contents
    // of the messages from the PubSub channel.

    let mut wifi_state = WifiState::Searching;
    let mut server_state = ServerState::NoClient;
    let mut pri_state = PumpState::Unknown;
    let mut sec_state = PumpState::Unknown;

    // Infinite loop. This task never exits.

    loop {
        use embassy_futures::select::Either;
        use embassy_sync::pubsub::WaitResult;

        // Wait for either a tick or a message from the PubSub channel.
        // Convert either event into a `LoopEvent`.

        let event = embassy_futures::select::select(
            tick.next(),
            rx.next_message().map(|msg| {
                if let WaitResult::Message(msg) = msg {
                    LoopEvent::Message(msg)
                } else {
                    LoopEvent::Lagging
                }
            }),
        )
        .await;

        // Now update global state or the display, depending on the event.

        match event {
            Either::First(()) => {
                let now = Instant::now().as_millis();

                // Determine which if the two layouts to use. The offset for the
                // sidebar's icons is adjusted based on this value.

                let flip_layout = (now % (FLIP_LAYOUT * 2)) >= FLIP_LAYOUT;
                let sidebar_offset = if flip_layout { 111 } else { 0 };

                // Clear the video memory.

                display.clear(BinaryColor::Off).unwrap();

                // Draw the pump state.

                if let Some(pump_msg) = pump_message(&pri_state, &sec_state) {
                    let style = MonoTextStyle::new(&FONT_9X18_BOLD, BinaryColor::On);

                    Text::with_alignment(pump_msg, Point::new(64, 32), style, Alignment::Center)
                        .draw(&mut display)
                        .unwrap();
                }

                // Draw the side bar -- First draw the appropriate WiFi icon. If
                // we're not yet connected or an error occurred, we flash the
                // icon (by conditionally drawing it based on the time.)

                match wifi_state {
                    WifiState::Configuring | WifiState::Searching => {
                        let bmp = Image::new(
                            &wifi_search_data,
                            Point {
                                x: sidebar_offset,
                                y: 0,
                            },
                        );

                        if (now % 1000) >= 500 {
                            bmp.draw(&mut display).unwrap();
                        }
                    }
                    WifiState::AuthError => {
                        let bmp = Image::new(
                            &wifi_error_data,
                            Point {
                                x: sidebar_offset,
                                y: 0,
                            },
                        );

                        if (now % 500) >= 250 {
                            bmp.draw(&mut display).unwrap();
                        }
                    }
                    WifiState::Connected => {
                        let bmp = Image::new(
                            &wifi_data,
                            Point {
                                x: sidebar_offset,
                                y: 0,
                            },
                        );

                        bmp.draw(&mut display).unwrap();
                    }
                }

                // Drawing the sidebar -- now draw the state of the server (whether it
                // has a connected client.)

                match server_state {
                    ServerState::NoClient => Image::new(
                        &no_client_data,
                        Point {
                            x: sidebar_offset,
                            y: 20,
                        },
                    ),
                    ServerState::Client => Image::new(
                        &client_data,
                        Point {
                            x: sidebar_offset,
                            y: 20,
                        },
                    ),
                }
                .draw(&mut display)
                .unwrap();

                // Copy the memory to the display.

                display.flush().await.unwrap();
            }
            Either::Second(LoopEvent::Lagging) => {
                defmt::warn!("display task lagging");
            }
            Either::Second(LoopEvent::Message(Message::PumpOn { stamp, pump })) => match pump {
                Pump::Primary => pri_state = PumpState::On(stamp),
                Pump::Secondary => sec_state = PumpState::On(stamp),
            },
            Either::Second(LoopEvent::Message(Message::PumpOff { stamp, pump })) => match pump {
                Pump::Primary => pri_state = PumpState::Off(stamp),
                Pump::Secondary => sec_state = PumpState::Off(stamp),
            },
            Either::Second(LoopEvent::Message(Message::WifiUpdate { state })) => {
                wifi_state = state;
            }
            Either::Second(LoopEvent::Message(Message::ClientConnected { addr })) => {
                server_state = ServerState::Client;
                defmt::info!(
                    "Client connected: {:02}.{:02}.{:02}.{:02}",
                    (addr >> 24) & 0xFF,
                    (addr >> 16) & 0xFF,
                    (addr >> 8) & 0xFF,
                    addr & 0xFF
                );
            }
            Either::Second(LoopEvent::Message(Message::ClientDisconnected)) => {
                server_state = ServerState::NoClient;
            }
        }
    }
}
