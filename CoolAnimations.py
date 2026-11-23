#CoolAnimations
#Sample animations for the BCIT TTED 5x5x5 LED cube board and the CubeLib.py driver module (PIO/DMA version).
#Copy CubeLib.py and this file to the root of your Pico. Save this file as "main.py" to run it on startup.
#
#Animations: rain, ripple, spinner, fireworks, sphere pulse, corkscrew, snake. They play in a loop.
#Press button 1 (GP13) to skip to the next animation.
#
#Every animation uses the same three ideas:
#	1. leds.dim(n) at the start of each frame fades the old picture, so moving LEDs leave a glowing trail.
#	2. plot(x, y, h, brightness) lights an LED. It takes decimals (it rounds them) and ignores points outside
#	   the cube, so you can do maths with sin() and cos() without worrying about the edges.
#	3. wait(ms) is sleep_ms() that also watches button 1.

#******************************* This section MUST be included for initialization ******************************
from machine import Pin
from time import sleep_ms, ticks_ms, ticks_diff
import math
import random
import CubeLib		#The CubeLib.py module must be installed in the root directory of your Pico

button1 = Pin(13, Pin.IN, Pin.PULL_UP)
button2 = Pin(10, Pin.IN, Pin.PULL_UP)
button3 = Pin(11, Pin.IN, Pin.PULL_UP)
button4 = Pin(12, Pin.IN, Pin.PULL_UP)

sleep_ms(1000)		#gives you a chance to stop the code (Ctrl-C) before it starts looping

leds=CubeLib.Cube(5)		#creates a 5x5x5 LED Cube object
CubeLib.leds = leds
leds.xy_map=(27,25,24,19,16, 28,30,20,22,23, 29,31,21,17,4, 13,26,3,1,0, 15,14,5,6,7) #for two-sided board
leds.layer_map=(9,10,11,8,12)	#depending on how you've wired your layers this might be 12,8,11,10,9

leds.start()		#starts the PIO/DMA hardware refresh
print(leds.info())
#************************ The section above MUST be included for initialization... now you're good to code *****************

N = 5					#cube size
FULL = leds.levels		#full brightness (10)

TOP_IS_Z4 = True		#if the rain falls UP on your cube, your layers are wired the other way: set this to False

#----------------------------------------------- helpers -----------------------------------------------

class Skip(Exception):	#raised by wait() when button 1 is pressed
    pass

def wait(ms):
    """Like sleep_ms(ms), but if button 1 is pressed it skips to the next animation."""
    start = ticks_ms()
    while ticks_diff(ticks_ms(), start) < ms:
        if button1.value() == 0:			#the buttons read 0 when pressed (pull-up)
            while button1.value() == 0:		#wait for the button to be let go
                sleep_ms(10)
            raise Skip
        sleep_ms(5)

def elapsed(start):
    """Milliseconds since start (a value from ticks_ms())."""
    return ticks_diff(ticks_ms(), start)

def plot(x, y, h, brightness):
    """Light the LED at (x, y) and height h (0 = bottom layer, 4 = top layer).
    Decimals are rounded, points outside the cube are ignored, and an LED that is
    already brighter is left alone, so overlapping shapes don't darken each other."""
    x = round(x)
    y = round(y)
    h = round(h)
    if 0 <= x < N and 0 <= y < N and 0 <= h < N:
        z = h if TOP_IS_Z4 else N - 1 - h
        brightness = max(0, min(FULL, int(brightness)))
        if brightness > leds.get(x, y, z):
            leds.set(x, y, z, brightness)

#distance of every LED from the centre of the cube, worked out once (used by sphere_pulse)
CENTRE = (N - 1) / 2
DIST = [[[math.sqrt((x - CENTRE) ** 2 + (y - CENTRE) ** 2 + (h - CENTRE) ** 2)
          for h in range(N)] for y in range(N)] for x in range(N)]

#the 16 LEDs around the outside edge of one layer, in order (used by corkscrew)
RING = ([(x, 0) for x in range(4)] + [(4, y) for y in range(4)] +
        [(x, 4) for x in range(4, 0, -1)] + [(0, y) for y in range(4, 0, -1)])

#----------------------------------------------- animations -----------------------------------------------

def rain(seconds=8):
    """Drops fall from the top layer to the bottom, leaving fading trails."""
    drops = []											#each drop is [x, y, height]
    start = ticks_ms()
    while elapsed(start) < seconds * 1000:
        leds.dim(3)										#old drop positions fade out: that's the trail
        if random.random() < 0.6:						#60% chance of a new drop each frame
            drops.append([random.randrange(N), random.randrange(N), N - 1])
        for d in drops:
            plot(d[0], d[1], d[2], FULL)
            d[2] -= 1									#fall one layer
        drops = [d for d in drops if d[2] >= 0]			#forget drops that hit the bottom
        wait(70)

