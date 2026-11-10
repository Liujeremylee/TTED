#CubeLibDemo
#for use with the BCIT TTED 5x5x5 LED cube board and CubeLib.py driver module
#BCIT TTED 4060, October 2026, J.Brett

#******************************* This section MUST be included for initialization ******************************
from machine import Pin, Timer
from time import sleep_ms
import random
import CubeLib		#The CubeLib.py module must be installed in the root directory of your Pico

button1 = Pin(13, Pin.IN, Pin.PULL_UP)
button2 = Pin(10, Pin.IN, Pin.PULL_UP)
button3 = Pin(11, Pin.IN, Pin.PULL_UP)
button4 = Pin(12, Pin.IN, Pin.PULL_UP)

sleep_ms(1000)		#If you save this code as "main.py" it will run immediately on startup. This is great unless it crashes.
                    #this sleep statement gives you a chance to stop the code before it gets stuck in an endless loop.
                    #not that this is likely to happen anymore, of course. I'm pretty sure I fixed that problem. I think.

leds=CubeLib.Cube(5)		#creates a 5x5x5 LED Cube object
CubeLib.leds = leds			#passes the object back to the library so that the library also has access to it
print (leds.data)
# for original single sided pcb leds.xy_map=(27,25,24,19,16, 28,30,20,22,23, 29,31,21,2,4, 13,12,3,1,0, 15,14,5,6,7)
# for original single sided pcb leds.layer_map=(17,8,11,10,9)
leds.xy_map=(27,25,24,19,16, 28,30,20,22,23, 29,31,21,17,4, 13,26,3,1,0, 15,14,5,6,7) #for two-sided board
leds.layer_map=(9,10,11,8,12)	#depending on how you've wired your layers this might be 12,8,11,10,9

leds.start()		#starts the PIO/DMA hardware refresh (~10,000 full brightness cycles per second, almost no CPU)
print(leds.info())

#************************ The section above MUST be included for initialization... now you're good to code *****************
def animations():

    for x in range (5):
        for y in range (5):
            for z in range (5):
                leds.dim()
                leds.set(x,y,z,6)
                sleep_ms(60)
                  
    leds.clear()
    
    for y in range (5):
        for z in range (5):
            for x in range (5):
                leds.dim()
                leds.set(x,y,z,6)
                sleep_ms(30)
                
    leds.clear()
    
    for z in range (5):
        for x in range (5):
            for y in range (5):
                leds.dim()
                leds.set(x,y,z,6)
                sleep_ms(20)
                
    leds.clear()
    
    for i in range (8):
        for z in range (5):
            for x in range (5):
                for y in range (5):
                    leds.set(x,y,z,6)
            sleep_ms(40)
            leds.dim(2)
        leds.clear()
            
        for z in range (4,-1,-1):
            for x in range (5):
                for y in range (5):
                    leds.set(x,y,z,6)
            sleep_ms(40)
            leds.dim(2)
        leds.clear()
            
    for i in range (8):
        for x in range (5):
            for y in range (5):
                for z in range (5):
                    leds.set(x,y,z,6)
            sleep_ms(50)
            leds.dim(3)
        leds.clear()
            
        for x in range (4,-1,-1):
            for y in range (5):
                for z in range (5):
                    leds.set(x,y,z,6)
            sleep_ms(50)
            leds.dim(3)
        leds.clear()
            
    for i in range (8):
        for y in range (5):
            for z in range (5):
                for x in range (5):
                    leds.set(x,y,z,6)
            sleep_ms(40)
            leds.dim(3)
        leds.clear()
            
        for y in range (4,-1,-1):
            for z in range (5):
                for x in range (5):
                    leds.set(x,y,z,6)
            sleep_ms(40)
            leds.dim(3)
        leds.clear()
            
    for i in range (12):
        leds.clear()
        bright=int(i/2)
        for x in range (5):
            for y in range (5):
                leds.set(x,y,0,bright)
                leds.set(x,y,4,bright)
        for x in range (5):
            for z in range (5):
                leds.set(x,0,z,bright)
                leds.set(x,4,z,bright)
        for z in range (5):
            for y in range (5):
                leds.set(0,y,z,bright)
                leds.set(4,y,z,bright)
        sleep_ms(100)
        leds.clear()
        for x in range (1,4):
            for y in range (1,4):
                leds.set(x,y,1,bright)
                leds.set(x,y,3,bright)
        for x in range (5):
            for z in range (1,4):
                leds.set(x,1,z,bright)
                leds.set(x,3,z,bright)
        for z in range (1,4):
            for y in range (1,4):
                leds.set(1,y,z,bright)
                leds.set(3,y,z,bright)
        sleep_ms(100)
        leds.clear()
        leds.set(2,2,2,6)
        sleep_ms(300)
        leds.clear()
        for x in range (1,4):
            for y in range (1,4):
                leds.set(x,y,1,bright)
                leds.set(x,y,3,bright)
        for x in range (5):
            for z in range (1,4):
                leds.set(x,1,z,bright)
                leds.set(x,3,z,bright)
        for z in range (1,4):
            for y in range (1,4):
                leds.set(1,y,z,bright)
                leds.set(3,y,z,bright)
        sleep_ms(300)
        leds.clear()
        
        for brightness in range (1,10):
            leds.fill(brightness)
            #print (brightness)
            sleep_ms(20)
        for brightness in range (10,1,-1):
            leds.fill(brightness)
            #print (brightness)
            sleep_ms(20)
                
while (True):
    animations()

    




    