import time
import _thread
import network
import rp2
from machine import Pin

# Defines the sump pump scan logic.

@rp2.asm_pio(out_shiftdir=rp2.PIO.SHIFT_LEFT,
             in_shiftdir=rp2.PIO.SHIFT_LEFT,
             sideset_init=rp2.PIO.OUT_LOW,
             fifo_join=rp2.PIO.JOIN_RX)
def monitor_sump():
    label("start")
    jmp(pin, "idle")    .side(1)    [1]
    jmp("sump_on")      .side(1)

    label("idle")
    set(x, 15)          .side(0)    [5]

    label("idle_loop")
    set(y, 6)          .side(0)

    label("idle_test")
    jmp(pin, "dec_loop").side(0)    [7]
    jmp("sump_on")      .side(1)    [7]

    label("dec_loop")
    jmp(y_dec, "idle_test").side(0) [7]
    jmp(x_dec, "idle_loop").side(0) [7]
    jmp("start")

    label("sump_on")
    nop()               .side(1)    [7]
    nop()               .side(1)    [7]
    nop()               .side(1)    [7]
    nop()               .side(1)    [7]

    label("on_loop")
    jmp(pin, "idle")    .side(1)
    jmp("on_loop")      .side(1)

# This function defines the PIO assembly for a state machine that reports
# an error status through an LED. If no error is active, it blips the LED
# as a heartbeat indicator.

@rp2.asm_pio(out_shiftdir=rp2.PIO.SHIFT_LEFT,
             in_shiftdir=rp2.PIO.SHIFT_LEFT,
             sideset_init=rp2.PIO.OUT_LOW,
             fifo_join=rp2.PIO.JOIN_TX)
def status():
    # Zero out ISR. ISR holds the latest error code and we default it to
    # zero (i.e. "no error".)

    in_(null, 32)       .side(0)

    # Emit a 2 sec low signal to separate blips/codes.

    label("get_next")
    set(y, 7)           .side(0)
    label("loop2")
    set(x, 30)          .side(0)    [2]
    label("loop1")
    jmp(x_dec, "loop1") .side(0)    [15]
    jmp(y_dec, "loop2") .side(0)

    # ISR always holds the lastest error code. We transfer it into X so, if
    # the PULL fails, OSR will equal X (i.e. ISR) and the error code stays
    # the same. Place whatever value we get back into ISR.

    mov(x, isr)         .side(0)
    pull(noblock)       .side(0)
    mov(isr, osr)       .side(0)

    # If the code is zero, just blip the LED.

    mov(x, osr)         .side(0)
    jmp(not_x, "blip")  .side(0)

    # Ignore leading zeroes. The error code stream of bits must at the
    # least significant end of the integer. The error stream starts with
    # a 1, which gets thrown away.

    label("skip")
    out(y, 1)           .side(0)
    jmp(not_y, "skip")  .side(0)

    # Main loop of the digit stream. If the OSR has been fully shifted out,
    # then we're done processing the code. Go look for an updated value.

    label("next")
    jmp(not_osre, "load").side(0)
    jmp("get_next")     .side(0)

    # Get the next bit of the OSR and set X to the width of the resulting
    # pulse.

    label("load")
    set(x, 7)           .side(1)
    out(y, 1)           .side(1)
    jmp(not_y, "digit") .side(1)    [5]
    set(x, 23)          .side(1)

    # Generate a pulse.

    label("digit")
    nop()               .side(1)    [14]
    jmp(x_dec, "digit") .side(1)    [14]

    # Generate a space and then loop back to process more bits in the OSR.

    set(x, 15)          .side(0)    [7]
    label("space")
    jmp(x_dec, "space") .side(0)    [14]
    jmp("next")         .side(0)

    # Generate a very short blip as a "heartbeat" indicator.

    label("blip")
    nop()               .side(1)
    jmp("get_next")     .side(1)

# Creates a `StateMachine` that runs the sump monitor logic.

