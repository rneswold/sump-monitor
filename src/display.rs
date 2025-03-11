use super::{
    types::{Message, Pump, PumpState, ServerState},
    SysSubscriber,
};
use embassy_rp::{
    i2c::{Async, I2c},
    peripherals::I2C1,
};
use embedded_graphics::{
    image::Image,
    mono_font::{
        ascii::{FONT_6X10, FONT_9X18_BOLD},
        MonoTextStyle,
    },
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Alignment, Text},
};
use futures::future::FutureExt;

enum LoopEvent {
    Lagging,
    Message(Message),
}

#[derive(PartialEq)]
enum WiFiConfig {
    Connected { addr: u32, stamp: u64 },
    Disconnected { stamp: u64 },
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

async fn report_pump_state(
    display: &mut impl DrawTarget<Color = BinaryColor>,
    center: i32,
    pri: &PumpState,
    sec: &PumpState,
) -> bool {
    if let Some(pump_msg) = pump_message(pri, sec) {
        let style = MonoTextStyle::new(&FONT_9X18_BOLD, BinaryColor::On);
        let _ = Text::with_alignment(pump_msg, Point::new(center, 32), style, Alignment::Center)
            .draw(display);

        true
    } else {
        false
    }
}

async fn report_wifi_state(
    display: &mut impl DrawTarget<Color = BinaryColor>,
    center: i32,
    wifi: &WiFiConfig,
) {
    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    // If the pumps are off, we can display the WiFi address
    // (if we have one.)

    match wifi {
        WiFiConfig::Connected { addr, .. } => {
            use core::fmt::Write;
            use heapless::String;

            let mut text = String::<32>::new();
            let _ = write!(
                text,
                "WiFi\n\n{}.{}.{}.{}",
                (addr >> 24) & 0xFF,
                (addr >> 16) & 0xFF,
                (addr >> 8) & 0xFF,
                addr & 0xFF
            );
            let _ = Text::with_alignment(
                text.as_str(),
                Point::new(center, 22),
                style,
                Alignment::Center,
            )
            .draw(display);
        }
        WiFiConfig::Disconnected { .. } => {
            let _ = Text::with_alignment(
                "No WiFi\nconnection",
                Point::new(center, 27),
                style,
                Alignment::Center,
            )
            .draw(display);
        }
    }
}

async fn report_client_state(
    display: &mut impl DrawTarget<Color = BinaryColor>,
    center: i32,
    state: &ServerState,
) {
    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    // If the pumps are off, we can display the WiFi address
    // (if we have one.)

    match state {
        ServerState::Client { addr } => {
            use core::fmt::Write;
            use heapless::String;

            let mut text = String::<32>::new();
            let _ = write!(
                text,
                "Client\n\n{}.{}.{}.{}",
                (addr >> 24) & 0xFF,
                (addr >> 16) & 0xFF,
                (addr >> 8) & 0xFF,
                addr & 0xFF
            );
            let _ = Text::with_alignment(
                text.as_str(),
                Point::new(center, 22),
                style,
                Alignment::Center,
            )
            .draw(display);
        }
        ServerState::NoClient => {
            let _ = Text::with_alignment(
                "No client\nconnected",
                Point::new(center, 27),
                style,
                Alignment::Center,
            )
            .draw(display);
        }
    }
}

// This task is responsible for updating the OLED display. It has a `Ticker`
// which fires every 1/4 second. This is used to blink icons, if necessary.
// It also waits for messages from the PubSub channel. The messages are used
// to update internal state which determines what goes on the display.

#[embassy_executor::task]
pub async fn task(
    stack: embassy_net::Stack<'static>,
    i2c: I2c<'static, I2C1, Async>,
    mut rx: SysSubscriber,
) -> ! {
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

    // These assignments create the bitmaps. The use of `.unwrap()` is safe
    // here because the bitmap data is compiled into the executable and it
    // didn't fail while developing the code, so it can't fail in the
    // production version.

    let wifi_data = Bmp::from_slice(include_bytes!("assets/wifi.bmp")).unwrap();
    let client_data = Bmp::from_slice(include_bytes!("assets/client.bmp")).unwrap();
    let no_client_data = Bmp::from_slice(include_bytes!("assets/noclient.bmp")).unwrap();

    // Initialize the display hardware.

    display.init().await.unwrap();

    // Create the ticker that drives our 1/4 second update rate (for flashing
    // icons.)

    let mut tick = Ticker::every(Duration::from_millis(250));

    // The task's "global" state. These variables are updated by the contents
    // of the messages from the PubSub channel.

    let mut server_state = ServerState::NoClient;
    let mut pri_state = PumpState::Unknown;
    let mut sec_state = PumpState::Unknown;
    let mut wifi_config = WiFiConfig::Disconnected;

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
                let sidebar_offset = if flip_layout { 104 } else { 0 };

                // Clear the video memory.

                display.clear(BinaryColor::Off).unwrap();

                // Draw any text that needs to be displayed.

                {
                    let center = if flip_layout { 52 } else { 76 };

                    // Draw the pump state. Drawing the pump state always takes
                    // precedence. If the pumps are off, then we can display
                    // other, less-interesting messages.

                    if !report_pump_state(&mut display, center, &pri_state, &sec_state).await {
                        match now % 10_000 {
                            0..4_000 => {}
                            4_000..7_000 => {
                                report_wifi_state(&mut display, center, &wifi_config).await
                            }
                            7_000.. => {
                                report_client_state(&mut display, center, &server_state).await
                            }
                        }
                    }
                }

                // Draw the side bar -- First draw the appropriate WiFi icon. If
                // we're not yet connected or an error occurred, we flash the
                // icon (by conditionally drawing it based on the time.)

                if stack.is_config_up() || (now % 1000) >= 500 {
                    let bmp = Image::new(
                        &wifi_data,
                        Point {
                            x: sidebar_offset,
                            y: 4,
                        },
                    );

                    bmp.draw(&mut display).unwrap();

                    // If we go from no DHCP config to having one, update the
                    // state and mark it with the current time.

                    if matches!(wifi_config, WiFiConfig::Disconnected { .. })
                        && stack.is_config_up()
                    {
                        wifi_config = WiFiConfig::Connected {
                            addr: stack
                                .config_v4()
                                .map(|v| v.address.address().to_bits())
                                .unwrap_or(0u32),
                        };
                    }
                } else if matches!(wifi_config, WiFiConfig::Connected { .. }) {
                    wifi_config = WiFiConfig::Disconnected;
                }

                // Drawing the sidebar -- now draw the state of the server (whether it
                // has a connected client.)

                match server_state {
                    ServerState::NoClient => Image::new(
                        &no_client_data,
                        Point {
                            x: sidebar_offset,
                            y: 36,
                        },
                    ),
                    ServerState::Client => Image::new(
                        &client_data,
                        Point {
                            x: sidebar_offset,
                            y: 36,
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
