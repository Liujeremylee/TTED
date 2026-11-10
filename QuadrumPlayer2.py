#Written by Microsoft CoPilot under the instruction and supervision of J. Brett
#For use with the Quadrum LED Cube Code Generator by geckoAnton https://geckoanton.github.io/quadrum/download.html
#and the CubeLib.py LED cube library by J.Brett

#******************************* Initialization ******************************

from machine import Pin, Timer
from time import sleep_ms
import struct
import CubeLib

button1 = Pin(13, Pin.IN, Pin.PULL_UP)
button2 = Pin(10, Pin.IN, Pin.PULL_UP)
button3 = Pin(11, Pin.IN, Pin.PULL_UP)
button4 = Pin(12, Pin.IN, Pin.PULL_UP)

sleep_ms(1000)

leds = CubeLib.Cube(5)
CubeLib.leds = leds

leds.xy_map = (
    27,25,24,19,16,
    28,30,20,22,23,
    29,31,21,17,4,
    13,26,3,1,0,
    15,14,5,6,7
)

leds.layer_map = (9,10,11,8,12)

timer = Timer()
timer.init(
    freq=3000,
    mode=Timer.PERIODIC,
    callback=CubeLib.refresh_layers
)

#******************************* Quadrum Loader ******************************

class QuadrumAnimation:

    def __init__(self, filename):

        self.size = 0
        self.bits_per_voxel = 0
        self.frame_count = 0

        self.frames = []
        self.delays = []

        with open(filename, "r") as f:
            text = f.read()

        raw = self.extract_bytes(text)

        print("Bytes extracted:", len(raw))

        self.parse(raw)

    def extract_bytes(self, text):

        values = []

        pos = 0

        while True:

            pos = text.find("0x", pos)

            if pos == -1:
                break

            values.append(
                int(text[pos + 2:pos + 4], 16)
            )

            pos += 4

        return bytes(values)

    def remap_index(self, voxel):

        x = voxel % self.size
        y = (voxel // self.size) % self.size
        z = voxel // (self.size * self.size)

        nx = x
        ny = self.size - 1 - z
        nz = y

        return (
            nx +
            ny * self.size +
            nz * self.size * self.size
        )
    
    def parse(self, raw):

        if len(raw) < 5:
            raise ValueError("Invalid Quadrum file")

        self.size = raw[0]

        flag = raw[1]
        self.bits_per_voxel = raw[2]
        self.frame_count = raw[3]

        voxels = self.size * self.size * self.size

        print("Cube Size:", self.size)
        print("Bits per Voxel:", self.bits_per_voxel)
        print("Frames:", self.frame_count)

        if self.bits_per_voxel == 1:

            bitmap_bytes = (voxels + 7) // 8

        elif self.bits_per_voxel == 4:

            bitmap_bytes = (voxels + 1) // 2

        else:

            raise ValueError(
                "Unsupported bits-per-voxel format: {}".format(
                    self.bits_per_voxel
                )
            )

        print("Bitmap bytes per frame:", bitmap_bytes)

        ptr = 5

        for frame_number in range(self.frame_count):

            if ptr + 4 + bitmap_bytes > len(raw):

                print("Unexpected EOF at frame", frame_number)
                break

            #
            # Read frame delay
            #
            delay_ms = struct.unpack(
                "<f",
                raw[ptr:ptr+4]
            )[0]

            ptr += 4

            #
            # Read frame bitmap
            #
            bitmap = raw[ptr:ptr+bitmap_bytes]

            ptr += bitmap_bytes

            frame = bytearray(voxels)

            #
            # Monochrome format
            #
            if self.bits_per_voxel == 1:

                for voxel in range(voxels):

                    byte_index = voxel >> 3
                    bit_index = voxel & 7

                    if bitmap[byte_index] & (1 << bit_index):
                        value = 15
                    else:
                        value = 0

                    frame[self.remap_index(voxel)] = value

            #
            # 4-bit grayscale format
            #
            elif self.bits_per_voxel == 4:

                for voxel in range(voxels):

                    byte_index = voxel >> 1

                    #
                    # Low nibble first
                    #
                    if voxel & 1:
                        value = (bitmap[byte_index] >> 4) & 0x0F
                    else:
                        value = bitmap[byte_index] & 0x0F

                    frame[self.remap_index(voxel)] = value

            self.frames.append(frame)
            self.delays.append(int(delay_ms))

        print("Frames loaded:", len(self.frames))

    def show_frame(self, frame_number):

        leds.data[:] = self.frames[frame_number]

        return self.delays[frame_number]


#******************************* Load Animation ******************************

ANIMATION_FILE = "TTED.txt"

print("Loading animation...")

anim = QuadrumAnimation(ANIMATION_FILE)

print("Animation ready")

#******************************* Play Animation ******************************

while True:

    for frame in range(len(anim.frames)):

        leds.data[:] = anim.frames[frame]

        sleep_ms(anim.delays[frame])