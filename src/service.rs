// Provide a network service to receive sump pump events.
//
// Due to limitations in the network stack, it only handles one client at a
// time. This is fine for our use case, as we only expect one client (DrMem)
// to be interested in this data. General interest in the data should be
// obtained by using the DrMem control system API.
//
// When a client connects, it only receives data from the server. The protocol
// is a simple, fixed-size message. Each message is 16 bytes long and is made
// up of 2 64-bit fields. The first field is a timestamp, and the second field
// is descriptor field. The timestamp is based on the internal, microsecond
// timer -- not time-of-day. All values are big-endian.
//
//      +----+----+----+----+----+----+----+----+
//   0  |         microsecond timestamp         |
//      +----+----+----+----+----+----+----+----+
//   8  | 00 | 00 | 00 | 00 | 00 | 00 | EC | TC |
//      +----+----+----+----+----+----+----+----+
//
//   type codes (TC):
//
//       0x00: Keepalive
//       0x01: Error Condition (EC holds the error code)
//       0x02: Primary pump OFF
//       0x03: Primary pump ON
//       0x04: Secondary pump OFF
//       0x05: Secondary pump ON
//
//   the EC field is only used for error conditions (TC = 1) and will be 0 for
//   all other messages.
//
// When a client connects, it receives up to three messages: a "keep-alive"
// message which contains the current timestamp of the controller, and two
// optional messages indicating the last state of the pumps. The client then
// receives messages as they are generated by the pump monitor task.

use super::{
    types::{Message, Pump, PumpState},
    SysPublisher, SysSubscriber,
};
use defmt::warn;
use embassy_futures::select::{select, Either};
use embassy_net::{
    tcp::{Error, TcpSocket},
    IpAddress, IpEndpoint,
};
use embassy_sync::pubsub::WaitResult;
use embassy_time::{Duration, Instant};

const NOOP: u8 = 0x00;
// const ERROR: u8 = 0x01;
const PRIMARY_OFF: u8 = 0x02;
const PRIMARY_ON: u8 = 0x03;
const SECONDARY_OFF: u8 = 0x04;
const SECONDARY_ON: u8 = 0x05;

const SERVICE_PORT: u16 = 10_000;

// Builds the 16-byte packet that is used to report service status to the
// client.

fn build_packet(stamp: u64, tc: u8, ec: u8, buf: &mut [u8; 16]) {
    const FILL: [u8; 6] = [0u8; 6];

    let stamp: [u8; 8] = stamp.to_be_bytes();

    buf[0..8].copy_from_slice(&stamp);
    buf[8..14].copy_from_slice(&FILL);
    buf[14] = ec;
    buf[15] = tc;
}

// Sends the 16-byte packet to the client.

async fn send_report(s: &mut TcpSocket<'_>, stamp: u64, tc: u8, ec: u8) -> Result<(), Error> {
    let mut buf = [0u8; 16];

    build_packet(stamp, tc, ec, &mut buf);
    if let Ok(n) = s.write(&buf).await {
        if n != buf.len() {
            return Err(Error::ConnectionReset);
        }
    }
        Ok(())
}

// Sends initial reports to the clients based on the state of the primary
// and secondary pumps.

async fn initial_reports(
    s: &mut TcpSocket<'_>,
    pri: &PumpState,
    sec: &PumpState,
) -> Result<(), Error> {
    // Send a keepalive message to the client so they know the controller's
    // current timestamp.

    send_report(s, Instant::now().as_micros(), NOOP, 0).await?;

    // Now send the state of the pumps.

    match pri {
        PumpState::Off(pts) => {
            send_report(s, *pts, PRIMARY_OFF, 0).await?;
        }
        PumpState::On(pts) => {
            send_report(s, *pts, PRIMARY_ON, 0).await?;
        }
        PumpState::Unknown => {}
    }

    match sec {
        PumpState::Off(sts) => {
            send_report(s, *sts, SECONDARY_OFF, 0).await?;
        }
        PumpState::On(sts) => {
            send_report(s, *sts, SECONDARY_ON, 0).await?;
        }
        PumpState::Unknown => {}
    }

    // Send the initial state to the client.

    s.flush().await
}

// Waits for a client to connect.
//
// While we wait for a client, we also listen for pump state updates from the
// PubSub channel to update the global state.

