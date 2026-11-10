#This is the library for driving the multiplexed display of a common cathode LED Cube using HC595 shift registers
#	the shift register outputs also control a higher current driver for providing power to each layer.
#The columns (and layers) of the cube are mapped to the appropriate bits of the SPI data stream controlling the 595 ICs.
#	this allows the design of the board to be optimized to place the various columns and layers to the nearest 595,
#	rather than having to match the columns in a 1:1 relationship to the outputs of the 595
#	This should allow for using this diver routine with various board designs and sizes of LED "Cubes" or other multiplexed displays.
#
#Code is by Jason Brett (jbrett@bcit.ca) developed for use in BCIT TTED4060 "Teaching Electronics 1" class
#Code is published under the GNU General Public License v3.0
#
#Nov. 2026: added the hardware refresh engine (PIO + DMA). The LEDs are now refreshed entirely by the RP2350's
#	PIO and DMA hardware, so the CPU spends ZERO time per layer. The CPU only rebuilds a small table of
#	precomputed 32-bit words (one per layer per brightness step) a few hundred times a second, which takes ~10uS.
#
#	Commands you can use from your code:
#
#	Required initialization routines:
#		leds=CubeLib.Cube(5)			#creates an initializes a cube object called "leds" (you can call it whatever you want) of size 5x5x5
#		CubeLib.leds = leds				#(optional now - Cube() does this for you - but old code that has this line still works)
#		leds.xy_map=(27,25,24,19,16, 28,30,20,22,23, 29,31,21,17,4, 13,26,3,1,0, 15,14,5,6,7)
#										maps the x (rows) and y (columns) of your cube to the bits in the SPI bitstream that will set them HIGH (off) or LOW (on)
#		leds.layer_map=(9,10,11,8,12)	#maps the layers of your cube to the bits in the SPI bitstream that will set them HIGH (on) or LOW (off)
#
#	To activate the cube's display (NEW, fast hardware refresh):
#		leds.start()					#starts the PIO/DMA refresh as fast as the 595s allow, overclocks the CPU to 240MHz
#		leds.start(refresh_hz=2000)		#or pick a full-brightness-cycle rate, if you see ghosting between layers
#		leds.stop()						#stops the hardware refresh and turns the layers off
#		print(leds.info())				#shows the actual refresh rates and how long the CPU spends rebuilding the table
#
#	The OLD way still works (and is how CubeTest steps through the layers by hand):
#		CubeLib.refresh_layers(0)		#displays one layer at one brightness step using the CPU and SPI
#		timer.init(freq=3000, mode=Timer.PERIODIC, callback=CubeLib.refresh_layers)
#
#	To create your animation you can use:
#		leds.set(x,y,z,6)			#this gives the x(row), y(col) and z (layer) coordinates of the LED you want to set, and the brightness you want to set it to
#										0 is off, 10 (leds.levels) is "full bright". Anything above leds.levels is also full bright.
#		leds.dim()					#dims all LEDs in the cube by one brightness level
#		leds.clear()				#sets all LEDs in the cube to "off"
#		leds.fill(6)				#sets all LEDs in the cube to the brightness level in the brackets
#		You can also write straight into leds.data (125 bytes, index = z*25 + y*5 + x) - the hardware refresh picks it up.

from array import array
from machine import Pin, SPI, Timer
import machine
import micropython
import rp2
import time
import uctypes

layer=0					#used to keep track of which layer the refresh_layers function is displaying
pwm_value=1             #used to control the brightness of the leds in the refresh_layers function
leds=None               #the Cube object (set automatically when you create one)

SCK_PIN = 2
MOSI_PIN = 3
LATCH_PIN = 1

# Initialize SPI0 (SCK=GP2, MOSI=GP3) - only used by the old CPU-driven refresh_layers()
spi = SPI(0, baudrate=20000000, polarity=0, phase=0, sck=Pin(SCK_PIN), mosi=Pin(MOSI_PIN))
latch = Pin(LATCH_PIN, Pin.OUT)
#NOTE: the old timing pin on GP10 has been removed - GP10 is button2 on the TTED board.

# ---------------------------------------------------------------------------------------------------------------
# PIO program: shifts one 32-bit word MSB first into the 595 chain, pulses the latch, then holds the outputs for
# a programmable time before the next word. SCK = side-set pin, data = OUT pin, latch = SET pin.
# Every bit takes 2 PIO cycles, so SCK = PIO clock / 2.
# ---------------------------------------------------------------------------------------------------------------
@rp2.asm_pio(out_init=rp2.PIO.OUT_LOW, sideset_init=rp2.PIO.OUT_LOW, set_init=rp2.PIO.OUT_LOW,
             out_shiftdir=rp2.PIO.SHIFT_LEFT, autopull=False, fifo_join=rp2.PIO.JOIN_TX)
