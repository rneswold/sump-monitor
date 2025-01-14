# drmem-drv-sump

This project provides a Python script that can be loaded on a
RaspberryPi Pico W or Pico 2 W. It uses two pairs of GPIO pins to
monitor and show the state of two sump pumps. It also uses a GPIO pin to
control an LED that reports any error condition (number of long pulses
for ten's digit followed by number of short pulses for one's digit.)
Lastly, it uses a GPIO pin to control an LED that indicates a client has
connected over the network and is receiving sump pump status.

The DrMem control system has a driver that understands this protocol.

## Design

A major requirement is the sump pump cannot be affected by the presence
or absense of the RaspberryPi -- it must be able to operate at all
times. This was accomplished by using a "current switch" (I am a happy
customer of the Dwyer MCS-111050, but there are other, similar
products.) I ran the hot wire through the current switch so when the
sump pump runs, the relay closes.

For my battery-backed sump pump, I connected the secondary monitor using
the relay outputs that are on the battery-backed controller.
