use defmt::Format;

#[derive(Clone)]
pub enum WifiState {
    Searching,
    AuthError,
    Configuring,
    Connected,
}

pub enum ServerState {
    NoClient,
    Client,
}

#[derive(Copy, Clone, Format)]
pub enum Pump {
    Primary,
    Secondary,
}
// Local representation of the state of a pump.
pub enum PumpState {
    Off(u64),
    On(u64),
    Unknown,
}

#[derive(Clone)]
pub enum Message {
    PumpOn { stamp: u64, pump: Pump },
    PumpOff { stamp: u64, pump: Pump },
    ClientConnected { addr: u32 },
    ClientDisconnected,
    WifiUpdate { state: WifiState },
}