def _shift595():
    pull()                .side(0)      # runs once: first word is the hold time (in PIO cycles)
    mov(isr, osr)         .side(0)      # park it in the ISR, which this program never otherwise uses
    wrap_target()
    pull()                .side(0)      # get the next layer word from DMA
    set(x, 31)            .side(0)
    label("bit")
    out(pins, 1)          .side(0)      # data changes while SCK is low...
    jmp(x_dec, "bit")     .side(1)      # ...and is clocked in on SCK's rising edge
    set(pins, 1)          .side(0) [1]  # latch: all 595 outputs change at once (50nS+ pulse)
    set(pins, 0)          .side(0)
    mov(y, isr)           .side(0)
    label("hold")
    jmp(y_dec, "hold")    .side(0)      # keep this layer lit while... (next word is already waiting in the FIFO)
    wrap()

_PIO_FIXED_CYCLES = 72      # cycles per word with a hold of 0: pull + set + 64 bit cycles + 3 latch + mov + 1 hold
_PIO0_TXF0 = 0x50200010     # PIO0 TX FIFO 0 address (same on RP2040 and RP2350)
_DMA_BASE = 0x50000000
_DREQ_PIO0_TX0 = 0
_SM_ID = 0                  # PIO0, state machine 0

# ---------------------------------------------------------------------------------------------------------------
# Table builder in inline Thumb assembly (viper array access costs ~36 cycles per word on this firmware; this is ~3).
# An LED is OFF (its column bit HIGH) in brightness slot s when its value <= s.
# For each layer: bucket every LED's mask by its value (one pass), then a running OR over the slots gives the
# "off" bits for every slot. That is n*n + levels steps per layer instead of n*n*levels.
#   r0 = leds.data, r1 = masks, r3 = params:
#   [0]=n [4]=levels [8]=n*n [12]=&lbits [16]=&tmp [20]=z (scratch) [24]=n*4 [28]=&frame
# ---------------------------------------------------------------------------------------------------------------
@micropython.asm_thumb
def _build_asm(r0, r1, r2, r3):
    mov(r7, 0)
    str(r7, [r3, 20])           # z = 0
    label(LAYER)
    ldr(r4, [r3, 16])           # clear tmp[0..levels-1]
    ldr(r5, [r3, 4])
    mov(r6, 0)
    label(CLR)
    str(r6, [r4, 0])
    add(r4, 4)
    sub(r5, 1)
    bne(CLR)
    ldr(r4, [r3, 16])           # r4 = tmp
    ldr(r5, [r3, 8])            # r5 = n*n LEDs in this layer
    mov(r6, r1)                 # r6 = mask pointer
    push({r1, r2})
    label(BUCKET)
    ldrb(r7, [r0, 0])           # v = data[...]
    add(r0, 1)
    ldr(r1, [r6, 0])            # mask
    add(r6, 4)
    ldr(r2, [r3, 4])            # levels
    cmp(r7, r2)
    bge(SKIP)                   # v >= levels: always on, no off-bit
    lsl(r7, r7, 2)
    add(r7, r7, r4)
    ldr(r2, [r7, 0])
    orr(r2, r1)
    str(r2, [r7, 0])            # tmp[v] |= mask
    label(SKIP)
    sub(r5, 1)
    bne(BUCKET)
    pop({r1, r2})
    ldr(r2, [r3, 28])           # r2 = &frame[z] (slot 0)
    ldr(r7, [r3, 20])
    lsl(r6, r7, 2)
    add(r2, r2, r6)
    ldr(r6, [r3, 12])           # r6 = lbits[z]
    add(r6, r6, r7)
    add(r6, r6, r7)
    add(r6, r6, r7)
    add(r6, r6, r7)
    ldr(r6, [r6, 0])
    ldr(r4, [r3, 16])           # r4 = tmp
    ldr(r5, [r3, 4])            # r5 = levels
    mov(r7, 0)                  # r7 = acc
    push({r0, r1})
    ldr(r1, [r3, 24])           # r1 = stride between slots (n words)
    label(ACC)
    ldr(r0, [r4, 0])
    add(r4, 4)
    orr(r7, r0)                 # acc |= tmp[s]
    mov(r0, r7)
    orr(r0, r6)
    str(r0, [r2, 0])            # frame[s*n + z] = acc | layer bit
    add(r2, r2, r1)
    sub(r5, 1)
    bne(ACC)
    pop({r0, r1})
    ldr(r7, [r3, 20])           # next layer
    add(r7, 1)
    str(r7, [r3, 20])
    ldr(r5, [r3, 0])
    cmp(r7, r5)
    blt(LAYER)


