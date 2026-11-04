#!/usr/bin/env python3
"""A cable helper for a browser without Web Serial: one WebSocket on 127.0.0.1:8765 carries bytes
to and from the badge's serial port. Run `python3 scripts/cable-helper.py /dev/cu.usbmodem*` and
open the app with `?cable=ws://127.0.0.1:8765` in its address; the app's "Add the badge on USB"
then talks to this socket instead of asking the browser for a port. Needs the `websockets` package.
"""
import asyncio, os, sys, termios, tty, fcntl, struct
import websockets

PORT = sys.argv[1]

def open_tty(path):
    fd = os.open(path, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    attrs = termios.tcgetattr(fd)
    tty.setraw(fd)
    attrs = termios.tcgetattr(fd)
    attrs[6][termios.VMIN] = 0
    attrs[6][termios.VTIME] = 0
    termios.tcsetattr(fd, termios.TCSANOW, attrs)
    # Raise DTR, as a serial program does when it opens the port: the badge takes that as the host.
    TIOCMBIS = 0x8004746C
    TIOCM_DTR = 0x002
    try:
        fcntl.ioctl(fd, TIOCMBIS, struct.pack('I', TIOCM_DTR))
    except OSError as e:
        print("cable: DTR not raised:", e, flush=True)
    return fd

async def serve(ws):
    fd = open_tty(PORT)
    print("cable: port open, browser attached", flush=True)
    stop = asyncio.Event()
    async def pump_out():
        while not stop.is_set():
            try:
                data = os.read(fd, 4096)
            except BlockingIOError:
                data = b""
            if data:
                await ws.send(data)
            else:
                await asyncio.sleep(0.005)
    task = asyncio.create_task(pump_out())
    try:
        async for msg in ws:
            if isinstance(msg, (bytes, bytearray)):
                view = memoryview(msg)
                while len(view):
                    try:
                        n = os.write(fd, view)
                    except BlockingIOError:
                        await asyncio.sleep(0.005)
                        continue
                    view = view[n:]
    finally:
        stop.set()
        await task
        os.close(fd)
        print("cable: browser detached", flush=True)

async def main():
    async with websockets.serve(serve, "127.0.0.1", 8765, max_size=None):
        print("cable: listening on ws://127.0.0.1:8765", flush=True)
        await asyncio.Future()

asyncio.run(main())
