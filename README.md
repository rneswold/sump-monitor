# drmem-drv-sump

## Introduction

This project contains the source for a RaspberryPi Pico W
microcontroller to be used in monitoring sump pumps. It uses its GPIO
to read the state of the pumps and has a WiFi interface so a DrMem
node can connect and monitor them.

A major requirement is the sump pump cannot be affected by the presence or
absense of the RaspberryPi. This was accomplished by using a "current switch"
(I am a happy customer of the Dwyer MCS-111050, but there are other, similar
products.) I ran the hot wire through the current switch so when the sump pump
runs, the relay closes. I configured GPIO4 to be an input with a pull-up
resistor so the RaspberryPi can sense the relay's state.