class Cube:					#This is the "Cube" object class that will be called from your code using something like "leds=CubeLib.Cube(5)"
    def __init__(self, size, levels=10):
        global leds
        self.size = size
        self.levels = levels                                #number of brightness steps (0=off ... levels=full)
        self.data = array('B', [0] * (size * size * size))  #brightness of each LED in the cube
        self.xy_map = array('B', [0] * (size * size))       #cube columns -> bit in the 595 bitstream
        self.layer_map = array('B',[0] * size)              #cube layers -> bit in the 595 bitstream
        self.bytes_required=(((size*size)+size)//8)+1
        self.buf = bytearray(self.bytes_required)
        self._masks = array('I', [0] * (size * size))       #1 << xy_map[i], precomputed
        self._lbits = array('I', [0] * size)                #1 << layer_map[z], precomputed
        self._tmp = array('I', [0] * levels)                #scratch for _build
        self._frame = array('I', [0] * (levels * size))     #the table the DMA streams to the PIO: slot-major, layer-minor
        if levels < 1:
            raise ValueError("levels must be at least 1")
        self._params = array('I', [size, levels, size * size, uctypes.addressof(self._lbits),
                                   uctypes.addressof(self._tmp), 0, size * 4, uctypes.addressof(self._frame)])
        self._running = False
        self._sm = None
        self._dma = None
        self._rebuild_timer = None
        self.build_us = 0
        leds = self                                         #so CubeLib.leds = leds is no longer required

    def _update_maps(self):
        for i in range(self.size * self.size):
            self._masks[i] = 1 << self.xy_map[i]
        for z in range(self.size):
            self._lbits[z] = 1 << self.layer_map[z]

    @micropython.native
    def _index(self, x, y, z):
        if not (0 <= x < self.size and 0 <= y < self.size and 0 <= z < self.size):
            raise IndexError("Coordinates out of bounds")
        return z * self.size * self.size + y * self.size + x

    @micropython.native
    def set(self, x, y, z, value):
        self.data[self._index(x, y, z)] = value

    @micropython.native
    def get(self, x, y, z):
        return self.data[self._index(x, y, z)]

    @micropython.viper
    def _fill(self, value:int):
        d = ptr8(self.data)
        n:int = int(len(self.data))
        for i in range(n):
            d[i] = value

    def clear(self):
        self._fill(0)

    def fill(self, value=1):
        self._fill(value)

    @micropython.viper
    def _dim(self, step:int):
        d = ptr8(self.data)
        n:int = int(len(self.data))
        for i in range(n):
            v:int = d[i]
            d[i] = v - step if v >= step else 0

    def dim(self,step=1):
        self._dim(step)

    def layer(self, z):
        """Returns a 2D list of a single Z-layer"""
        return [[self.get(x, y, z) for x in range(self.size)] for y in range(self.size)]

    def __str__(self):
        out = ""
        for z in range(self.size):
            out += f"Layer {z}:\n"
            for y in range(self.size):
                row = [str(self.get(x, y, z)) for x in range(self.size)]
                out += " ".join(row) + "\n"
            out += "\n"
        return out

    def _build(self):                       #rebuild the whole refresh table from leds.data (see _build_asm)
        _build_asm(self.data, self._masks, self._frame, self._params)

    def _rebuild(self, t=None):
        t0 = time.ticks_us()
        self._build()
        self.build_us = time.ticks_diff(time.ticks_us(), t0)

    # -----------------------------------------------------------------------------------------------------------
    # Hardware refresh engine
    # -----------------------------------------------------------------------------------------------------------
    def start(self, refresh_hz=None, cpu_hz=240_000_000, spi_hz=20_000_000, rebuild_hz=250):
        """Start the PIO + DMA refresh.
        refresh_hz : full brightness cycles per second (all layers x all brightness steps). None = as fast as possible.
        cpu_hz     : CPU clock. 240MHz is a safe overclock for the RP2350 (stock is 150MHz) and divides by exactly 6
                     to give the PIO a jitter-free 40MHz (= 20MHz 595 clock). None = leave it alone.
        spi_hz     : 595 shift clock. 20MHz is what the original SPI code used.
        rebuild_hz : how often the CPU rebuilds the table from leds.data (sets how quickly changes appear)."""
        if (self.size * self.size + self.size) > 32:
            raise ValueError("hardware refresh supports up to 32 bits (5x5x5 uses 30)")
        self.stop()
        if cpu_hz:
            machine.freq(cpu_hz)
        sys_hz = machine.freq()
        self._update_maps()
        self._rebuild()                 # first table, and records build_us for info()

        # PIO clock: an integer divider of the system clock (no jitter), at most 2 x spi_hz
        div = -(-sys_hz // (2 * spi_hz))
        self.pio_hz = sys_hz // div
        self.sck_hz = self.pio_hz // 2
        slots_per_cycle = self.levels * self.size
        if refresh_hz:
            hold = self.pio_hz // (refresh_hz * slots_per_cycle) - _PIO_FIXED_CYCLES
            if hold < 0:
                hold = 0
        else:
            hold = 0
        self._hold = hold
        cycles_per_slot = _PIO_FIXED_CYCLES + hold
        self.slot_us = cycles_per_slot * 1_000_000 / self.pio_hz
        self.layer_hz = self.pio_hz / (cycles_per_slot * self.size)
        self.refresh_hz = self.pio_hz / (cycles_per_slot * slots_per_cycle)

        sm = rp2.StateMachine(_SM_ID, _shift595, freq=self.pio_hz,
                              sideset_base=Pin(SCK_PIN), out_base=Pin(MOSI_PIN), set_base=Pin(LATCH_PIN))
        sm.put(hold)                    # first word = hold time
        sm.active(1)
        self._sm = sm

        # DMA: channel "data" streams the table into the PIO TX FIFO, paced by the PIO.
        # When it finishes it chains to channel "ctrl", which rewrites data's read address (a trigger register),
        # restarting it. The loop runs forever with no CPU involvement.
        data_ch = rp2.DMA()
        ctrl_ch = rp2.DMA()
        self._frame_addr = array('I', [uctypes.addressof(self._frame)])
        data_trig = _DMA_BASE + data_ch.channel * 0x40 + 0x3C        # CHx_AL3_READ_ADDR_TRIG
        ctrl_ch.config(read=uctypes.addressof(self._frame_addr), write=data_trig, count=1,
                       ctrl=ctrl_ch.pack_ctrl(size=2, inc_read=False, inc_write=False, treq_sel=0x3F,
                                              chain_to=ctrl_ch.channel, irq_quiet=True))
        data_ch.config(read=self._frame, write=_PIO0_TXF0 + 4 * _SM_ID, count=len(self._frame),
                       ctrl=data_ch.pack_ctrl(size=2, inc_read=True, inc_write=False,
                                              treq_sel=_DREQ_PIO0_TX0 + _SM_ID,
                                              chain_to=ctrl_ch.channel, irq_quiet=True),
                       trigger=True)
        self._dma = (data_ch, ctrl_ch)

        self._rebuild_timer = Timer(freq=rebuild_hz, mode=Timer.PERIODIC, callback=self._rebuild)
        self._running = True

    def stop(self):
        """Stop the hardware refresh and switch every layer off."""
        if self._rebuild_timer:
            self._rebuild_timer.deinit()
            self._rebuild_timer = None
        if self._dma:
            for ch in self._dma:
                ch.ctrl = 0             # disable first, so the chain can't restart it
            for ch in self._dma:
                ch.close()
            self._dma = None
        if self._sm:
            self._sm.active(0)
            self._sm = None
            spi.init(baudrate=20000000, polarity=0, phase=0, sck=Pin(SCK_PIN), mosi=Pin(MOSI_PIN))
            latch.init(Pin.OUT)
            self.buf[:] = b'\x00' * len(self.buf)      # all layers off
            latch.value(0); spi.write(self.buf); latch.value(1)
        self._running = False

    def info(self):
        if not self._running:
            return "hardware refresh is stopped"
        return ("CPU %dMHz | 595 clock %.1fMHz | slot %.2fuS | layer rate %dHz | full PWM cycle %dHz | table rebuild %duS"
                % (machine.freq() // 1_000_000, self.sck_hz / 1e6, self.slot_us, self.layer_hz, self.refresh_hz, self.build_us))

    # -----------------------------------------------------------------------------------------------------------
    # Old CPU-driven path: show one layer at one brightness step (used by CubeTest to step layers by hand)
    # -----------------------------------------------------------------------------------------------------------
    @micropython.viper
    def display_layer(self, layer:int, pwm_value:int):
        bits:int = 1 << int(self.layer_map[layer])
        size:int = int(self.size)
        base:int = layer * size * size
        data = ptr8(self.data)
        for i in range(size * size):
            if data[base + i] <= pwm_value:
                bits |= 1 << int(self.xy_map[i])        #column HIGH = LED off
        buf = ptr8(self.buf)
        buf[0] = (bits >> 24) & 0xFF
        buf[1] = (bits >> 16) & 0xFF
        buf[2] = (bits >> 8) & 0xFF
        buf[3] = bits & 0xFF
        latch.value(0)
        spi.write(self.buf)
        latch.value(1)

@micropython.native
def refresh_layers(t):      #the old timer-driven refresh: one layer per call
    global layer
    global pwm_value
    leds.display_layer(layer,pwm_value)
    layer=layer+1
    if (layer>=leds.size):
        layer=0
        pwm_value=pwm_value+1
        if (pwm_value>=leds.levels):
            pwm_value=0
