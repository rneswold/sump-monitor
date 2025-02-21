use defmt::unwrap;
use embassy_executor::Spawner;
use embassy_net::{DhcpConfig, Stack, StackResources};
use embassy_rp::clocks::RoscRng;
use rand::RngCore;
use static_cell::StaticCell;

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

// Starts the network stack and spawns the network task. Returns a `Stack`
// object to be used to allocate network resources.

pub fn start(spawner: &Spawner, net_device: cyw43::NetDriver<'static>) -> Stack<'static> {
    static RESOURCES: StaticCell<StackResources<2>> = StaticCell::new();

    let mut rng = RoscRng;
    let config = embassy_net::Config::dhcpv4(DhcpConfig::default());

    let (stack, runner) = embassy_net::new(
        net_device,
        config,
        RESOURCES.init(StackResources::new()),
        rng.next_u64(),
    );
    unwrap!(spawner.spawn(net_task(runner)));
    stack
}