async fn wait_for_client(
    s: &mut TcpSocket<'_>,
    rx: &mut SysSubscriber,
    pri: &mut PumpState,
    sec: &mut PumpState,
) -> Result<(), Error> {
    loop {
        if let Either::Second(msg) = select(s.accept(SERVICE_PORT), rx.next_message()).await {
            match msg {
                WaitResult::Message(payload) => match payload {
                    Message::PumpOff {
                        stamp,
                        pump: Pump::Primary,
                    } => {
                        *pri = PumpState::Off(stamp);
                    }
                    Message::PumpOff {
                        stamp,
                        pump: Pump::Secondary,
                    } => {
                        *sec = PumpState::Off(stamp);
                    }
                    Message::PumpOn {
                        stamp,
                        pump: Pump::Primary,
                    } => {
                        *pri = PumpState::On(stamp);
                    }
                    Message::PumpOn {
                        stamp,
                        pump: Pump::Secondary,
                    } => {
                        *sec = PumpState::On(stamp);
                    }
                    _ => {}
                },
                WaitResult::Lagged(_) => {}
            }
        } else {
            break Ok(());
        }
    }
}

async fn serve_client(
    s: &mut TcpSocket<'_>,
    rx: &mut SysSubscriber,
    pri: &mut PumpState,
    sec: &mut PumpState,
) -> () {
    loop {
        let mut buf = [0u8; 16];

        // Wait for a message from the client or a message from the
        // PubSub channel.

        let msg = select(s.read(&mut buf[..]), rx.next_message()).await;

        match msg {
            Either::First(_) => {
                // Either the socket has an error or the client sent us
                // data.
                //
                // The client isn't supposed to send us data. This is a
                // programming error on their part, or a DOS attack. We
                // shutdown the socket and break out of the loop.

                break;
            }
            Either::Second(msg) => match msg {
                WaitResult::Message(payload) => match payload {
                    Message::PumpOff {
                        stamp,
                        pump: Pump::Primary,
                    } => {
                        *pri = PumpState::Off(stamp);
                        if send_report(s, stamp, PRIMARY_OFF, 0).await.is_err() {
                            break;
                        }
                    }
                    Message::PumpOff {
                        stamp,
                        pump: Pump::Secondary,
                    } => {
                        *sec = PumpState::Off(stamp);
                        if send_report(s, stamp, SECONDARY_OFF, 0).await.is_err() {
                            break;
                        }
                    }
                    Message::PumpOn {
                        stamp,
                        pump: Pump::Primary,
                    } => {
                        *pri = PumpState::On(stamp);
                        if send_report(s, stamp, PRIMARY_ON, 0).await.is_err() {
                            break;
                        }
                    }
                    Message::PumpOn {
                        stamp,
                        pump: Pump::Secondary,
                    } => {
                        *sec = PumpState::On(stamp);
                        if send_report(s, stamp, SECONDARY_ON, 0).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                },
                WaitResult::Lagged(_) => {}
            },
        }
    }
}

// This is the main task that handles the network connection.

#[embassy_executor::task]
pub async fn task(
    stack: embassy_net::Stack<'static>,
    tx: SysPublisher,
    mut rx: SysSubscriber,
) -> ! {
    let mut tx_buf = [0u8; 128];
    let mut rx_buf = [0u8; 32];

    // Create local variables to hold mutateable state. These contain the
    // latest values reported for the pumps.

    let mut primary = PumpState::Unknown;
    let mut secondary = PumpState::Unknown;

    // The task follows a simple state machine. It waits for a client to
    // connect and then sends messages to the client. When the client
    // disconnects, it waits for a new client to connect. This outer
    // loop lets us transition back to the wait-for-client state.

    loop {
        tx.publish_immediate(Message::ClientDisconnected);

        // Create the TCP socket and bind it to the local address.

        let mut s = TcpSocket::new(stack, &mut rx_buf, &mut tx_buf);

        s.set_timeout(Some(Duration::from_secs(10)));
        s.set_keep_alive(Some(Duration::from_secs(5)));

        // STATE 1: Wait for client to connect. While we're waiting, we have to
        // listen to the PubSub channel for messages for pump state changes.

        if wait_for_client(&mut s, &mut rx, &mut primary, &mut secondary)
            .await
            .is_ok()
        {
            // Get the client's address and announce that it has connected.

            let addr = match s.remote_endpoint() {
                Some(IpEndpoint {
                    addr: IpAddress::Ipv4(addr),
                    ..
                }) => addr.into(),
                None => 0,
            };

            tx.publish_immediate(Message::ClientConnected { addr });

            // Transition to the next state. We need to immediately send the client
            // the last state of the pumps. If we have no state, just send a keepalive.

            if initial_reports(&mut s, &primary, &secondary).await.is_ok() {
                // STATE 2: In this state, pump updates are forwarded to the client.
                // Keepalives are also generated (since the pumps don't cycle very
                // often between rain events.)

                serve_client(&mut s, &mut rx, &mut primary, &mut secondary).await;
            }
        }

        // Shutdown the socket and free resources so we can make a new one.

        s.abort();

        let _ = s.flush().await;
    }
}
