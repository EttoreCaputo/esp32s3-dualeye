#!/usr/bin/env python3
"""Host side of the M0 audio self-test (firmware: main/audio_selftest.c).

Close the DualEye app / `dualeye` bridge first: they hold the same port.
Needs pyserial (the ESP-IDF Python env has it).

  audio_selftest.py tone                 chime on the speaker
  audio_selftest.py tone 440 1000        440 Hz for 1 s
  audio_selftest.py rec 5                record 5 s, save WAVs, print per-channel levels
  audio_selftest.py rec 3 --tone         same while the speaker plays 1 kHz (finds the AEC reference)
  audio_selftest.py rec 5 --beep --play 0  beep ("speak now"), record, play channel 0 back
  audio_selftest.py play 0               play channel 0 of the last recording (same boot only)
  audio_selftest.py vol 70 | gain 30 | stats
"""

import argparse
import base64
import math
import os
import struct
import sys
import time
import wave
import zlib

try:
    import serial
    from serial.tools import list_ports
except ImportError:
    sys.exit("pyserial missing: pip install pyserial (or run from the ESP-IDF Python env)")

ESPRESSIF_VID = 0x303A
TEST_TONE_HZ = 1000


def find_port():
    ports = [p.device for p in list_ports.comports()
             if p.vid == ESPRESSIF_VID and not p.device.startswith("/dev/tty.")]
    if len(ports) != 1:
        sys.exit(f"expected one DualEye board, found {ports or 'none'}; pass --port")
    return ports[0]


def open_port(name):
    # Don't toggle DTR/RTS: on the S3's USB Serial/JTAG that resets the chip.
    s = serial.Serial()
    s.port = name
    s.baudrate = 115200
    s.timeout = 0.5
    s.dtr = False
    s.rts = False
    s.open()
    settle(s)
    return s


def settle(port):
    """macOS raises DTR/RTS on open anyway, which resets the board: wait out the boot
    (until the log goes quiet) so the first command isn't lost."""
    deadline = time.monotonic() + 5
    quiet_since = time.monotonic()
    while time.monotonic() < deadline:
        if port.readline():
            quiet_since = time.monotonic()
        elif time.monotonic() - quiet_since > 0.6:
            return


def run(port, cmd, timeout):
    """Send one command, echo board output, return (ok, collected AUD lines)."""
    port.write(f"!audio {cmd}\n".encode())
    lines = []
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        raw = port.readline()
        if not raw:
            continue
        line = raw.decode(errors="replace").rstrip("\r\n")
        if line.startswith("AUD:D "):
            lines.append(line)
            continue
        if line.startswith("AUD:"):
            lines.append(line)
            if line.startswith("AUD:OK") or line.startswith("AUD:ERR"):
                print(line)
                return line.startswith("AUD:OK"), lines
            if not line.startswith(("AUD:BEGIN", "AUD:END")):
                print(line[4:])
        else:
            print(f"  board: {line}")
        deadline = max(deadline, time.monotonic() + 2)
    sys.exit(f"timed out waiting for the board on '{cmd}'")


def decode_dump(lines):
    begin = next(l for l in lines if l.startswith("AUD:BEGIN"))
    end = next(l for l in lines if l.startswith("AUD:END"))
    meta = dict(kv.split("=") for kv in begin.split()[1:])
    data = b"".join(base64.b64decode(l[6:]) for l in lines if l.startswith("AUD:D "))
    want = int(meta["bytes"])
    if len(data) != want:
        sys.exit(f"dump truncated: {len(data)} of {want} bytes")
    crc = int(end.split("crc32=")[1], 16)
    if zlib.crc32(data) != crc:
        sys.exit("dump CRC mismatch")
    return int(meta["rate"]), int(meta["channels"]), data


