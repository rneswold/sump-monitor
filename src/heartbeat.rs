use cyw43::Control;
use embassy_time::{Duration, Ticker};

const DELAY: Duration = Duration::from_millis(100);

// Runs a task that is used as a heartbeat indicator. Eventually, all
// background tasks will need to periodically notify this task to prove
// they're still running. This task will flash the LED (and feed the
// watchdog?) while everything is healthy.
//
// Right now it simply flashes the onboard LED.

#[embassy_executor::task]
pub async fn task(mut control: Control<'static>) -> ! {
    let mut ticker = Ticker::every(DELAY);
    let mut state = false;

    loop {
        control.gpio_set(0, state).await;
        state = !state;
        ticker.next().await;
    }
}