def ripple(seconds=8):
    """A water surface with rings spreading out from the middle, like a stone dropped in a pond."""
    phase = 0.0
    start = ticks_ms()
    while elapsed(start) < seconds * 1000:
        leds.clear()
        for x in range(N):
            for y in range(N):
                d = math.sqrt((x - CENTRE) ** 2 + (y - CENTRE) ** 2)	#distance from the middle column
                h = 2 + 2 * math.sin(d * 1.3 - phase)					#height of the water here, 0..4
                plot(x, y, h, FULL)						#the surface
                plot(x, y, h - 1, 2)					#a dim glow just under it
        phase += 0.35									#moving the phase makes the rings travel outwards
        wait(40)

def spinner(seconds=8):
    """A flat vertical sheet of light spinning around the middle, like a revolving door."""
    angle = 0.0
    start = ticks_ms()
    while elapsed(start) < seconds * 1000:
        leds.dim(4)										#leaves a short ghost behind the sheet
        c = math.cos(angle)
        s = math.sin(angle)
        for i in range(-6, 7):							#walk along the sheet in half-LED steps (long enough to reach the corners)
            r = i / 2
            for h in range(N):
                plot(CENTRE + r * c, CENTRE + r * s, h, FULL)
        angle += 0.22									#radians per frame: bigger = faster
        wait(30)

def fireworks(count=6):
    """A rocket climbs, bursts into sparks, and the sparks fall and fade under gravity."""
    for _ in range(count):
        x = random.randint(1, 3)
        y = random.randint(1, 3)
        top = random.randint(3, 4)
        for h in range(top + 1):						#the rocket climbs
            leds.dim(4)
            plot(x, y, h, FULL)
            wait(70)
        sparks = []										#each spark is [x, y, h, speed x, speed y, speed up]
        for _ in range(16):
            a = random.random() * 2 * math.pi			#random direction around
            speed = random.uniform(0.5, 1.0)
            sparks.append([x, y, top, speed * math.cos(a), speed * math.sin(a), random.uniform(-0.5, 1.0)])
        for step in range(10):
            leds.dim(2)
            for s in sparks:
                s[0] += s[3]							#move
                s[1] += s[4]
                s[2] += s[5]
                s[5] -= 0.25							#gravity pulls the spark down a bit more each frame
                plot(s[0], s[1], s[2], FULL - step)		#sparks get dimmer as they burn out
            wait(60)
        for _ in range(6):								#let the last sparks fade away
            leds.dim(2)
            wait(40)

def sphere_pulse(beats=5):
    """A glowing ball grows from the centre to the corners and shrinks back, like a heartbeat."""
    for _ in range(beats):
        sizes = [i * 0.25 for i in range(15)]			#radius 0 to 3.5 (the corners are 3.46 away)
        for r in sizes + sizes[::-1]:					#grow, then shrink
            for x in range(N):
                for y in range(N):
                    for h in range(N):
                        off = abs(DIST[x][y][h] - r)	#how far this LED is from the ball's surface
                        b = FULL - int(off * 8)			#bright on the surface, fading either side
                        z = h if TOP_IS_Z4 else N - 1 - h
                        leds.set(x, y, z, max(0, b))
            wait(25)

def corkscrew(laps=6):
    """Two comets chase each other around the outside of the cube, climbing as they go."""
    for step in range(laps * len(RING)):
        leds.dim(1)										#dim(1) gives a long tail
        for offset in (0, len(RING) // 2):				#the second comet is half a lap behind
            i = step + offset
            x, y = RING[i % len(RING)]
            h = (i // 4) % N							#climb one layer every 4 steps, then wrap to the bottom
            plot(x, y, h, FULL)
        wait(45)

def snake(seconds=10):
    """A dot wanders randomly through the cube, one LED at a time, with a fading tail."""
    p = [2, 2, 2]										#start in the middle
    start = ticks_ms()
    while elapsed(start) < seconds * 1000:
        leds.dim(1)
        axis = random.randrange(3)						#move along x, y or height
        step = random.choice((-1, 1))
        if 0 <= p[axis] + step < N:
            p[axis] += step
        else:
            p[axis] -= step								#hit a wall: bounce back the other way
        plot(p[0], p[1], p[2], FULL)
        wait(60)

#----------------------------------------------- main loop -----------------------------------------------

ANIMATIONS = (rain, ripple, spinner, fireworks, sphere_pulse, corkscrew, snake)

while True:
    for animation in ANIMATIONS:
        print("now playing:", animation.__name__)
        leds.clear()
        try:
            animation()
        except Skip:
            print("skipped")
