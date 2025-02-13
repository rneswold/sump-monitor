use super::{ServerState, WifiState, Pump, Message};
use display_interface_i2c::I2CInterface;
use embassy_rp::{
    i2c::{Async, I2c},
    peripherals::I2C1,
};
use ssd1306::{
    mode::{BufferedGraphicsModeAsync, DisplayConfigAsync},
    size::DisplaySize128x64,
    Ssd1306Async,
};

// Local representation of the state of a pump.
enum PumpState {
    Off(u64),
    On(u64),
    Unknown,
}

// Determines the amount of time to use a layout. OLEDs can get dim over
// time -- especially when the image is static, like this application's.
// When this number of milliseconds has elapsed, we use a different layout.

const FLIP_LAYOUT: u64 = 10_000;

// Define `OLED` to be the type that manages the SSD1306-based OLED display.

type OLED = Ssd1306Async<
    I2CInterface<I2c<'static, I2C1, Async>>,
    DisplaySize128x64,
    BufferedGraphicsModeAsync<DisplaySize128x64>,
>;

fn pump_message(pri: PumpState, sec: PumpState) -> Option<&'static str> {
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
pub async fn task(mut display: OLED) -> ! {
    use embassy_time::{Duration, Instant, Ticker};
    use embedded_graphics::{image::Image, pixelcolor::BinaryColor, prelude::*, Drawable};
    use tinybmp::Bmp;

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
    let mut pump_updated: u64 = 0;

    // Infinite loop. This task never exits.

    loop {
        let now = Instant::now().as_millis();

        // Determine which if the two layouts to use. The offset for the
        // sidebar's icons is adjusted based on this value.

        let flip_layout = (now % (FLIP_LAYOUT * 2)) >= FLIP_LAYOUT;
        let sidebar_offset = if flip_layout { 111 } else { 0 };

        // Clear the video memory.

        display.clear(BinaryColor::Off).unwrap();

        // Draw the side bar -- First draw the appropriate WiFi icon. If
        // we're not yet connected or an error occurred, we flash the
        // icon (by conditionally drawing it based on the time.)

        match wifi_state {
            WifiState::Searching => {
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

        // Wait for the next tick.

        tick.next().await;
    }
}
