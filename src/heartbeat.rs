use cyw43::Control;
use embassy_time::{Duration, Ticker};
use embassy_rp::peripherals::BOOTSEL;

const DELAY: Duration = Duration::from_millis(50);

// Runs a task that is used as a heartbeat indicator. Eventually, all
// background tasks will need to periodically notify this task to prove
// they're still running. This task will flash the LED (and feed the
// watchdog?) while everything is healthy.
//
// Right now it simply flashes the onboard LED.

#[embassy_executor::task]
pub async fn task(mut control: Control<'static>, mut button: BOOTSEL) -> ! {
    let mut ticker = Ticker::every(DELAY);
    let mut state = 0u32;

    loop {
        control.gpio_set(0, state == 0 || button.is_pressed()).await;
        state = (state + 1) % 20;
        ticker.next().await;
    }
}
