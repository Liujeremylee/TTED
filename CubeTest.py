#Cube Test for the BCIT TTED 5x5x5 LED Cube
#October 2026, J.Brett
#Must be used with "CubeLib.py"

from machine import Pin, Timer
from time import sleep_ms
import random
import CubeLib

button1 = Pin(13, Pin.IN, Pin.PULL_UP)
button2 = Pin(10, Pin.IN, Pin.PULL_UP)
button3 = Pin(11, Pin.IN, Pin.PULL_UP)
button4 = Pin(12, Pin.IN, Pin.PULL_UP)

sleep_ms(1000)

leds=CubeLib.Cube(5)		#creates a 5x5x5 LED Cube object
CubeLib.leds = leds			#passes the object back to the library so that the library also has access to it
leds.xy_map=(27,25,24,19,16, 28,30,20,22,23, 29,31,21,17,4, 13,26,3,1,0, 15,14,5,6,7)
leds.layer_map=(9,10,11,8,12)

def advance_one_layer(delay):
    print("")
    print("Press the button.")
    while (button1.value()==1):
        sleep_ms(delay)
    CubeLib.refresh_layers(0)
    while (button1.value()==0):
        sleep_ms(delay)


print("Look at your cube from the front. That's the side with the buttons on it.")
leds.fill(10)	#this sets all of the LEDs in the matrix to maximum brightness, but does not turn them on... the CubeLib.refresh_layers will do that, one layer at a time.
print("We will use the leftmost button to advance through our tests.")
advance_one_layer(5)
print("The rightmost layer pad (where the layer power wires go in) should be HIGH, and your TOP LED layer should be ON.")
print("All other layer pads should be LOW")
print("If your bottom layer is turned on, don't worry... you can just the leds.layer_map numbers, above, to match your cube setup.")
advance_one_layer(5)
print("The second from the right layer pad should now be HIGH and the second layer of LEDS should be ON.")
advance_one_layer(5)
print("The middle layer pad should now be HIGH. All other LED pads should be LOW.")
advance_one_layer(5)
print("The second pad from the left should now be high and your second layer from the bottom should be ON.")
advance_one_layer(5)
print("The leftmost layer pad should now be high and your bottom layer should be turned on.")
advance_one_layer(5)
print("We're back to the start: the rightmost layer pad is HIGH and the TOP layer is ON again.")
advance_one_layer(5)
print("The LED cube works by rotating through these layers rapidly. Try pushing the button really, really fast.")
      
for i in range(20):
    advance_one_layer(1)

print("Sigh... you humans are SO slow. Let me show you how that's done.")

for t in range(200,10,-10):
    for i in range(5):
        sleep_ms(t)
        CubeLib.refresh_layers(0)
for t in range(10,1,-1):
    for i in range(5):
        sleep_ms(t)
        CubeLib.refresh_layers(0)
        
print("Now I'll let the Pico's PIO and DMA hardware take over, so that the layers advance automatically with no CPU work at all.")
leds.start()		#hardware refresh: PIO shifts the data into the 595s, DMA feeds it, the CPU is free
print(leds.info())
print("Now the cube is refreshing about 100,000 layers every second... so it looks like all the LEDs are on all the time.")
print("Did you notice that the LEDs look slightly less bright than they did when we were keeping each layer on?")
print("That's because each LED is now only on for 1/5 of the time. But we can dim this even more using PWM.")
advance_one_layer(5)

for loop in range (8):
    for bright in range (10,1,-1):
        leds.fill(bright)
        sleep_ms(80)
    for bright in range (1,10,1):
        leds.fill(bright)
        sleep_ms(80)
        
print("I hope all your LEDs are on right now... but they don't have to be. We can turn some of them off.")
print("To do this the LED cube driver software will set the column pin high, when the layer pin for that LED is high.")
print("With a high signal on the column, there will be no way for the high signal on the LED to pass through that LED.")
print("Press the button to turn your LEDs off.")
advance_one_layer(5)
leds.clear()
print("Now all of the pads connected to the vertical columns should be high. The layers are still going high and low, but the LEDs are all off!")
print("Press the button to turn on just a few LEDs")
advance_one_layer(5)
leds.set(0,0,0,10)
leds.set(1,1,1,8)
leds.set(2,2,2,6)
leds.set(3,3,3,4)
leds.set(4,4,4,2)

print("Now there should be a diagonal line of LEDs of gradually reducing brightness.")
print("Looks like your cube is working well!")