def create_sump_sm(idx, state, status):
    state_pin = Pin(state, mode=Pin.IN, pull=Pin.PULL_UP)
    status_pin = Pin(status, mode=Pin.OUT, pull=Pin.PULL_UP, value=0)

    return rp2.StateMachine(idx, monitor_sump, freq=2000, sideset_base=status_pin, jmp_pin=state_pin)

# Initialize global resources.

# `led` controls the onboard LED. On the Pico W, the PIO modules can't
# control this LED, so we'll use it for low-speed indication. In this
# script, it's use to indicate we are connected to the WiFi.

led = Pin("LED", Pin.OUT)

# `status_pin` is connected to a red LED. It's used by the state machine
# to show the flashing heatbeat or, is an error is reported, the value
# of the error code by using long and short pulses.

status_pin = Pin("GP19", mode=Pin.OUT, pull=Pin.PULL_UP, value=0)
sm_status = rp2.StateMachine(0, status, freq=2000, sideset_base=status_pin)

# Create two state machines, each which monitor a sump pump input.

sm_sump1 = create_sump_sm(5, "GP12", "GP13")
sm_sump2 = create_sump_sm(6, "GP14", "GP15")

# Start up the state machines.

sm_status.active(1)
sm_sump1.active(1)
sm_sump2.active(1)

# The main routine for the secondary thread. This thread polls the status
# pin and sends any changes to the main thread through an IPC.

def sump_monitor():
    global sump_state
    global sump_status
    
    last_value = None
    base_time = time.time_ns()
    
    # Infinitely loop.
    
    while True:
        # Poll the input at 20 Hz.
        
        time.sleep_ms(50)

        # Toggle the LED that indicates we're sampling. Sleep for 20
        # milliseconds so the LED will be on long enough to see and to
        # let the relay contacts debounce a bit.

        sump_status.on()
        tmp = sump_state.value()
        time.sleep_ms(5)
        if tmp == 1:
            sump_status.off()
    
        # If the pin state has changed, check to see if it is still different.
        # If so, reset the timeout, save the new state and report it.
    
        if tmp != last_value:
            if tmp == sump_state.value():
                last_value = tmp
                base_time = time.time_ns()
                continue

        # Nothing interesting happened with the input, so see if 5 seconds has
        # elapsed. If so, send a keep alive message.

        if time.time_ns() - base_time >= 5000000000:
            base_time += 5000000000

# Global variable used by `set_status` so it doesn't send the same status
# over and over to the PIO module (once the PIO FIFO gets full, the Python
# script will block adding new, same values.)

prev_code = -1

# Takes an integer from 0 to 99 and converts it into the stream of bits
# required by the status state machine. If the code is zero, it clears the
# state machine back to the heartbeat mode.

def set_status(code):
    global sm_status
    global prev_code

    if prev_code != code:
        prev_code = code
        if code == 0:
            sm_status.put(0)
        elif code > 0 and code < 100:
            tens = code // 10
            ones = code % 10
            sm_status.put((1 << (tens + ones)) + (((1 << tens) - 1) << ones))

# ****************************************************************************
# Entry point
# ****************************************************************************

def main():
    socket = False

    # Initialize the WiFi hardware.

    wlan = network.WLAN(network.STA_IF)
    wlan.active(True)
    wlan.connect('*****', '*****')
    led.off()
    
    #_thread.start_new_thread(sump_monitor, ())

    # Infinite loop.

    while True:

        # Update global state based on the current network condition. If we
        # have an IP address, then set up the socket.

        status = wlan.status()

        # If the status is 3, we are connected to Wifi. Any other status means
        # we're either trying to connect or an error occurred. Since the status
        # codes can be positive or negative, we translate them into positive
        # values for the PIO.
        #
        # The values are:
        #   11 = link down
        #   12 = joining wifi network
        #   13 = joined, but no IP assigned yet
        #   14 = the WiFi link failed
        #   15 = no network to join
        #   16 = bad SSID/password combo

        if status == 3:
            led.on()
            if socket == False:
                set_status(0)
                socket = True
        else:
            led.off()
            socket = False
            if status < 0:
                set_status(-status + 13)
            else:
                set_status(status + 11)

        time.sleep_ms(250)

main()
