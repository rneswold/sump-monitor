use super::{
    types::{Message, Pump},
    SysPublisher,
};
use embassy_rp::gpio::{Input, Level};
use embassy_time::{Duration, Instant, Timer};

// Defines a task that monitors an input pin which indicates the state of a
// sump pump.

#[embassy_executor::task(pool_size = 2)]
pub async fn task(mut pin: Input<'static>, pump: Pump, tx: SysPublisher) -> ! {
    let mut last_state = pin.get_level();

    loop {
        pin.wait_for_any_edge().await;

        let stamp = Instant::now().as_micros();

        Timer::after(Duration::from_millis(30)).await;

        let state = pin.get_level();

        if state == last_state {
            continue;
        }
        last_state = state;

        match state {
            Level::Low => {
                tx.publish_immediate(Message::PumpOn { stamp, pump });
            }
            Level::High => {
                tx.publish_immediate(Message::PumpOff { stamp, pump });
            }
        }
    }
}