def goertzel(samples, rate, hz):
    k = 2 * math.cos(2 * math.pi * hz / rate)
    s1 = s2 = 0.0
    for x in samples:
        s1, s2 = x + k * s1 - s2, s1
    power = s1 * s1 + s2 * s2 - k * s1 * s2
    return math.sqrt(max(power, 0.0)) * 2 / len(samples)


def dbfs(v):
    return 20 * math.log10(v / 32768) if v > 0 else -120.0


def save_and_analyse(rate, channels, data, out_dir, tone):
    os.makedirs(out_dir, exist_ok=True)
    count = len(data) // 2
    samples = struct.unpack(f"<{count}h", data)
    frames = count // channels

    with wave.open(os.path.join(out_dir, "rec_all.wav"), "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(data)

    print(f"\n{frames / rate:.1f} s at {rate} Hz, {channels} channels -> {out_dir}/")
    header = f"{'ch':>3} {'rms dBFS':>9} {'peak dBFS':>10} {'clip':>5}"
    if tone:
        header += f" {'1 kHz dBFS':>11}"
    print(header)
    for ch in range(channels):
        chan = samples[ch::channels]
        with wave.open(os.path.join(out_dir, f"rec_ch{ch}.wav"), "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(rate)
            w.writeframes(struct.pack(f"<{len(chan)}h", *chan))
        mean = sum(chan) / len(chan)
        rms = math.sqrt(sum((x - mean) ** 2 for x in chan) / len(chan))
        peak = max(abs(x) for x in chan)
        clip = sum(1 for x in chan if abs(x) >= 32767)
        row = f"{ch:>3} {dbfs(rms):>9.1f} {dbfs(peak):>10.1f} {clip:>5}"
        if tone:
            # Skip the first 100 ms: stale DMA data from before the tone started.
            row += f" {dbfs(goertzel(chan[rate // 10:], rate, TEST_TONE_HZ)):>11.1f}"
        print(row)
    if tone:
        print("\nThe AEC reference is the channel with a strong, clean 1 kHz and ~no room noise;"
              "\nthe mic channel(s) hear the tone quieter, plus the room.")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--port", help="serial port (default: the one Espressif board attached)")
    ap.add_argument("--out", default="audio_capture", help="where rec saves WAVs (default ./audio_capture)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    t = sub.add_parser("tone")
    t.add_argument("hz", nargs="?", type=int)
    t.add_argument("ms", nargs="?", type=int)
    r = sub.add_parser("rec")
    r.add_argument("seconds", type=int)
    r.add_argument("--tone", action="store_true", help="play 1 kHz while recording")
    r.add_argument("--beep", action="store_true", help="beep first, as a 'speak now' cue")
    r.add_argument("--play", type=int, metavar="CH", help="then play channel CH back on the speaker")
    p = sub.add_parser("play")
    p.add_argument("ch", nargs="?", type=int, default=0)
    v = sub.add_parser("vol")
    v.add_argument("level", type=int)
    g = sub.add_parser("gain")
    g.add_argument("db", type=float)
    sub.add_parser("stats")
    args = ap.parse_args()

    port = open_port(args.port or find_port())
    if args.cmd == "tone":
        cmd = f"tone {args.hz} {args.ms}" if args.hz and args.ms else "tone"
        run(port, cmd, 10)
    elif args.cmd == "rec":
        ok, lines = run(port, f"rec {args.seconds}{' tone' if args.tone else ' beep' if args.beep else ''}", args.seconds + 30)
        if ok:
            save_and_analyse(*decode_dump(lines), args.out, args.tone)
            if args.play is not None:
                print(f"\nplaying channel {args.play} back...")
                run(port, f"play {args.play}", args.seconds + 15)
    elif args.cmd == "play":
        run(port, f"play {args.ch}", 15)
    elif args.cmd == "vol":
        run(port, f"vol {args.level}", 5)
    elif args.cmd == "gain":
        run(port, f"gain {args.db}", 5)
    elif args.cmd == "stats":
        run(port, "stats", 5)


if __name__ == "__main__":
    main()
